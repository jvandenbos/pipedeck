//! `pipedeck` — the PipeDeck CLI (SPEC §2.3).
//!
//! A thin D-Bus client: it never touches PipeWire directly, so anything it can
//! do the GNOME Shell extension can do too.

mod proxy;

use anyhow::{bail, Context as _, Result};
use clap::{Args, Parser, Subcommand};
use futures_util::StreamExt as _;

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

fn device_line(device: &DeviceTuple) -> String {
    let (id, name, description, _kind, is_default, virtual_, volume, mute) = device;
    let marker = if *is_default { '*' } else { ' ' };
    let mut flags = String::new();
    if *mute {
        flags.push_str(" [muted]");
    }
    if *virtual_ {
        flags.push_str(" [virtual]");
    }
    format!(
        "{marker} {id:>5}  {:>4.0}%  {description}{flags}\n            {name}",
        linear_to_percent(*volume)
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
            println!("{}", device_line(device));
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
    let mut any = false;
    for device in devices.iter().filter(|d| d.3 == kind.as_str()) {
        println!("{}", device_line(device));
        any = true;
    }
    if !any {
        println!("(no {kind}s)");
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

    #[test]
    fn device_line_marks_the_default_and_flags() {
        let line = device_line(&(
            11,
            "sink-a".to_owned(),
            "Analog".to_owned(),
            "sink".to_owned(),
            true,
            true,
            0.125, // 50 % on the cubic scale wpctl shows
            true,
        ));
        assert!(line.starts_with('*'));
        assert!(line.contains("50%"));
        assert!(line.contains("[muted]"));
        assert!(line.contains("[virtual]"));
        assert!(line.contains("sink-a"));

        let plain = device_line(&(
            12,
            "sink-b".to_owned(),
            "HDMI".to_owned(),
            "sink".to_owned(),
            false,
            false,
            1.0,
            false,
        ));
        assert!(plain.starts_with(' '));
        assert!(!plain.contains("[muted]"));
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
