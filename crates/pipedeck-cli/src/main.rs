//! `pipedeck` — the PipeDeck CLI (SPEC §2.3).
//!
//! A thin D-Bus client: it never touches PipeWire directly, so anything it can
//! do the GNOME Shell extension can do too.

mod proxy;

use std::collections::BTreeMap;

use anyhow::{bail, Context as _, Result};
use clap::{Args, Parser, Subcommand};
use futures_util::StreamExt as _;

use pipedeckd::eq;
use pipedeckd::route::PortTuple;
use pipedeckd::state::{
    AutoMuteTuple, DeviceKind, DeviceTuple, EqPresetTuple, EqTuple, StreamTuple,
};
use pipedeckd::volume::{linear_to_percent, percent_to_linear, MAX_VOLUME};

use proxy::DaemonProxy;

/// Control PipeWire audio through the PipeDeck daemon.
#[derive(Debug, Parser)]
#[command(name = "pipedeck", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Show defaults, devices and playback streams.
    Status,
    /// List output devices (sinks).
    Outputs,
    /// List input devices (sources).
    Inputs,
    /// List playback streams.
    Streams,
    /// List the selectable ports of every device.
    Ports,
    /// Make a sink the default output.
    SetOutput(NameArg),
    /// Make a source the default input.
    SetInput(NameArg),
    /// Send notification sounds to a sink; `none` follows the default output.
    SetNotify(NameArg),
    /// Set the volume of a device or stream, as a percentage (0-150).
    Vol {
        /// Node id, from `pipedeck status`.
        id: u32,
        /// Percentage; 100 is unity gain.
        percent: f64,
    },
    /// Mute, unmute, or toggle a device or stream.
    Mute {
        /// Node id, from `pipedeck status`.
        id: u32,
        /// `on`, `off`, or omitted to toggle.
        state: Option<String>,
    },
    /// Switch a device to one of its ports.
    SetPort {
        /// Node id, from `pipedeck outputs`.
        id: u32,
        /// Route name (`analog-output-lineout`), description, or index.
        port: String,
    },
    /// EQ presets (SPEC §7.3).
    #[command(subcommand)]
    Eq(EqCmd),
    /// Show or set ALSA "Auto-Mute Mode" (SPEC §8.1).
    ///
    /// Realtek codecs hard-mute the line-out whenever a headphone plug is
    /// present, whatever port is selected. Turning it off lets the speaker
    /// port play with headphones still plugged in.
    #[command(name = "automute")]
    AutoMute {
        /// Node id, from `pipedeck outputs`. Omit to list every card.
        id: Option<u32>,
        /// `on` or `off`. Required when an id is given.
        state: Option<String>,
    },
    /// Show or set the port-switch level cap (SPEC §9.2).
    ///
    /// WirePlumber restores volume per port, so switching from a quiet
    /// headphone port to a loud speaker port jumps straight to the loud
    /// level. The cap clamps whatever a PipeDeck-initiated switch restores.
    Cap {
        /// `0`-`150` (cubic percent, like `pipedeck vol`) or `off`. Omit to show.
        value: Option<String>,
    },
    /// Ask the daemon to re-read the graph.
    Refresh,
    /// Print a line every time the daemon signals a change.
    Watch,
}

#[derive(Debug, Args)]
struct NameArg {
    /// `node.name` of the device (see `pipedeck outputs`).
    name: String,
}

#[derive(Debug, Subcommand)]
enum EqCmd {
    /// List the available presets.
    List,
    /// Show one preset's bands.
    Show {
        /// Preset id, from `pipedeck eq list`.
        id: String,
    },
    /// Apply a preset to an output device; `off` turns EQ off.
    Set {
        /// Node id, from `pipedeck outputs`.
        id: u32,
        /// Preset id, or `off`/`none`/`-`.
        preset: String,
    },
    /// Import an AutoEq `ParametricEQ.txt` as a preset.
    Import {
        /// Path to the AutoEq file.
        file: std::path::PathBuf,
        /// Display name; the id is derived from it. Defaults to the file stem.
        #[arg(long)]
        name: Option<String>,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let connection = zbus::Connection::session()
        .await
        .context("could not connect to the session bus")?;
    let daemon = DaemonProxy::new(&connection)
        .await
        .context("could not reach the PipeDeck daemon (try: systemctl --user start pipedeckd)")?;

    match cli.command {
        Cmd::Status => status(&daemon).await,
        Cmd::Outputs => list_devices(&daemon, DeviceKind::Sink).await,
        Cmd::Inputs => list_devices(&daemon, DeviceKind::Source).await,
        Cmd::Streams => list_streams(&daemon).await,
        Cmd::Ports => list_ports(&daemon).await,
        Cmd::SetOutput(arg) => {
            daemon.set_default("sink", &arg.name).await?;
            println!("default output -> {}", arg.name);
            Ok(())
        }
        Cmd::SetInput(arg) => {
            daemon.set_default("source", &arg.name).await?;
            println!("default input -> {}", arg.name);
            Ok(())
        }
        Cmd::SetNotify(arg) => {
            let name = if arg.name.eq_ignore_ascii_case("none") || arg.name == "-" {
                String::new()
            } else {
                arg.name.clone()
            };
            daemon.set_notification_sink(&name).await?;
            if name.is_empty() {
                println!("notifications -> default output");
            } else {
                println!("notifications -> {name}");
            }
            Ok(())
        }
        Cmd::Vol { id, percent } => {
            let max_percent = linear_to_percent(MAX_VOLUME);
            if !(0.0..=max_percent).contains(&percent) {
                bail!("volume must be between 0 and {max_percent:.0}");
            }
            let volume = percent_to_linear(percent);
            daemon.set_volume(id, volume).await?;
            println!("{id} volume -> {:.0}%", linear_to_percent(volume));
            Ok(())
        }
        Cmd::Mute { id, state } => {
            let mute = match state.as_deref() {
                Some("on" | "true" | "yes" | "1") => true,
                Some("off" | "false" | "no" | "0") => false,
                None | Some("toggle") => !current_mute(&daemon, id).await?,
                Some(other) => bail!("expected `on`, `off` or `toggle`, got `{other}`"),
            };
            daemon.set_mute(id, mute).await?;
            println!("{id} mute -> {}", if mute { "on" } else { "off" });
            Ok(())
        }
        Cmd::SetPort { id, port } => {
            let ports = daemon.ports().await?;
            let index = resolve_port(&ports, id, &port)?;
            daemon.set_port(id, index).await?;
            let label = ports
                .iter()
                .find(|p| p.0 == id && p.1 == index)
                .map_or_else(|| index.to_string(), |p| p.3.clone());
            println!("{id} port -> {label}");
            Ok(())
        }
        Cmd::Eq(cmd) => eq_command(&daemon, cmd).await,
        Cmd::AutoMute { id, state } => auto_mute_command(&daemon, id, state).await,
        Cmd::Cap { value } => cap_command(&daemon, value).await,
        Cmd::Refresh => {
            daemon.refresh().await?;
            println!("refreshed");
            Ok(())
        }
        Cmd::Watch => watch(&daemon).await,
    }
}

async fn current_mute(daemon: &DaemonProxy<'_>, id: u32) -> Result<bool> {
    if let Some(device) = daemon.devices().await?.into_iter().find(|d| d.0 == id) {
        return Ok(device.7);
    }
    if let Some(stream) = daemon.streams().await?.into_iter().find(|s| s.0 == id) {
        return Ok(stream.6);
    }
    bail!("no device or stream with id {id}")
}

/// One device row: nick first (ALSA descriptions truncate), then the active
/// port in brackets when the node has more than one (SPEC §6.1), the EQ preset
/// (SPEC §7.3) and `[auto-mute]` (SPEC §8.1), then the full description and
/// `node.name` on continuation lines.
fn device_line(
    device: &DeviceTuple,
    ports: &[PortTuple],
    eq_label: Option<&str>,
    auto_mute: bool,
) -> String {
    let (id, name, description, _kind, is_default, virtual_, volume, mute, nick) = device;
    let marker = if *is_default { '*' } else { ' ' };
    let label = if nick.is_empty() { description } else { nick };

    let mine: Vec<&PortTuple> = ports.iter().filter(|p| p.0 == *id).collect();
    let port = if mine.len() > 1 {
        mine.iter()
            .find(|p| p.5)
            .map(|p| format!(" [{}]", p.3))
            .unwrap_or_default()
    } else {
        String::new()
    };
    // SPEC §7.3: `{eq: <name>}` after the port bracket, only when EQ is on.
    let eq = eq_label.map_or_else(String::new, |name| format!(" {{eq: {name}}}"));
    // SPEC §8.1: `[auto-mute]` while the card's Auto-Mute Mode is enabled.
    let auto_mute = if auto_mute { " [auto-mute]" } else { "" };

    let mut flags = String::new();
    if *mute {
        flags.push_str(" [muted]");
    }
    if *virtual_ {
        flags.push_str(" [virtual]");
    }

    let mut line = format!(
        "{marker} {id:>5}  {:>4.0}%  {label}{port}{eq}{auto_mute}{flags}",
        linear_to_percent(*volume)
    );
    if description != label && !description.is_empty() {
        line.push_str(&format!("\n            {description}"));
    }
    line.push_str(&format!("\n            {name}"));
    line
}

/// One port row for `pipedeck ports`.
fn port_line(port: &PortTuple) -> String {
    let (node_id, index, name, description, available, active) = port;
    let marker = if *active { '*' } else { ' ' };
    let flag = if *available { "" } else { "  [unavailable]" };
    format!("{marker} {node_id:>5}  {index:>3}  {description}{flag}\n            {name}")
}

/// Resolve `pipedeck set-port <id> <name|description|index>` to a route index.
///
/// Index first (so a numeric argument that *is* a route index always wins),
/// then the route name, then its description — both case-insensitively.
fn resolve_port(ports: &[PortTuple], id: u32, wanted: &str) -> Result<u32> {
    let mine: Vec<&PortTuple> = ports.iter().filter(|p| p.0 == id).collect();
    if mine.is_empty() {
        bail!("node {id} has no ports (see `pipedeck ports`)");
    }
    if let Ok(index) = wanted.parse::<u32>() {
        if mine.iter().any(|p| p.1 == index) {
            return Ok(index);
        }
    }
    if let Some(port) = mine.iter().find(|p| p.2.eq_ignore_ascii_case(wanted)) {
        return Ok(port.1);
    }
    if let Some(port) = mine.iter().find(|p| p.3.eq_ignore_ascii_case(wanted)) {
        return Ok(port.1);
    }
    let known: Vec<String> = mine.iter().map(|p| p.2.clone()).collect();
    bail!(
        "node {id} has no port `{wanted}`; it has: {}",
        known.join(", ")
    )
}

/// Node id -> preset *display name*, for the `{eq: ...}` bracket.
///
/// An id the daemon reports but the preset list does not know is shown as the
/// raw id rather than dropped — that combination means someone edited the
/// config by hand, and hiding it would be confusing.
fn eq_labels(eq: &[EqTuple], presets: &[EqPresetTuple]) -> BTreeMap<u32, String> {
    eq.iter()
        .filter(|(_, preset)| !preset.is_empty())
        .map(|(id, preset)| {
            let name = presets
                .iter()
                .find(|(pid, _)| pid == preset)
                .map_or_else(|| preset.clone(), |(_, name)| name.clone());
            (*id, name)
        })
        .collect()
}

fn stream_line(stream: &StreamTuple) -> String {
    let (id, app_name, binary, media_name, target_name, volume, mute) = stream;
    let label = if app_name.is_empty() {
        binary
    } else {
        app_name
    };
    let muted = if *mute { " [muted]" } else { "" };
    let target = if target_name.is_empty() {
        String::new()
    } else {
        format!("  -> {target_name}")
    };
    format!(
        "  {id:>5}  {:>4.0}%  {label}{muted}{target}\n            {media_name}",
        linear_to_percent(*volume)
    )
}

async fn status(daemon: &DaemonProxy<'_>) -> Result<()> {
    let devices = daemon.devices().await?;
    let ports = daemon.ports().await.unwrap_or_default();
    let eq = eq_labels(
        &daemon.eq().await.unwrap_or_default(),
        &daemon.eq_presets().await.unwrap_or_default(),
    );
    let auto_mute = daemon.auto_mute().await.unwrap_or_default();
    let streams = daemon.streams().await?;
    let notify = daemon.notification_sink().await?;
    let version = daemon.version().await.unwrap_or_default();

    println!("pipedeckd {version}");
    println!(
        "notifications: {}",
        if notify.is_empty() {
            "<default output>".to_owned()
        } else {
            notify
        }
    );

    for (kind, title) in [
        (DeviceKind::Sink, "Outputs"),
        (DeviceKind::Source, "Inputs"),
    ] {
        println!("\n{title}:");
        let mut any = false;
        for device in devices.iter().filter(|d| d.3 == kind.as_str()) {
            println!(
                "{}",
                device_line(
                    device,
                    &ports,
                    eq.get(&device.0).map(String::as_str),
                    auto_mute_on(&auto_mute, device.0)
                )
            );
            any = true;
        }
        if !any {
            println!("  (none)");
        }
    }

    println!("\nStreams:");
    if streams.is_empty() {
        println!("  (none)");
    }
    for stream in &streams {
        println!("{}", stream_line(stream));
    }
    Ok(())
}

async fn list_devices(daemon: &DaemonProxy<'_>, kind: DeviceKind) -> Result<()> {
    let devices = daemon.devices().await?;
    let ports = daemon.ports().await.unwrap_or_default();
    let eq = eq_labels(
        &daemon.eq().await.unwrap_or_default(),
        &daemon.eq_presets().await.unwrap_or_default(),
    );
    let auto_mute = daemon.auto_mute().await.unwrap_or_default();
    let mut any = false;
    for device in devices.iter().filter(|d| d.3 == kind.as_str()) {
        println!(
            "{}",
            device_line(
                device,
                &ports,
                eq.get(&device.0).map(String::as_str),
                auto_mute_on(&auto_mute, device.0)
            )
        );
        any = true;
    }
    if !any {
        println!("(no {kind}s)");
    }
    Ok(())
}

async fn list_ports(daemon: &DaemonProxy<'_>) -> Result<()> {
    let ports = daemon.ports().await?;
    if ports.is_empty() {
        println!("(no ports; only cards with ALSA routes have them)");
    }
    for port in &ports {
        println!("{}", port_line(port));
    }
    Ok(())
}

async fn list_streams(daemon: &DaemonProxy<'_>) -> Result<()> {
    let streams = daemon.streams().await?;
    if streams.is_empty() {
        println!("(no streams)");
    }
    for stream in &streams {
        println!("{}", stream_line(stream));
    }
    Ok(())
}

/// `pipedeck eq …` (SPEC §7.3).
///
/// `list` and `set` go through the daemon; `show` and `import` are local file
/// work, since the presets directory is the daemon's source of truth and the
/// daemon exposes only `(id, name)` on the bus.
async fn eq_command(daemon: &DaemonProxy<'_>, cmd: EqCmd) -> Result<()> {
    match cmd {
        EqCmd::List => {
            let presets = daemon.eq_presets().await?;
            if presets.is_empty() {
                let dir = eq::presets_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|_| "<no config dir>".to_owned());
                println!("(no EQ presets; drop preset files in {dir})");
            }
            for (id, name) in &presets {
                println!("  {id:<24}  {name}");
            }
            Ok(())
        }
        EqCmd::Show { id } => {
            let dir = eq::presets_dir().map_err(|e| anyhow::anyhow!("{e}"))?;
            let path = dir.join(format!("{id}.toml"));
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let preset = eq::parse_preset(&id, &text).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("{}  ({})", preset.name, preset.id);
            println!("preamp: {:+.1} dB", preset.preamp_db);
            if preset.bands.is_empty() {
                println!("  (no bands — flat)");
            }
            for band in &preset.bands {
                println!(
                    "  {:<10}  {:>8.1} Hz  Q {:<5.2}  {:+.1} dB",
                    band.kind.as_str(),
                    band.freq,
                    band.q,
                    band.gain_db
                );
            }
            Ok(())
        }
        EqCmd::Set { id, preset } => {
            let preset = if is_off(&preset) {
                String::new()
            } else {
                preset
            };
            daemon.set_eq(id, &preset).await?;
            if preset.is_empty() {
                println!("{id} eq -> off");
            } else {
                println!("{id} eq -> {preset}");
            }
            Ok(())
        }
        EqCmd::Import { file, name } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("could not read {}", file.display()))?;
            let import = eq::parse_autoeq(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
            let label = name.unwrap_or_else(|| import_label(&file));
            let id = eq::slugify(&label);
            if id.is_empty() {
                bail!("`{label}` has no usable preset id; pass --name");
            }
            let preset =
                eq::autoeq_to_preset(&id, &label, &import).map_err(|e| anyhow::anyhow!("{e}"))?;
            let dir = eq::presets_dir().map_err(|e| anyhow::anyhow!("{e}"))?;
            let path = eq::write_preset(&dir, &preset)
                .with_context(|| format!("could not write into {}", dir.display()))?;
            for warning in &import.warnings {
                eprintln!("warning: {warning}");
            }
            println!(
                "{} -> {} ({} band(s), preamp {:+.1} dB)",
                path.display(),
                preset.id,
                preset.bands.len(),
                preset.preamp_db
            );
            println!("{}", preset.id);
            // The daemon rescans on SetEq/Refresh; nudge it so `eq list` is
            // immediately current.
            let _ = daemon.refresh().await;
            Ok(())
        }
    }
}

/// Is a node's card reporting `Auto-Mute Mode` as enabled? (SPEC §8.1.)
///
/// A node with no row — a virtual sink, an HDMI card without the control, or
/// an older daemon that has no `AutoMute` property at all — reads as "off",
/// which is exactly what "no `[auto-mute]` tag" should mean.
fn auto_mute_on(rows: &[AutoMuteTuple], node_id: u32) -> bool {
    rows.iter()
        .find(|(id, _)| *id == node_id)
        .is_some_and(|(_, enabled)| *enabled)
}

/// Parse the `on`/`off` argument of `pipedeck automute <id> <state>`.
fn parse_on_off(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "enabled" | "enable" => Some(true),
        "off" | "false" | "no" | "0" | "disabled" | "disable" => Some(false),
        _ => None,
    }
}

/// `pipedeck automute [<id> on|off]` (SPEC §8.1).
async fn auto_mute_command(
    daemon: &DaemonProxy<'_>,
    id: Option<u32>,
    state: Option<String>,
) -> Result<()> {
    let Some(id) = id else {
        let rows = daemon.auto_mute().await?;
        if rows.is_empty() {
            println!("(no cards with an `Auto-Mute Mode` control)");
            return Ok(());
        }
        let devices = daemon.devices().await.unwrap_or_default();
        for (node_id, enabled) in &rows {
            let label = devices
                .iter()
                .find(|d| d.0 == *node_id)
                .map_or_else(String::new, |d| {
                    if d.8.is_empty() {
                        d.2.clone()
                    } else {
                        d.8.clone()
                    }
                });
            println!(
                "  {node_id:>5}  {:<3}  {label}",
                if *enabled { "on" } else { "off" }
            );
        }
        return Ok(());
    };

    let Some(state) = state else {
        bail!("expected `on` or `off` after the node id (see `pipedeck automute`)");
    };
    let enabled = parse_on_off(&state)
        .ok_or_else(|| anyhow::anyhow!("expected `on` or `off`, got `{state}`"))?;
    daemon.set_auto_mute(id, enabled).await?;
    println!("{id} auto-mute -> {}", if enabled { "on" } else { "off" });
    Ok(())
}

/// Highest cap the daemon will take, on the cubic scale (SPEC §9.2).
const MAX_CAP_PERCENT: u32 = 150;

/// Parse the argument of `pipedeck cap <0-150|off>` into a percentage.
///
/// `off`/`none`/`-` and a bare `0` all mean the same thing to the daemon: the
/// rule is disabled. A trailing `%` is accepted because the output prints one.
fn parse_cap(value: &str) -> Result<u32> {
    let value = value.trim();
    if is_off(value) {
        return Ok(0);
    }
    let digits = value.strip_suffix('%').unwrap_or(value).trim();
    let percent: u32 = digits.parse().map_err(|_| {
        anyhow::anyhow!("expected a percentage 0-{MAX_CAP_PERCENT} or `off`, got `{value}`")
    })?;
    if percent > MAX_CAP_PERCENT {
        bail!("the cap must be between 0 and {MAX_CAP_PERCENT} percent (0 turns it off)");
    }
    Ok(percent)
}

/// How `pipedeck cap` renders the daemon's current value.
fn cap_label(percent: u32) -> String {
    if percent == 0 {
        "off".to_owned()
    } else {
        format!("{percent}% (cubic)")
    }
}

/// `pipedeck cap [<0-150|off>]` (SPEC §9.2).
async fn cap_command(daemon: &DaemonProxy<'_>, value: Option<String>) -> Result<()> {
    let Some(value) = value else {
        println!("{}", cap_label(daemon.port_switch_cap().await?));
        return Ok(());
    };
    let percent = parse_cap(&value)?;
    daemon.set_port_switch_cap(percent).await?;
    println!("port-switch cap -> {}", cap_label(percent));
    Ok(())
}

/// Is this argument one of the spellings that mean "no preset"?
fn is_off(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || value == "-"
        || value.eq_ignore_ascii_case("off")
        || value.eq_ignore_ascii_case("none")
}

/// Default display name for an import: the file stem, minus AutoEq's usual
/// ` ParametricEQ` suffix.
fn import_label(path: &std::path::Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("preset")
        .trim();
    stem.strip_suffix(" ParametricEQ")
        .or_else(|| stem.strip_suffix("ParametricEQ"))
        .map_or(stem, str::trim)
        .to_owned()
}

async fn watch(daemon: &DaemonProxy<'_>) -> Result<()> {
    let mut changes = daemon.receive_changed().await?;
    println!("watching {} (ctrl-c to stop)", proxy_label(daemon));
    while changes.next().await.is_some() {
        let devices = daemon.devices().await.unwrap_or_default().len();
        let streams = daemon.streams().await.unwrap_or_default().len();
        println!("changed: {devices} devices, {streams} streams");
    }
    Ok(())
}

fn proxy_label(daemon: &DaemonProxy<'_>) -> String {
    format!("{}{}", daemon.inner().destination(), daemon.inner().path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn subcommands_cover_the_spec() {
        let cmd = Cli::command();
        let names: Vec<_> = cmd.get_subcommands().map(clap::Command::get_name).collect();
        for expected in [
            "status",
            "outputs",
            "set-output",
            "set-input",
            "set-notify",
            "ports",
            "set-port",
            "vol",
            "mute",
            "eq",
            "automute",
            "cap",
            "watch",
        ] {
            assert!(names.contains(&expected), "missing subcommand {expected}");
        }
    }

    #[test]
    fn mute_state_is_optional() {
        let cli = Cli::try_parse_from(["pipedeck", "mute", "42"]).expect("parses");
        assert!(matches!(
            cli.command,
            Cmd::Mute {
                id: 42,
                state: None
            }
        ));
        let cli = Cli::try_parse_from(["pipedeck", "mute", "42", "on"]).expect("parses");
        assert!(matches!(cli.command, Cmd::Mute { id: 42, state: Some(ref s) } if s == "on"));
    }

    #[test]
    fn vol_takes_a_percentage() {
        let cli = Cli::try_parse_from(["pipedeck", "vol", "7", "62.5"]).expect("parses");
        match cli.command {
            Cmd::Vol { id, percent } => {
                assert_eq!(id, 7);
                assert!((percent - 62.5).abs() < f64::EPSILON);
                assert!((percent_to_linear(percent) - 0.244_140_625).abs() < 1e-9);
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    fn device(id: u32, name: &str, description: &str, nick: &str) -> DeviceTuple {
        (
            id,
            name.to_owned(),
            description.to_owned(),
            "sink".to_owned(),
            false,
            false,
            1.0,
            false,
            nick.to_owned(),
        )
    }

    /// The chronos card: two ports on node 39, one on node 43.
    fn ports() -> Vec<PortTuple> {
        vec![
            (
                39,
                3,
                "analog-output-lineout".to_owned(),
                "Line Out".to_owned(),
                true,
                false,
            ),
            (
                39,
                4,
                "analog-output-headphones".to_owned(),
                "Headphones".to_owned(),
                true,
                true,
            ),
            (
                41,
                1,
                "analog-input-linein".to_owned(),
                "Line In".to_owned(),
                false,
                false,
            ),
            (
                43,
                2,
                "hdmi-output-0".to_owned(),
                "HDMI / DisplayPort".to_owned(),
                true,
                true,
            ),
        ]
    }

    #[test]
    fn device_line_marks_the_default_and_flags() {
        let mut d = device(11, "sink-a", "Analog", "ALC892 Analog");
        d.4 = true;
        d.5 = true;
        d.6 = 0.125; // 50 % on the cubic scale wpctl shows
        d.7 = true;
        let line = device_line(&d, &[], None, false);
        assert!(line.starts_with('*'));
        assert!(line.contains("50%"));
        assert!(line.contains("[muted]"));
        assert!(line.contains("[virtual]"));
        assert!(line.contains("sink-a"));

        let plain = device_line(
            &device(12, "sink-b", "HDMI", "Dell AW3423DW"),
            &[],
            None,
            false,
        );
        assert!(plain.starts_with(' '));
        assert!(!plain.contains("[muted]"));
    }

    #[test]
    fn device_line_leads_with_the_nick_and_keeps_the_description() {
        let line = device_line(
            &device(
                39,
                "alsa_output.pci-0000_28_00.4.analog-stereo",
                "Starship/Matisse HD Audio Controller Analog Stereo",
                "ALC892 Analog",
            ),
            &[],
            None,
            false,
        );
        let first = line.lines().next().expect("a first line");
        assert!(first.contains("ALC892 Analog"));
        assert!(!first.contains("Starship"));
        assert!(line.contains("Starship/Matisse HD Audio Controller Analog Stereo"));
        assert!(line.contains("alsa_output.pci-0000_28_00.4.analog-stereo"));

        // No nick: fall back to the description, and do not print it twice.
        let bare = device_line(&device(12, "sink-b", "HDMI", ""), &[], None, false);
        assert_eq!(bare.matches("HDMI").count(), 1);
    }

    #[test]
    fn device_line_shows_the_active_port_only_when_there_is_a_choice() {
        let multi = device_line(
            &device(39, "sink-a", "Analog", "ALC892 Analog"),
            &ports(),
            None,
            false,
        );
        assert!(multi.contains("[Headphones]"));

        // One port is not a choice: no bracket.
        let single = device_line(
            &device(43, "sink-hdmi", "HDMI", "Dell AW3423DW"),
            &ports(),
            None,
            false,
        );
        assert!(!single.contains("[HDMI / DisplayPort]"));

        // No ports at all (virtual sink).
        let none = device_line(&device(60, "null", "Null", "Null"), &ports(), None, false);
        assert!(!none.contains('['));
    }

    #[test]
    fn port_line_marks_the_active_and_dims_the_unavailable() {
        let all = ports();
        let active = port_line(&all[1]);
        assert!(active.starts_with('*'));
        assert!(active.contains("Headphones"));
        assert!(active.contains("analog-output-headphones"));
        assert!(!active.contains("[unavailable]"));

        let inactive = port_line(&all[0]);
        assert!(inactive.starts_with(' '));

        let unavailable = port_line(&all[2]);
        assert!(unavailable.contains("[unavailable]"));
    }

    #[test]
    fn resolve_port_takes_an_index_a_name_or_a_description() {
        let all = ports();
        assert_eq!(resolve_port(&all, 39, "3").expect("index"), 3);
        assert_eq!(
            resolve_port(&all, 39, "analog-output-lineout").expect("name"),
            3
        );
        assert_eq!(
            resolve_port(&all, 39, "ANALOG-OUTPUT-HEADPHONES").expect("name, any case"),
            4
        );
        assert_eq!(resolve_port(&all, 39, "Line Out").expect("description"), 3);
        // An unavailable port still resolves; the daemon is what rejects it.
        assert_eq!(
            resolve_port(&all, 41, "analog-input-linein").expect("name"),
            1
        );
    }

    #[test]
    fn resolve_port_reports_unknown_nodes_and_ports() {
        let all = ports();
        let err = resolve_port(&all, 99, "3").expect_err("no such node");
        assert!(err.to_string().contains("no ports"));
        let err = resolve_port(&all, 39, "nonsense").expect_err("no such port");
        assert!(err.to_string().contains("analog-output-lineout"));
        // An index that belongs to a different node is not accepted either.
        assert!(resolve_port(&all, 39, "2").is_err());
    }

    #[test]
    fn set_port_parses_an_id_and_a_port() {
        let cli = Cli::try_parse_from(["pipedeck", "set-port", "39", "analog-output-lineout"])
            .expect("parses");
        match cli.command {
            Cmd::SetPort { id, port } => {
                assert_eq!(id, 39);
                assert_eq!(port, "analog-output-lineout");
            }
            other => panic!("wrong subcommand: {other:?}"),
        }
    }

    /// SPEC §7.3: `outputs` shows `{eq: <name>}` after the port bracket.
    #[test]
    fn device_line_shows_the_eq_bracket_after_the_port() {
        let line = device_line(
            &device(39, "sink-a", "Analog", "ALC892 Analog"),
            &ports(),
            Some("Sennheiser HD 650"),
            false,
        );
        let first = line.lines().next().expect("a first line");
        assert!(first.contains("[Headphones]"));
        assert!(first.contains("{eq: Sennheiser HD 650}"));
        assert!(
            first.find("[Headphones]") < first.find("{eq:"),
            "eq must come after the port bracket: {first}"
        );

        // Off means no bracket at all.
        let off = device_line(
            &device(39, "sink-a", "Analog", "ALC892 Analog"),
            &ports(),
            None,
            false,
        );
        assert!(!off.contains("{eq:"));
    }

    /// SPEC §8.1: `outputs` appends `[auto-mute]` after the port bracket while
    /// the card's Auto-Mute Mode is enabled.
    #[test]
    fn device_line_shows_the_auto_mute_tag_after_the_port() {
        let line = device_line(
            &device(39, "sink-a", "Analog", "ALC892 Analog"),
            &ports(),
            Some("Sennheiser HD 650"),
            true,
        );
        let first = line.lines().next().expect("a first line");
        assert!(first.contains("[auto-mute]"));
        assert!(
            first.find("[Headphones]") < first.find("[auto-mute]"),
            "the auto-mute tag must come after the port bracket: {first}"
        );
        assert!(
            first.find("{eq:") < first.find("[auto-mute]"),
            "the auto-mute tag must come after the eq bracket: {first}"
        );

        // Disabled means no tag at all.
        let off = device_line(
            &device(39, "sink-a", "Analog", "ALC892 Analog"),
            &ports(),
            None,
            false,
        );
        assert!(!off.contains("[auto-mute]"));
    }

    /// A node with no `AutoMute` row — a virtual sink, an HDMI card without
    /// the control, or an older daemon with no such property — reads as off.
    #[test]
    fn auto_mute_rows_are_looked_up_by_node_id() {
        let rows: Vec<AutoMuteTuple> = vec![(39, true), (43, false)];
        assert!(auto_mute_on(&rows, 39));
        assert!(!auto_mute_on(&rows, 43));
        assert!(!auto_mute_on(&rows, 60));
        assert!(!auto_mute_on(&[], 39));
    }

    #[test]
    fn auto_mute_state_spellings() {
        for value in ["on", "ON", " true ", "yes", "1", "Enabled"] {
            assert_eq!(parse_on_off(value), Some(true), "{value} should mean on");
        }
        for value in ["off", "OFF", "false", "no", "0", "Disabled"] {
            assert_eq!(parse_on_off(value), Some(false), "{value} should mean off");
        }
        for value in ["toggle", "", "maybe"] {
            assert_eq!(parse_on_off(value), None, "{value} should not parse");
        }
    }

    /// `pipedeck automute` lists; `pipedeck automute <id> on|off` sets.
    #[test]
    fn automute_parses_with_and_without_arguments() {
        let cli = Cli::try_parse_from(["pipedeck", "automute"]).expect("parses");
        assert!(matches!(
            cli.command,
            Cmd::AutoMute {
                id: None,
                state: None
            }
        ));

        let cli = Cli::try_parse_from(["pipedeck", "automute", "39", "off"]).expect("parses");
        match cli.command {
            Cmd::AutoMute { id, state } => {
                assert_eq!(id, Some(39));
                assert_eq!(state.as_deref(), Some("off"));
            }
            other => panic!("wrong subcommand: {other:?}"),
        }

        // An id with no state parses; the command itself is what complains.
        let cli = Cli::try_parse_from(["pipedeck", "automute", "39"]).expect("parses");
        assert!(matches!(
            cli.command,
            Cmd::AutoMute {
                id: Some(39),
                state: None
            }
        ));
    }

    #[test]
    fn eq_labels_resolve_names_and_skip_the_off_rows() {
        let presets = vec![
            ("hd650".to_owned(), "Sennheiser HD 650".to_owned()),
            ("flat".to_owned(), "Flat".to_owned()),
        ];
        let eq = vec![
            (39, "hd650".to_owned()),
            (43, String::new()),
            (44, "handedited".to_owned()),
        ];
        let labels = eq_labels(&eq, &presets);
        assert_eq!(
            labels.get(&39).map(String::as_str),
            Some("Sennheiser HD 650")
        );
        assert!(!labels.contains_key(&43));
        // An id with no matching preset still shows, as the raw id.
        assert_eq!(labels.get(&44).map(String::as_str), Some("handedited"));
        assert!(eq_labels(&[], &presets).is_empty());
    }

    #[test]
    fn eq_off_spellings() {
        for value in ["off", "OFF", "none", "-", "", "  "] {
            assert!(is_off(value), "{value} should mean off");
        }
        for value in ["hd650", "flat", "offset"] {
            assert!(!is_off(value), "{value} should not mean off");
        }
    }

    #[test]
    fn import_label_strips_the_autoeq_suffix() {
        use std::path::Path;
        assert_eq!(
            import_label(Path::new("/tmp/Sennheiser HD 650 ParametricEQ.txt")),
            "Sennheiser HD 650"
        );
        assert_eq!(
            import_label(Path::new("/tmp/HD650ParametricEQ.txt")),
            "HD650"
        );
        assert_eq!(import_label(Path::new("/tmp/hd650.txt")), "hd650");
        assert_eq!(
            eq::slugify(&import_label(Path::new("x/HD 650.txt"))),
            "hd-650"
        );
    }

    #[test]
    fn eq_subcommands_parse() {
        let cli = Cli::try_parse_from(["pipedeck", "eq", "set", "39", "hd650"]).expect("parses");
        match cli.command {
            Cmd::Eq(EqCmd::Set { id, ref preset }) => {
                assert_eq!(id, 39);
                assert_eq!(preset, "hd650");
            }
            ref other => panic!("wrong subcommand: {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "pipedeck",
            "eq",
            "import",
            "hd650.txt",
            "--name",
            "Sennheiser HD 650",
        ])
        .expect("parses");
        match cli.command {
            Cmd::Eq(EqCmd::Import { ref file, ref name }) => {
                assert_eq!(file.to_str(), Some("hd650.txt"));
                assert_eq!(name.as_deref(), Some("Sennheiser HD 650"));
            }
            ref other => panic!("wrong subcommand: {other:?}"),
        }

        assert!(Cli::try_parse_from(["pipedeck", "eq", "list"]).is_ok());
        assert!(Cli::try_parse_from(["pipedeck", "eq", "show", "hd650"]).is_ok());
        assert!(Cli::try_parse_from(["pipedeck", "eq"]).is_err());
    }

    #[test]
    fn stream_line_falls_back_to_the_binary_name() {
        let line = stream_line(&(
            7,
            String::new(),
            "firefox".to_owned(),
            "Some Track".to_owned(),
            "sink-a".to_owned(),
            1.0,
            false,
        ));
        assert!(line.contains("firefox"));
        assert!(line.contains("-> sink-a"));
        assert!(line.contains("Some Track"));

        let untargeted = stream_line(&(
            8,
            "Firefox".to_owned(),
            "firefox".to_owned(),
            "Track".to_owned(),
            String::new(),
            1.0,
            true,
        ));
        assert!(untargeted.contains("Firefox"));
        assert!(untargeted.contains("[muted]"));
        assert!(!untargeted.contains("->"));
    }

    /// SPEC §9.2: `pipedeck cap` shows, `pipedeck cap <value>` sets — so the
    /// argument is optional and every "off" spelling reaches the daemon as 0.
    #[test]
    fn cap_takes_an_optional_value() {
        let cli = Cli::try_parse_from(["pipedeck", "cap"]).expect("parses");
        assert!(matches!(cli.command, Cmd::Cap { value: None }));
        let cli = Cli::try_parse_from(["pipedeck", "cap", "off"]).expect("parses");
        assert!(matches!(cli.command, Cmd::Cap { value: Some(ref v) } if v == "off"));

        assert_eq!(parse_cap("60").expect("parses"), 60);
        assert_eq!(parse_cap(" 60% ").expect("parses"), 60);
        assert_eq!(parse_cap("0").expect("parses"), 0);
        for off in ["off", "OFF", "none", "-"] {
            assert_eq!(parse_cap(off).expect("parses"), 0, "spelling {off}");
        }
        assert_eq!(parse_cap("150").expect("parses"), 150);
        assert!(parse_cap("151").is_err());
        assert!(parse_cap("sixty").is_err());
        assert!(parse_cap("-5").is_err());
        assert!(parse_cap("60.5").is_err());
    }

    /// The two lines SPEC §9.2 asks `pipedeck cap` to print.
    #[test]
    fn cap_label_reads_as_the_spec_writes_it() {
        assert_eq!(cap_label(60), "60% (cubic)");
        assert_eq!(cap_label(45), "45% (cubic)");
        assert_eq!(cap_label(0), "off");
    }
}
