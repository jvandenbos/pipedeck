//! `pipedeck` — the PipeDeck CLI (SPEC §2.3).
//!
//! A thin D-Bus client: it never touches PipeWire directly, so anything it can
//! do the GNOME Shell extension can do too.

mod proxy;

use anyhow::{bail, Context as _, Result};
use clap::{Args, Parser, Subcommand};
use futures_util::StreamExt as _;

use pipedeckd::route::PortTuple;
use pipedeckd::state::{DeviceKind, DeviceTuple, StreamTuple};
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
/// port in brackets when the node has more than one (SPEC §6.1), then the full
/// description and `node.name` on continuation lines.
fn device_line(device: &DeviceTuple, ports: &[PortTuple]) -> String {
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

    let mut flags = String::new();
    if *mute {
        flags.push_str(" [muted]");
    }
    if *virtual_ {
        flags.push_str(" [virtual]");
    }

    let mut line = format!(
        "{marker} {id:>5}  {:>4.0}%  {label}{port}{flags}",
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
            println!("{}", device_line(device, &ports));
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
    let mut any = false;
    for device in devices.iter().filter(|d| d.3 == kind.as_str()) {
        println!("{}", device_line(device, &ports));
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
        let line = device_line(&d, &[]);
        assert!(line.starts_with('*'));
        assert!(line.contains("50%"));
        assert!(line.contains("[muted]"));
        assert!(line.contains("[virtual]"));
        assert!(line.contains("sink-a"));

        let plain = device_line(&device(12, "sink-b", "HDMI", "Dell AW3423DW"), &[]);
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
        );
        let first = line.lines().next().expect("a first line");
        assert!(first.contains("ALC892 Analog"));
        assert!(!first.contains("Starship"));
        assert!(line.contains("Starship/Matisse HD Audio Controller Analog Stereo"));
        assert!(line.contains("alsa_output.pci-0000_28_00.4.analog-stereo"));

        // No nick: fall back to the description, and do not print it twice.
        let bare = device_line(&device(12, "sink-b", "HDMI", ""), &[]);
        assert_eq!(bare.matches("HDMI").count(), 1);
    }

    #[test]
    fn device_line_shows_the_active_port_only_when_there_is_a_choice() {
        let multi = device_line(&device(39, "sink-a", "Analog", "ALC892 Analog"), &ports());
        assert!(multi.contains("[Headphones]"));

        // One port is not a choice: no bracket.
        let single = device_line(&device(43, "sink-hdmi", "HDMI", "Dell AW3423DW"), &ports());
        assert!(!single.contains("[HDMI / DisplayPort]"));

        // No ports at all (virtual sink).
        let none = device_line(&device(60, "null", "Null", "Null"), &ports());
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
}
