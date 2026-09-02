//! The `dev.pipedeck.Daemon1` D-Bus interface (SPEC §2.2).
//!
//! `missing_docs` is off for this module only: `#[zbus::interface]` emits a
//! `DaemonSignals` trait whose methods it does not document, and the lint fires
//! on generated code we cannot annotate. Everything hand-written here still
//! carries doc comments.
#![allow(missing_docs)]

use std::sync::{Arc, RwLock};

use tokio::sync::{oneshot, watch, Mutex};
use tracing::{debug, warn};
use zbus::object_server::SignalEmitter;

use crate::command::{await_reply, Command};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::pw::PwHandle;
use crate::state::{DeviceKind, DeviceTuple, State, StreamTuple};
use crate::volume::clamp_volume;

/// Well-known bus name the daemon owns.
pub const BUS_NAME: &str = "dev.pipedeck.Daemon";
/// Object path the interface is exported at.
pub const OBJECT_PATH: &str = "/dev/pipedeck/Daemon";
/// Interface name.
pub const INTERFACE: &str = "dev.pipedeck.Daemon1";

/// Interface implementation. Holds no PipeWire types: it reads the shared
/// snapshot and posts commands into the PipeWire thread.
pub struct Daemon {
    state: Arc<RwLock<State>>,
    pw: PwHandle,
    /// Serialised so a burst of `SetNotificationSink` calls cannot interleave a
    /// read-modify-write on the config file.
    config: Arc<Mutex<Config>>,
    config_path: Option<std::path::PathBuf>,
}

impl Daemon {
    /// Build the interface object.
    #[must_use]
    pub fn new(
        state: Arc<RwLock<State>>,
        pw: PwHandle,
        config: Config,
        config_path: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            state,
            pw,
            config: Arc::new(Mutex::new(config)),
            config_path,
        }
    }

    fn snapshot(&self) -> State {
        match self.state.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    async fn dispatch(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<()>>) -> Command,
    ) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.pw.send(make(tx))?;
        await_reply(rx).await
    }
}

#[zbus::interface(name = "dev.pipedeck.Daemon1")]
impl Daemon {
    /// Sinks and sources: `(id, name, description, kind, is_default, virtual, volume, mute)`.
    #[zbus(property)]
    async fn devices(&self) -> Vec<DeviceTuple> {
        self.snapshot().devices_dbus()
    }

    /// Playback streams: `(id, app_name, binary, media_name, target_name, volume, mute)`.
    #[zbus(property)]
    async fn streams(&self) -> Vec<StreamTuple> {
        self.snapshot().streams_dbus()
    }

    /// `node.name` of the notification sink; empty means "follow the default output".
    #[zbus(property)]
    async fn notification_sink(&self) -> String {
        self.config.lock().await.notification_sink.clone()
    }

    /// Daemon version string.
    #[zbus(property)]
    async fn version(&self) -> String {
        crate::VERSION.to_owned()
    }

    /// Make `name` the default sink or source.
    async fn set_default(&self, kind: &str, name: &str) -> Result<()> {
        let kind = DeviceKind::parse(kind).ok_or_else(|| {
            Error::invalid(format!("kind must be `sink` or `source`, got `{kind}`"))
        })?;
        if name.is_empty() {
            return Err(Error::invalid("name must not be empty"));
        }
        let name = name.to_owned();
        self.dispatch(move |reply| Command::SetDefault { kind, name, reply })
            .await
    }

    /// Set the notification sink; an empty name follows the default output.
    async fn set_notification_sink(
        &self,
        name: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        let name = name.trim().to_owned();
        if !name.is_empty()
            && self
                .snapshot()
                .device_by_name_of_kind(&name, DeviceKind::Sink)
                .is_none()
        {
            return Err(Error::not_found(format!("no sink named `{name}`")));
        }

        let mut guard = self.config.lock().await;
        if guard.notification_sink == name {
            return Ok(());
        }
        let previous = std::mem::replace(&mut guard.notification_sink, name);
        let config = guard.clone();

        if let Some(path) = self.config_path.as_ref() {
            if let Err(e) = config.save_to(path) {
                guard.notification_sink = previous;
                return Err(Error::from(e));
            }
        }
        drop(guard);

        let result = self
            .dispatch(move |reply| Command::SetConfig {
                config: Box::new(config),
                reply,
            })
            .await;
        if result.is_ok() {
            let _ = self.notification_sink_changed(&emitter).await;
        }
        result
    }

    /// Set the linear volume (0.0–1.5) of a device or stream node.
    async fn set_volume(&self, id: u32, volume: f64) -> Result<()> {
        if volume.is_nan() {
            return Err(Error::invalid("volume must be a number"));
        }
        if !(0.0..=crate::volume::MAX_VOLUME).contains(&volume) {
            return Err(Error::invalid(format!(
                "volume must be between 0.0 and {}, got {volume}",
                crate::volume::MAX_VOLUME
            )));
        }
        let volume = clamp_volume(volume);
        self.dispatch(move |reply| Command::SetVolume { id, volume, reply })
            .await
    }

    /// Mute or unmute a device or stream node.
    async fn set_mute(&self, id: u32, mute: bool) -> Result<()> {
        self.dispatch(move |reply| Command::SetMute { id, mute, reply })
            .await
    }

    /// Route one stream at a named sink; an empty name restores the default.
    async fn set_stream_target(&self, id: u32, name: &str) -> Result<()> {
        let name = name.trim().to_owned();
        self.dispatch(move |reply| Command::SetStreamTarget { id, name, reply })
            .await
    }

    /// Re-read every node's params and re-publish the snapshot.
    async fn refresh(&self) -> Result<()> {
        self.dispatch(|reply| Command::Refresh { reply }).await
    }

    /// "Something changed, re-read the properties." Coalesced to <= 10/s.
    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Minimum gap between `Changed` signals, giving the <= 10/s of SPEC §2.2.
pub const CHANGED_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Watch the PipeWire thread's revision counter and emit coalesced change
/// notifications: one `Changed` signal plus `PropertiesChanged` for the two
/// list properties, at most ten times a second.
pub async fn run_change_notifier(
    iface: zbus::object_server::InterfaceRef<Daemon>,
    mut revisions: watch::Receiver<u64>,
) {
    loop {
        if revisions.changed().await.is_err() {
            debug!("revision channel closed; change notifier stopping");
            return;
        }
        // Collapse anything that arrived while we were busy.
        tokio::time::sleep(CHANGED_INTERVAL).await;
        revisions.mark_unchanged();

        let emitter = iface.signal_emitter();
        let guard = iface.get().await;
        if let Err(e) = guard.devices_changed(emitter).await {
            warn!("could not emit PropertiesChanged for Devices: {e}");
        }
        if let Err(e) = guard.streams_changed(emitter).await {
            warn!("could not emit PropertiesChanged for Streams: {e}");
        }
        drop(guard);
        if let Err(e) = Daemon::changed(emitter).await {
            warn!("could not emit Changed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::object_server::Interface as _;

    fn daemon() -> Daemon {
        Daemon::new(
            Arc::new(RwLock::new(State::default())),
            PwHandle::disconnected(),
            Config::default(),
            None,
        )
    }

    /// The XML the GNOME Shell extension embeds must describe the interface the
    /// daemon actually exports, so it is checked against the live one here.
    /// If this fails, paste the printed block into
    /// `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml` between `<node>` and
    /// `</node>`.
    #[test]
    fn introspection_xml_matches_the_checked_in_copy() {
        let mut generated = String::new();
        daemon().introspect_to_writer(&mut generated, 0);
        let checked_in = include_str!("../dbus/dev.pipedeck.Daemon1.xml");
        assert!(
            checked_in.contains(generated.trim()),
            "dbus/dev.pipedeck.Daemon1.xml is stale. Regenerate it from:\n\
             ---8<--- generated ---8<---\n{generated}\n---8<--- end ---8<---"
        );
    }

    /// SPEC §2.2 pins the property signatures; a change here breaks the
    /// extension silently, so assert on them directly.
    #[test]
    fn property_signatures_match_the_spec() {
        let mut xml = String::new();
        daemon().introspect_to_writer(&mut xml, 0);
        assert!(xml.contains(r#"<property name="Devices" type="a(usssbbdb)" access="read"/>"#));
        assert!(xml.contains(r#"<property name="Streams" type="a(ussssdb)" access="read"/>"#));
        assert!(xml.contains(r#"<property name="NotificationSink" type="s" access="read"/>"#));
        assert!(xml.contains(r#"<property name="Version" type="s" access="read"/>"#));
        assert!(xml.contains(r#"<signal name="Changed">"#));
        for method in [
            "SetDefault",
            "SetNotificationSink",
            "SetVolume",
            "SetMute",
            "SetStreamTarget",
            "Refresh",
        ] {
            assert!(
                xml.contains(&format!(r#"<method name="{method}">"#)),
                "missing method {method}"
            );
        }
    }

    /// With no PipeWire thread behind it, every method must fail with
    /// `dev.pipedeck.Error.PipeWire` rather than hang or panic.
    #[tokio::test]
    async fn methods_fail_cleanly_without_a_graph() {
        let daemon = daemon();
        assert!(matches!(
            daemon.set_volume(3, 0.5).await,
            Err(Error::PipeWire(_))
        ));
        assert!(matches!(
            daemon.set_mute(3, true).await,
            Err(Error::PipeWire(_))
        ));
        assert!(matches!(daemon.refresh().await, Err(Error::PipeWire(_))));
    }

    /// Argument validation happens before anything is queued, so these must be
    /// `InvalidArgument`/`NotFound` even with no graph.
    #[tokio::test]
    async fn arguments_are_validated_up_front() {
        let daemon = daemon();
        assert!(matches!(
            daemon.set_default("speaker", "x").await,
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            daemon.set_default("sink", "").await,
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            daemon.set_volume(1, 2.0).await,
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            daemon.set_volume(1, -0.5).await,
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            daemon.set_volume(1, f64::NAN).await,
            Err(Error::InvalidArgument(_))
        ));
    }

    /// Properties read straight from the shared snapshot.
    #[tokio::test]
    async fn properties_project_the_snapshot() {
        use crate::state::{Device, DeviceKind};

        let state = Arc::new(RwLock::new(State::default()));
        {
            let mut guard = state.write().expect("lock");
            guard.devices.insert(
                11,
                Device {
                    id: 11,
                    name: "sink-a".to_owned(),
                    description: "Analog".to_owned(),
                    kind: DeviceKind::Sink,
                    is_default: true,
                    virtual_: false,
                    volume: 0.5,
                    mute: false,
                },
            );
        }
        let daemon = Daemon::new(
            state,
            PwHandle::disconnected(),
            Config {
                notification_sink: "sink-a".to_owned(),
                ..Config::default()
            },
            None,
        );
        let devices = daemon.devices().await;
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].1, "sink-a");
        assert_eq!(daemon.notification_sink().await, "sink-a");
        assert_eq!(daemon.version().await, crate::VERSION);
        assert!(daemon.streams().await.is_empty());
    }

    /// The coalescing floor is what keeps `Changed` at or under 10/s.
    #[test]
    fn changed_is_coalesced_to_ten_per_second() {
        assert!(CHANGED_INTERVAL >= std::time::Duration::from_millis(100));
    }
}
