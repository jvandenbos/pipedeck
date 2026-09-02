//! Commands sent from the tokio/D-Bus side into the PipeWire loop thread.
//!
//! Deliberately free of PipeWire types so the D-Bus layer stays testable.

use tokio::sync::oneshot;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::state::DeviceKind;

/// One-shot reply channel for a command.
pub type Reply = oneshot::Sender<Result<()>>;

/// Work items for the PipeWire thread.
#[derive(Debug)]
pub enum Command {
    /// Write `default.configured.audio.{sink,source}` in the `default` metadata.
    SetDefault {
        /// Sink or source.
        kind: DeviceKind,
        /// `node.name` of the target device.
        name: String,
        /// Where to report success or failure.
        reply: Reply,
    },
    /// Set `channelVolumes` on a device or stream node.
    SetVolume {
        /// Node id.
        id: u32,
        /// Linear volume, already clamped by the caller.
        volume: f64,
        /// Where to report success or failure.
        reply: Reply,
    },
    /// Set `mute` on a device or stream node.
    SetMute {
        /// Node id.
        id: u32,
        /// New mute state.
        mute: bool,
        /// Where to report success or failure.
        reply: Reply,
    },
    /// Select a card route (port) for a node, via the device's `Route` param.
    SetPort {
        /// Node id of the sink or source the port belongs to.
        id: u32,
        /// Route index, from the `Ports` property.
        index: u32,
        /// Where to report success or failure.
        reply: Reply,
    },
    /// Route one stream at a named sink; an empty name clears the override.
    SetStreamTarget {
        /// Stream node id.
        id: u32,
        /// `node.name` of the sink, or "" to fall back to the default.
        name: String,
        /// Where to report success or failure.
        reply: Reply,
    },
    /// Replace the live config and re-apply notification routing.
    SetConfig {
        /// The new config.
        config: Box<Config>,
        /// Where to report success or failure.
        reply: Reply,
    },
    /// Re-enumerate params on every tracked node and re-publish the snapshot.
    Refresh {
        /// Where to report success or failure.
        reply: Reply,
    },
    /// Quit the PipeWire main loop.
    Terminate,
}

impl Command {
    /// Consume the command's reply channel, if it has one.
    pub fn into_reply(self) -> Option<Reply> {
        match self {
            Command::SetDefault { reply, .. }
            | Command::SetVolume { reply, .. }
            | Command::SetMute { reply, .. }
            | Command::SetPort { reply, .. }
            | Command::SetStreamTarget { reply, .. }
            | Command::SetConfig { reply, .. }
            | Command::Refresh { reply } => Some(reply),
            Command::Terminate => None,
        }
    }
}

/// Await a command's reply, turning a dropped channel into a `PipeWire` error.
///
/// A dropped sender means the PipeWire thread died, which is exactly the
/// condition SPEC §2.2 wants surfaced as `dev.pipedeck.Error.PipeWire`.
pub async fn await_reply(rx: oneshot::Receiver<Result<()>>) -> Result<()> {
    rx.await
        .unwrap_or_else(|_| Err(Error::pipewire("PipeWire thread is not running")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn await_reply_passes_results_through() {
        let (tx, rx) = oneshot::channel();
        tx.send(Ok(())).expect("send");
        assert!(await_reply(rx).await.is_ok());
    }

    #[tokio::test]
    async fn dropped_sender_becomes_a_pipewire_error() {
        let (tx, rx) = oneshot::channel::<Result<()>>();
        drop(tx);
        let err = await_reply(rx).await.expect_err("must fail");
        assert!(matches!(err, Error::PipeWire(_)));
    }

    #[test]
    fn terminate_has_no_reply() {
        assert!(Command::Terminate.into_reply().is_none());
        let (tx, _rx) = oneshot::channel();
        let cmd = Command::Refresh { reply: tx };
        assert!(cmd.into_reply().is_some());
    }
}
