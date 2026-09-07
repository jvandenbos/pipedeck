//! The `dev.pipedeck.Daemon1` D-Bus interface (SPEC §2.2).
//!
//! `missing_docs` is off for this module only: `#[zbus::interface]` emits a
//! `DaemonSignals` trait whose methods it does not document, and the lint fires
//! on generated code we cannot annotate. Everything hand-written here still
//! carries doc comments.
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use tokio::sync::{oneshot, watch, Mutex};
use tracing::{debug, info, warn};
use zbus::object_server::SignalEmitter;

use crate::alsa_mixer;
use crate::command::{await_reply, Command};
use crate::config::{self, Config};
use crate::eq::{self, Preset};
use crate::error::{Error, Result};
use crate::pw::PwHandle;
use crate::route::PortTuple;
use crate::state::{
    AlsaCard, AutoMuteTuple, DeviceKind, DeviceTuple, EqPresetTuple, EqTuple, State, StreamTuple,
};
use crate::volume::clamp_volume;

/// `Auto-Mute Mode` probe results, by ALSA card index (SPEC §8.1).
///
/// `Some(enabled)` is a card with the control; `None` is a card without one.
/// The negative is cached deliberately: without it every graph change would
/// re-open the mixer of every card that can never answer.
pub type AutoMuteCache = BTreeMap<u32, Option<bool>>;

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
    /// The preset library, shared with the PipeWire thread (SPEC §7.3).
    presets: Arc<RwLock<Vec<Preset>>>,
    /// Where presets are scanned from; `None` disables rescanning.
    presets_dir: Option<std::path::PathBuf>,
    /// Cached `Auto-Mute Mode` state per ALSA card (SPEC §8.1). Only ever
    /// touched from the tokio side; the mixer reads and writes behind it run on
    /// `spawn_blocking`, never on the PipeWire thread.
    auto_mute_cache: Arc<Mutex<AutoMuteCache>>,
}

impl Daemon {
    /// Build the interface object.
    #[must_use]
    pub fn new(
        state: Arc<RwLock<State>>,
        pw: PwHandle,
        config: Config,
        config_path: Option<std::path::PathBuf>,
        presets: Arc<RwLock<Vec<Preset>>>,
        presets_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            state,
            pw,
            config: Arc::new(Mutex::new(config)),
            config_path,
            presets,
            presets_dir,
            auto_mute_cache: Arc::new(Mutex::new(AutoMuteCache::new())),
        }
    }

    /// Re-read the presets directory into the shared library (SPEC §7.3: the
    /// `EqPresets` property is rescanned on `Refresh` and on `SetEq`).
    ///
    /// A directory that cannot be read leaves the previous library in place — a
    /// transient I/O error must not silently empty the panel's preset list.
    fn rescan_presets(&self) {
        let Some(dir) = self.presets_dir.as_ref() else {
            return;
        };
        let (presets, problems) = eq::load_presets(dir);
        for problem in problems {
            warn!("skipping EQ preset: {problem}");
        }
        match self.presets.write() {
            Ok(mut guard) => *guard = presets,
            Err(poisoned) => {
                warn!("preset lock was poisoned; recovering");
                *poisoned.into_inner() = presets;
            }
        }
    }

    /// SPEC §7.3's `SetEq` validation, split out so it can be tested without a
    /// D-Bus connection (the interface method needs a `SignalEmitter`).
    ///
    /// Returns the target sink's `node.name` and the trimmed preset id.
    fn validate_set_eq(&self, node_id: u32, preset: &str) -> Result<(String, String)> {
        let preset = preset.trim();
        let snapshot = self.snapshot();
        let device = snapshot
            .devices
            .get(&node_id)
            .ok_or_else(|| Error::not_found(format!("no device with id {node_id}")))?;
        if device.kind != DeviceKind::Sink {
            return Err(Error::invalid(format!(
                "node {node_id} is an input; EQ applies to output devices only"
            )));
        }
        if !preset.is_empty() && !self.preset_list().iter().any(|p| p.id == preset) {
            return Err(Error::not_found(format!("no EQ preset `{preset}`")));
        }
        Ok((device.name.clone(), preset.to_owned()))
    }

    /// A snapshot of the preset library.
    fn preset_list(&self) -> Vec<Preset> {
        match self.presets.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn snapshot(&self) -> State {
        match self.state.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    // -----------------------------------------------------------------
    // ALSA auto-mute (SPEC §8.1)
    // -----------------------------------------------------------------

    /// The `AutoMute` property payload: one row per sink whose card has the
    /// control, ordered by node id.
    ///
    /// Pure, so the projection can be tested without a mixer.
    #[must_use]
    fn auto_mute_rows(
        cards: &BTreeMap<u32, AlsaCard>,
        cache: &AutoMuteCache,
    ) -> Vec<AutoMuteTuple> {
        cards
            .iter()
            .filter_map(|(node_id, card)| {
                cache
                    .get(&card.index)
                    .copied()
                    .flatten()
                    .map(|enabled| (*node_id, enabled))
            })
            .collect()
    }

    /// The distinct ALSA cards behind the snapshot's sinks, index -> name.
    ///
    /// Two sinks on one card (rare, but real on split codecs) collapse to one
    /// entry — the mixer control is per card, not per sink.
    #[must_use]
    fn snapshot_cards(state: &State) -> BTreeMap<u32, String> {
        state
            .cards
            .values()
            .map(|card| (card.index, card.name.clone()))
            .collect()
    }

    /// Bring the auto-mute cache in step with the graph (SPEC §8.1).
    ///
    /// `force` re-probes every card the graph still has; otherwise only cards
    /// that are not in the cache yet are probed — which is precisely "a routed
    /// sink appeared", and, on the first run after startup, "startup".
    ///
    /// A **newly discovered** card also has its persisted choice re-applied,
    /// because `alsa-restore` puts the boot-time value back on every login. A
    /// forced re-probe deliberately does *not* re-apply: acceptance §8.3 14
    /// turns the control back on with `amixer` and then expects `SetPort` to be
    /// what turns it off again, so a re-probe must observe, never correct.
    ///
    /// Returns true when the published rows changed.
    async fn sync_auto_mute(&self, force: bool) -> bool {
        let wanted = Self::snapshot_cards(&self.snapshot());
        let stored: BTreeMap<String, bool> = self
            .config
            .lock()
            .await
            .auto_mute_entries()
            .into_iter()
            .collect();

        let mut cache = self.auto_mute_cache.lock().await;
        let before = cache.clone();
        // A card whose sink has gone stops being reported, and is probed
        // afresh (and re-corrected) if it ever comes back.
        cache.retain(|index, _| wanted.contains_key(index));

        for (index, name) in wanted {
            let fresh = !cache.contains_key(&index);
            if !fresh && !force {
                continue;
            }
            let mut probed = spawn_probe(index).await;
            if fresh {
                if let (Some(current), Some(&choice)) = (probed, stored.get(&name)) {
                    if current != choice {
                        match spawn_set(index, choice).await {
                            Ok(()) => {
                                info!(
                                    card = index,
                                    card_name = %name,
                                    enabled = choice,
                                    "re-applied the stored Auto-Mute Mode choice"
                                );
                                probed = Some(choice);
                            }
                            Err(e) => warn!(card = index, "could not re-apply Auto-Mute Mode: {e}"),
                        }
                    }
                }
            }
            cache.insert(index, probed);
        }
        *cache != before
    }

    /// SPEC §8.1's automatic switch, run after a successful `SetPort`.
    ///
    /// `route_name` is the port that was *requested*: the snapshot's `active`
    /// flag only catches up once the card re-enumerates its routes, so reading
    /// it back here would race.
    async fn maybe_disable_auto_mute(&self, node_id: u32, route_name: &str) {
        let snapshot = self.snapshot();
        let Some(card) = snapshot.card_of(node_id).cloned() else {
            return;
        };
        let headphones_available = snapshot.headphones_available(node_id);

        let policy = self.config.lock().await.auto_mute_policy();
        let mut cache = self.auto_mute_cache.lock().await;
        let current = match cache.get(&card.index).copied() {
            Some(known) => known,
            None => {
                let probed = spawn_probe(card.index).await;
                cache.insert(card.index, probed);
                probed
            }
        };
        let Some(enabled) = current else {
            return;
        };
        if !alsa_mixer::should_disable_auto_mute(route_name, headphones_available, enabled, policy)
        {
            return;
        }

        if let Err(e) = spawn_set(card.index, false).await {
            warn!(card = card.index, "could not turn Auto-Mute Mode off: {e}");
            return;
        }
        cache.insert(card.index, Some(false));
        drop(cache);
        info!(
            node = node_id,
            card = card.index,
            card_name = %card.name,
            port = route_name,
            "headphones are plugged in and a speaker port was selected; \
             turned ALSA Auto-Mute Mode off"
        );
        self.persist_auto_mute(&card.name, false).await;
    }

    /// Remember a card's `Auto-Mute Mode` choice in the config file.
    ///
    /// A write failure is logged, not returned: the mixer change has already
    /// happened, and refusing the whole call would leave the daemon reporting a
    /// state it just moved away from.
    async fn persist_auto_mute(&self, card_name: &str, enabled: bool) {
        let mut guard = self.config.lock().await;
        if guard.auto_mute(card_name) == Some(enabled) {
            return;
        }
        guard.set_auto_mute(card_name, enabled);
        let config = guard.clone();
        drop(guard);

        if let Some(path) = self.config_path.as_ref() {
            if let Err(e) = config.save_to(path) {
                warn!("could not persist the Auto-Mute Mode choice: {e}");
                return;
            }
        }
        let _ = self
            .dispatch(move |reply| Command::SetConfig {
                config: Box::new(config),
                reply,
            })
            .await;
    }

    /// `SetPort` without the `PropertiesChanged` emission, so it can be tested
    /// without a D-Bus connection (the interface method needs a
    /// `SignalEmitter`).
    async fn do_set_port(&self, node_id: u32, route_index: u32) -> Result<()> {
        // Read the requested route's name *before* the write: the snapshot's
        // `active` flag only catches up once the card re-enumerates.
        let route_name = self
            .snapshot()
            .port_name(node_id, route_index)
            .map(str::to_owned);

        self.dispatch(move |reply| Command::SetPort {
            id: node_id,
            index: route_index,
            reply,
        })
        .await?;

        // SPEC §8.1: the automatic switch, then a re-probe so the property
        // reflects whatever the card actually ended up at.
        if let Some(route_name) = route_name {
            self.maybe_disable_auto_mute(node_id, &route_name).await;
        }
        self.sync_auto_mute(true).await;
        Ok(())
    }

    /// `SetAutoMute` without the `PropertiesChanged` emission (SPEC §8.1).
    async fn do_set_auto_mute(&self, node_id: u32, enabled: bool) -> Result<()> {
        let card = self.validate_set_auto_mute(node_id)?;

        // A card we have never probed gets probed now, so "this card has no
        // such control" is an `InvalidArgument` rather than a silent no-op.
        {
            let mut cache = self.auto_mute_cache.lock().await;
            let known = match cache.get(&card.index).copied() {
                Some(known) => known,
                None => {
                    let probed = spawn_probe(card.index).await;
                    cache.insert(card.index, probed);
                    probed
                }
            };
            if known.is_none() {
                return Err(Error::invalid(format!(
                    "card {} behind node {node_id} has no `{}` control",
                    card.index,
                    alsa_mixer::AUTO_MUTE_CONTROL
                )));
            }
        }

        spawn_set(card.index, enabled)
            .await
            .map_err(Error::pipewire)?;
        {
            let mut cache = self.auto_mute_cache.lock().await;
            cache.insert(card.index, Some(enabled));
        }
        info!(
            node = node_id,
            card = card.index,
            card_name = %card.name,
            enabled,
            "Auto-Mute Mode set"
        );
        self.persist_auto_mute(&card.name, enabled).await;

        // SPEC §8.1: re-probe after every SetAutoMute.
        self.sync_auto_mute(true).await;
        Ok(())
    }

    /// `SetPortSwitchCap` without the `PropertiesChanged` emission (SPEC §9.2).
    ///
    /// Saves the config, then hands the PipeWire thread the new value — the
    /// same read-modify-write-dispatch shape `SetEq` and the notification sink
    /// use, so a failed save leaves both sides on the old cap.
    async fn do_set_port_switch_cap(&self, percent: u32) -> Result<()> {
        if percent > config::MAX_PORT_SWITCH_MAX_PERCENT {
            return Err(Error::invalid(format!(
                "the port-switch cap must be 0-{} percent (0 turns it off), got {percent}",
                config::MAX_PORT_SWITCH_MAX_PERCENT
            )));
        }

        let mut guard = self.config.lock().await;
        if guard.port_switch_max_percent() == percent {
            return Ok(());
        }
        let previous = guard.safety.port_switch_max_percent;
        guard.set_port_switch_max_percent(percent);
        let config = guard.clone();

        if let Some(path) = self.config_path.as_ref() {
            if let Err(e) = config.save_to(path) {
                guard.safety.port_switch_max_percent = previous;
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
            info!(percent, "port-switch level cap set");
        }
        result
    }

    /// `Refresh` without the `PropertiesChanged` emission.
    async fn do_refresh(&self) -> Result<()> {
        // SPEC §7.3: `EqPresets` is rescanned on Refresh.
        self.rescan_presets();
        self.dispatch(|reply| Command::Refresh { reply }).await?;
        // SPEC §8.1: auto-mute is re-probed on Refresh.
        self.sync_auto_mute(true).await;
        Ok(())
    }

    /// SPEC §8.1's `SetAutoMute` validation, split out so it can be tested
    /// without a D-Bus connection or a mixer.
    ///
    /// `NotFound` for an unknown node, `InvalidArgument` for a node that is not
    /// an output device or whose card PipeDeck has no ALSA card row for.
    fn validate_set_auto_mute(&self, node_id: u32) -> Result<AlsaCard> {
        let snapshot = self.snapshot();
        let device = snapshot
            .devices
            .get(&node_id)
            .ok_or_else(|| Error::not_found(format!("no device with id {node_id}")))?;
        if device.kind != DeviceKind::Sink {
            return Err(Error::invalid(format!(
                "node {node_id} is an input; auto-mute is an output control"
            )));
        }
        snapshot
            .card_of(node_id)
            .cloned()
            .ok_or_else(|| Error::invalid(format!("node {node_id} is not backed by an ALSA card")))
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

/// Read `Auto-Mute Mode` off a card without blocking the async runtime.
///
/// SPEC §8.1: "Mixer calls are quick but blocking; run them on the tokio side
/// via `spawn_blocking`, never on the PipeWire thread."
async fn spawn_probe(card: u32) -> Option<bool> {
    match tokio::task::spawn_blocking(move || alsa_mixer::probe(card)).await {
        Ok(result) => result,
        Err(e) => {
            warn!(card, "the auto-mute probe task failed: {e}");
            None
        }
    }
}

/// Write `Auto-Mute Mode` on a card without blocking the async runtime.
async fn spawn_set(card: u32, enabled: bool) -> std::result::Result<(), String> {
    match tokio::task::spawn_blocking(move || alsa_mixer::set(card, enabled)).await {
        Ok(result) => result,
        Err(e) => Err(format!("the auto-mute write task failed: {e}")),
    }
}

#[zbus::interface(name = "dev.pipedeck.Daemon1")]
impl Daemon {
    /// Sinks and sources: `(id, name, description, kind, is_default, virtual, volume, mute, nick)`.
    #[zbus(property)]
    async fn devices(&self) -> Vec<DeviceTuple> {
        self.snapshot().devices_dbus()
    }

    /// Selectable ports: `(node_id, route_index, name, description, available, active)`.
    #[zbus(property)]
    async fn ports(&self) -> Vec<PortTuple> {
        self.snapshot().ports_dbus()
    }

    /// Playback streams: `(id, app_name, binary, media_name, target_name, volume, mute)`.
    #[zbus(property)]
    async fn streams(&self) -> Vec<StreamTuple> {
        self.snapshot().streams_dbus()
    }

    /// Available EQ presets: `(id, name)`, scanned from the presets directory.
    #[zbus(property)]
    async fn eq_presets(&self) -> Vec<EqPresetTuple> {
        self.preset_list().iter().map(Preset::to_dbus).collect()
    }

    /// The EQ preset on each output device: `(node_id, preset id or "")`.
    #[zbus(property)]
    async fn eq(&self) -> Vec<EqTuple> {
        self.snapshot().eq_dbus()
    }

    /// ALSA `Auto-Mute Mode`: `(node_id, enabled)`, one row per sink whose card has the control.
    #[zbus(property)]
    async fn auto_mute(&self) -> Vec<AutoMuteTuple> {
        let cards = self.snapshot().cards;
        let cache = self.auto_mute_cache.lock().await;
        Self::auto_mute_rows(&cards, &cache)
    }

    /// `node.name` of the notification sink; empty means "follow the default output".
    #[zbus(property)]
    async fn notification_sink(&self) -> String {
        self.config.lock().await.notification_sink.clone()
    }

    /// Ceiling on the level a port switch may restore, as a cubic-scale percentage; 0 means off.
    #[zbus(property)]
    async fn port_switch_cap(&self) -> u32 {
        self.config.lock().await.port_switch_max_percent()
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

    /// Set the linear volume (0.0–3.375, i.e. 0–150 % cubic) of a device or stream node.
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

    /// Select a card route (port) for a sink or source node.
    async fn set_port(
        &self,
        node_id: u32,
        route_index: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.do_set_port(node_id, route_index).await?;
        let _ = self.auto_mute_changed(&emitter).await;
        Ok(())
    }

    /// Turn ALSA `Auto-Mute Mode` on or off for the card behind an output device.
    async fn set_auto_mute(
        &self,
        node_id: u32,
        enabled: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.do_set_auto_mute(node_id, enabled).await?;
        let _ = self.auto_mute_changed(&emitter).await;
        Ok(())
    }

    /// Apply an EQ preset to an output device; an empty preset turns EQ off.
    async fn set_eq(
        &self,
        node_id: u32,
        preset: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        // SPEC §7.3: the preset list is rescanned on SetEq, so a file dropped
        // in a moment ago can be selected without a Refresh first.
        self.rescan_presets();
        let (sink_name, preset) = self.validate_set_eq(node_id, preset)?;

        let mut guard = self.config.lock().await;
        let previous = guard.eq_preset(&sink_name).map(str::to_owned);
        if previous.as_deref().unwrap_or_default() == preset {
            let _ = self.eq_presets_changed(&emitter).await;
            return Ok(());
        }
        guard.set_eq_preset(&sink_name, &preset);
        let config = guard.clone();

        if let Some(path) = self.config_path.as_ref() {
            if let Err(e) = config.save_to(path) {
                guard.set_eq_preset(&sink_name, previous.as_deref().unwrap_or_default());
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
            let _ = self.eq_presets_changed(&emitter).await;
            let _ = self.eq_changed(&emitter).await;
        }
        result
    }

    /// Set the port-switch level cap, as a cubic-scale percentage; 0 turns it off.
    async fn set_port_switch_cap(
        &self,
        percent: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.do_set_port_switch_cap(percent).await?;
        let _ = self.port_switch_cap_changed(&emitter).await;
        Ok(())
    }

    /// Re-read every node's params and re-publish the snapshot.
    async fn refresh(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> Result<()> {
        self.do_refresh().await?;
        let _ = self.auto_mute_changed(&emitter).await;
        Ok(())
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
        // SPEC §8.1: probe (and correct) any card that has just appeared —
        // this is both the "routed sink appeared" hook and, on the first
        // revision after start, the startup one.
        guard.sync_auto_mute(false).await;
        if let Err(e) = guard.devices_changed(emitter).await {
            warn!("could not emit PropertiesChanged for Devices: {e}");
        }
        if let Err(e) = guard.streams_changed(emitter).await {
            warn!("could not emit PropertiesChanged for Streams: {e}");
        }
        if let Err(e) = guard.ports_changed(emitter).await {
            warn!("could not emit PropertiesChanged for Ports: {e}");
        }
        if let Err(e) = guard.eq_changed(emitter).await {
            warn!("could not emit PropertiesChanged for Eq: {e}");
        }
        if let Err(e) = guard.auto_mute_changed(emitter).await {
            warn!("could not emit PropertiesChanged for AutoMute: {e}");
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
            Arc::new(RwLock::new(Vec::new())),
            None,
        )
    }

    fn sink(id: u32, name: &str) -> crate::state::Device {
        crate::state::Device {
            id,
            name: name.to_owned(),
            description: "Analog".to_owned(),
            kind: DeviceKind::Sink,
            is_default: true,
            virtual_: false,
            volume: 0.5,
            mute: false,
            nick: "ALC892 Analog".to_owned(),
        }
    }

    fn preset(id: &str, name: &str) -> Preset {
        Preset {
            id: id.to_owned(),
            name: name.to_owned(),
            preamp_db: 0.0,
            bands: Vec::new(),
        }
    }

    /// A daemon with one sink, one source and one preset, and no PipeWire
    /// thread — enough to exercise every `SetEq` validation path.
    fn eq_daemon() -> Daemon {
        let state = Arc::new(RwLock::new(State::default()));
        {
            let mut guard = state.write().expect("lock");
            guard.devices.insert(39, sink(39, "sink-a"));
            let mut source = sink(41, "source-a");
            source.kind = DeviceKind::Source;
            guard.devices.insert(41, source);
            guard.eq = vec![(39, String::new())];
        }
        Daemon::new(
            state,
            PwHandle::disconnected(),
            Config::default(),
            None,
            Arc::new(RwLock::new(vec![preset("hd650", "Sennheiser HD 650")])),
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
        assert!(xml.contains(r#"<property name="Devices" type="a(usssbbdbs)" access="read"/>"#));
        assert!(xml.contains(r#"<property name="Ports" type="a(uussbb)" access="read"/>"#));
        assert!(xml.contains(r#"<property name="Streams" type="a(ussssdb)" access="read"/>"#));
        assert!(xml.contains(r#"<property name="NotificationSink" type="s" access="read"/>"#));
        assert!(xml.contains(r#"<property name="Version" type="s" access="read"/>"#));
        assert!(xml.contains(r#"<signal name="Changed">"#));
        assert!(xml.contains(r#"<property name="EqPresets" type="a(ss)" access="read"/>"#));
        assert!(xml.contains(r#"<property name="Eq" type="a(us)" access="read"/>"#));
        // SPEC §8.1.
        assert!(xml.contains(r#"<property name="AutoMute" type="a(ub)" access="read"/>"#));
        // SPEC §9.2's deviation: the cap is reachable over D-Bus so the CLI
        // stays a pure D-Bus client.
        assert!(xml.contains(r#"<property name="PortSwitchCap" type="u" access="read"/>"#));
        assert!(xml.contains(r#"<arg name="percent" type="u" direction="in"/>"#));
        assert!(xml.contains(r#"<arg name="node_id" type="u" direction="in"/>"#));
        assert!(xml.contains(r#"<arg name="enabled" type="b" direction="in"/>"#));
        for method in [
            "SetDefault",
            "SetNotificationSink",
            "SetVolume",
            "SetMute",
            "SetStreamTarget",
            "SetPort",
            "SetEq",
            "SetAutoMute",
            "SetPortSwitchCap",
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
        assert!(matches!(daemon.do_refresh().await, Err(Error::PipeWire(_))));
        assert!(matches!(
            daemon.do_set_port(3, 4).await,
            Err(Error::PipeWire(_))
        ));
        // SPEC §9.2: a cap change has to reach the PipeWire thread to count.
        assert!(matches!(
            daemon.do_set_port_switch_cap(45).await,
            Err(Error::PipeWire(_))
        ));
    }

    /// SPEC §9.2: the cap is readable and settable over D-Bus, validates its
    /// argument before touching anything, and no-ops when nothing changes.
    #[tokio::test]
    async fn port_switch_cap_reads_and_validates() {
        let daemon = daemon();
        // The default the property reports is SPEC §9.2's 60 %.
        assert_eq!(daemon.port_switch_cap().await, 60);

        // Above the daemon's own maximum volume is a rejection, not a clamp.
        assert!(matches!(
            daemon.do_set_port_switch_cap(151).await,
            Err(Error::InvalidArgument(_))
        ));
        assert_eq!(daemon.port_switch_cap().await, 60);

        // Setting the value it already has short-circuits before the dispatch
        // that would otherwise fail with no PipeWire thread behind it.
        assert!(daemon.do_set_port_switch_cap(60).await.is_ok());

        // `0` is a legal value (the cap off), so it gets as far as the
        // dispatch — which fails here for want of a PipeWire thread. The
        // config keeps the new value, exactly as `SetEq` and
        // `SetNotificationSink` already behave: the save is the record of
        // intent, the dispatch is the delivery.
        assert!(daemon.do_set_port_switch_cap(0).await.is_err());
        assert_eq!(daemon.port_switch_cap().await, 0);
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
            daemon.set_volume(1, 4.0).await,
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
                    nick: "ALC892 Analog".to_owned(),
                },
            );
            guard.ports = vec![crate::route::Port {
                node_id: 11,
                index: 4,
                name: "analog-output-headphones".to_owned(),
                description: "Headphones".to_owned(),
                available: true,
                active: true,
            }];
        }
        let daemon = Daemon::new(
            state,
            PwHandle::disconnected(),
            Config {
                notification_sink: "sink-a".to_owned(),
                ..Config::default()
            },
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
        );
        let devices = daemon.devices().await;
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].1, "sink-a");
        assert_eq!(devices[0].8, "ALC892 Analog");
        let ports = daemon.ports().await;
        assert_eq!(ports.len(), 1);
        assert_eq!(
            ports[0],
            (
                11,
                4,
                "analog-output-headphones".to_owned(),
                "Headphones".to_owned(),
                true,
                true
            )
        );
        assert_eq!(daemon.notification_sink().await, "sink-a");
        assert_eq!(daemon.version().await, crate::VERSION);
        assert!(daemon.streams().await.is_empty());
    }

    /// SPEC §7.3: `EqPresets` projects the library, `Eq` projects the snapshot.
    #[tokio::test]
    async fn eq_properties_project_the_library_and_the_snapshot() {
        let daemon = eq_daemon();
        assert_eq!(
            daemon.eq_presets().await,
            vec![("hd650".to_owned(), "Sennheiser HD 650".to_owned())]
        );
        assert_eq!(daemon.eq().await, vec![(39, String::new())]);
        // With no presets dir configured, a rescan must not wipe the library.
        daemon.rescan_presets();
        assert_eq!(daemon.eq_presets().await.len(), 1);
    }

    /// SPEC §7.3: NotFound for an unknown node or preset, InvalidArgument for a
    /// node that is not an output device.
    #[test]
    fn set_eq_validates_before_touching_the_graph() {
        let daemon = eq_daemon();

        assert!(matches!(
            daemon.validate_set_eq(99, "hd650"),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            daemon.validate_set_eq(41, "hd650"),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            daemon.validate_set_eq(39, "nope"),
            Err(Error::NotFound(_))
        ));

        assert_eq!(
            daemon.validate_set_eq(39, "hd650").expect("valid"),
            ("sink-a".to_owned(), "hd650".to_owned())
        );
        // "" is off, and needs no preset to exist.
        assert_eq!(
            daemon
                .validate_set_eq(39, "  ")
                .expect("off is always valid"),
            ("sink-a".to_owned(), String::new())
        );
    }

    /// A daemon with one card-backed sink, one HDMI-ish sink with no card, one
    /// source and no PipeWire thread — enough for every `SetAutoMute`
    /// validation path and the `AutoMute` projection (SPEC §8.1).
    fn auto_mute_daemon() -> Daemon {
        use crate::route::Port;

        let state = Arc::new(RwLock::new(State::default()));
        {
            let mut guard = state.write().expect("lock");
            guard.devices.insert(39, sink(39, "sink-a"));
            guard.devices.insert(43, sink(43, "sink-hdmi"));
            let mut source = sink(41, "source-a");
            source.kind = DeviceKind::Source;
            guard.devices.insert(41, source);
            guard.cards.insert(
                39,
                AlsaCard {
                    index: 1,
                    name: "HDA Intel PCH".to_owned(),
                },
            );
            guard.ports = vec![
                Port {
                    node_id: 39,
                    index: 3,
                    name: "analog-output-lineout".to_owned(),
                    description: "Line Out".to_owned(),
                    available: true,
                    active: true,
                },
                Port {
                    node_id: 39,
                    index: 4,
                    name: "analog-output-headphones".to_owned(),
                    description: "Headphones".to_owned(),
                    available: true,
                    active: false,
                },
            ];
        }
        Daemon::new(
            state,
            PwHandle::disconnected(),
            Config::default(),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
        )
    }

    /// SPEC §8.1: `AutoMute a(ub)`, one row per sink whose card *has* the
    /// control. Cards we probed and found nothing on contribute no row.
    #[test]
    fn auto_mute_rows_only_cover_cards_with_the_control() {
        let cards = BTreeMap::from([
            (
                39,
                AlsaCard {
                    index: 1,
                    name: "HDA Intel PCH".to_owned(),
                },
            ),
            (
                43,
                AlsaCard {
                    index: 2,
                    name: "HDA NVidia".to_owned(),
                },
            ),
            (
                47,
                AlsaCard {
                    index: 3,
                    name: "USB Audio".to_owned(),
                },
            ),
        ]);
        // Card 1 has it and it is on, card 2 has no such control, card 3 has
        // not been probed yet.
        let cache = AutoMuteCache::from([(1, Some(true)), (2, None)]);
        assert_eq!(Daemon::auto_mute_rows(&cards, &cache), vec![(39, true)]);

        let cache = AutoMuteCache::from([(1, Some(false)), (2, None), (3, Some(true))]);
        assert_eq!(
            Daemon::auto_mute_rows(&cards, &cache),
            vec![(39, false), (47, true)]
        );
        assert!(Daemon::auto_mute_rows(&cards, &AutoMuteCache::new()).is_empty());
        assert!(Daemon::auto_mute_rows(&BTreeMap::new(), &cache).is_empty());
    }

    /// Two sinks on one card collapse to one probe: the mixer control is per
    /// card, not per node.
    #[test]
    fn snapshot_cards_dedupe_by_card_index() {
        let state = State {
            cards: BTreeMap::from([
                (
                    39,
                    AlsaCard {
                        index: 1,
                        name: "HDA Intel PCH".to_owned(),
                    },
                ),
                (
                    40,
                    AlsaCard {
                        index: 1,
                        name: "HDA Intel PCH".to_owned(),
                    },
                ),
                (
                    43,
                    AlsaCard {
                        index: 2,
                        name: "HDA NVidia".to_owned(),
                    },
                ),
            ]),
            ..State::default()
        };
        assert_eq!(
            Daemon::snapshot_cards(&state),
            BTreeMap::from([
                (1, "HDA Intel PCH".to_owned()),
                (2, "HDA NVidia".to_owned())
            ])
        );
        assert!(Daemon::snapshot_cards(&State::default()).is_empty());
    }

    /// SPEC §8.1: NotFound for an unknown node, InvalidArgument for an input
    /// or for a node with no ALSA card behind it.
    #[test]
    fn set_auto_mute_validates_before_touching_the_mixer() {
        let daemon = auto_mute_daemon();

        assert!(matches!(
            daemon.validate_set_auto_mute(99),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            daemon.validate_set_auto_mute(41),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            daemon.validate_set_auto_mute(43),
            Err(Error::InvalidArgument(_))
        ));

        let card = daemon.validate_set_auto_mute(39).expect("valid");
        assert_eq!(card.index, 1);
        assert_eq!(card.name, "HDA Intel PCH");
    }

    /// Validation runs before anything blocking, so these fail even with no
    /// graph and no mixer — the same guarantee `SetEq` gives.
    #[tokio::test]
    async fn set_auto_mute_fails_cleanly_without_a_mixer() {
        let daemon = auto_mute_daemon();
        assert!(matches!(
            daemon.do_set_auto_mute(99, false).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            daemon.do_set_auto_mute(41, false).await,
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            daemon.do_set_auto_mute(43, false).await,
            Err(Error::InvalidArgument(_))
        ));
    }

    /// With no cards in the snapshot there is nothing to probe, so a sync is a
    /// no-op that reports no change — the path every non-ALSA setup takes.
    #[tokio::test]
    async fn syncing_an_empty_graph_probes_nothing() {
        let daemon = daemon();
        assert!(!daemon.sync_auto_mute(false).await);
        assert!(!daemon.sync_auto_mute(true).await);
        assert!(daemon.auto_mute().await.is_empty());
    }

    /// The `AutoMute` property is the projection of the cache over the
    /// snapshot's cards, so a card whose sink is gone stops being reported.
    #[tokio::test]
    async fn auto_mute_property_follows_the_cache_and_the_snapshot() {
        let daemon = auto_mute_daemon();
        {
            let mut cache = daemon.auto_mute_cache.lock().await;
            cache.insert(1, Some(true));
        }
        assert_eq!(daemon.auto_mute().await, vec![(39, true)]);

        // The sink goes away: no card row, no `AutoMute` row.
        {
            let mut guard = daemon.state.write().expect("lock");
            guard.cards.clear();
        }
        assert_eq!(daemon.auto_mute().await, Vec::<AutoMuteTuple>::new());
    }

    /// SPEC §8.1: the automatic switch is driven off the *requested* route
    /// name, and reads the headphones jack out of the same snapshot.
    #[test]
    fn the_snapshot_supplies_both_inputs_of_the_automatic_switch() {
        use crate::alsa_mixer::{should_disable_auto_mute, AutoMutePolicy};

        let daemon = auto_mute_daemon();
        let snapshot = daemon.snapshot();

        let line_out = snapshot.port_name(39, 3).expect("line out");
        let headphones = snapshot.port_name(39, 4).expect("headphones");
        assert!(snapshot.headphones_available(39));

        assert!(should_disable_auto_mute(
            line_out,
            snapshot.headphones_available(39),
            true,
            AutoMutePolicy::Auto
        ));
        assert!(!should_disable_auto_mute(
            headphones,
            snapshot.headphones_available(39),
            true,
            AutoMutePolicy::Auto
        ));
    }

    /// The coalescing floor is what keeps `Changed` at or under 10/s.
    #[test]
    fn changed_is_coalesced_to_ten_per_second() {
        assert!(CHANGED_INTERVAL >= std::time::Duration::from_millis(100));
    }
}
