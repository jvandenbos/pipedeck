//! `pipedeckd` — the PipeDeck daemon binary.

use std::sync::{Arc, RwLock};

use anyhow::{Context as _, Result};
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use pipedeckd::config::Config;
use pipedeckd::pw::PwThread;
use pipedeckd::service::{self, Daemon};
use pipedeckd::state::State;

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("pipedeckd {}", pipedeckd::VERSION);
        return Ok(());
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "pipedeckd {}\n\nPipeWire audio control daemon. Runs as a systemd user service and\n\
             serves {} on the session bus.\n\nOptions:\n  -h, --help       show this help\n  \
             -V, --version    show the version\n\nLogging is controlled by RUST_LOG (default: info).",
            pipedeckd::VERSION,
            service::INTERFACE
        );
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the tokio runtime")?;
    runtime.block_on(serve())
}

async fn serve() -> Result<()> {
    let config_path = match Config::path() {
        Ok(path) => Some(path),
        Err(e) => {
            warn!("running without an on-disk config: {e}");
            None
        }
    };
    let config = match config_path.as_ref() {
        Some(path) => Config::load_from(path)
            .with_context(|| format!("could not read the config at {}", path.display()))?,
        None => Config::default(),
    };
    info!(
        notification_sink = %if config.notification_sink.is_empty() {
            "<default output>"
        } else {
            config.notification_sink.as_str()
        },
        "loaded config"
    );

    let state = Arc::new(RwLock::new(State::default()));
    let (revision_tx, revision_rx) = watch::channel(0_u64);
    let (exited_tx, mut exited_rx) = mpsc::unbounded_channel();

    let pw = PwThread::spawn(config.clone(), state.clone(), revision_tx, exited_tx)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let daemon = Daemon::new(state, pw.handle(), config, config_path);

    // `serve_at` + `request_name` (rather than `name`) so a second instance
    // fails loudly instead of silently queueing behind the first.
    let connection = zbus::connection::Builder::session()
        .context("could not connect to the session bus")?
        .serve_at(service::OBJECT_PATH, daemon)
        .context("could not export the interface")?
        .build()
        .await
        .context("could not set up the session bus connection")?;

    connection
        .request_name_with_flags(
            service::BUS_NAME,
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await
        .with_context(|| {
            format!(
                "could not take the bus name {} (is another pipedeckd running?)",
                service::BUS_NAME
            )
        })?;

    let iface = connection
        .object_server()
        .interface::<_, Daemon>(service::OBJECT_PATH)
        .await
        .context("could not look up the exported interface")?;

    let notifier = tokio::spawn(service::run_change_notifier(iface, revision_rx));

    info!(
        "serving {} at {} as {}",
        service::INTERFACE,
        service::OBJECT_PATH,
        service::BUS_NAME
    );

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("could not install the SIGTERM handler")?;

    tokio::select! {
        r = tokio::signal::ctrl_c() => {
            if let Err(e) = r {
                error!("ctrl-c handler failed: {e}");
            }
            info!("interrupted; shutting down");
        }
        _ = sigterm.recv() => info!("SIGTERM; shutting down"),
        _ = exited_rx.recv() => error!("the PipeWire thread stopped; shutting down"),
    }

    notifier.abort();
    drop(connection);
    Ok(())
}
