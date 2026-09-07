//! Everything that touches libpipewire.
//!
//! The rest of the crate is pure data so it can be unit-tested without a graph;
//! this module is the only place that binds registry globals, reads `Props`
//! params and writes WirePlumber metadata. It runs on its own `std::thread`
//! driving a libpipewire `MainLoop`, per SPEC §2.1 — nothing here is ever
//! called from the tokio side.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::CString;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use pipewire as pw;
use pw::spa;

use spa::param::{ParamInfo, ParamInfoFlags, ParamType};
use spa::pod::deserialize::PodDeserializer;
use spa::pod::serialize::PodSerializer;
use spa::pod::{ChoiceValue, Object, Pod, Property, Value, ValueArray};
use spa::utils::{Choice, ChoiceEnum, SpaTypes};

use pw::context::{ContextRc, ContextWeak};
use pw::device::{Device as PwDevice, DeviceChangeMask, DeviceListener};
use pw::main_loop::MainLoopRc;
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeChangeMask, NodeListener};
use pw::proxy::ProxyT;
use pw::registry::GlobalObject;
use pw::types::ObjectType;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::command::Command;
use crate::config::Config;
use crate::eq::{self, Preset};
use crate::error::{Error, Result};
use crate::matching::is_notification_stream;
use crate::meta;
use crate::route::{
    self, ActiveRoute, Availability, DeviceRoutes, PendingPortSwitch, PendingPortSwitches, Port,
    Route as CardRoute, RouteDirection, RouteProps,
};
use crate::state::{AlsaCard, Device, DeviceKind, State, Stream};
use crate::volume::{clamp_volume, linear_to_percent, port_switch_clamp};

/// Node property keys we read off registry globals.
mod keys {
    pub const MEDIA_CLASS: &str = "media.class";
    pub const NODE_NAME: &str = "node.name";
    pub const NODE_DESCRIPTION: &str = "node.description";
    pub const NODE_NICK: &str = "node.nick";
    pub const NODE_VIRTUAL: &str = "node.virtual";
    pub const APPLICATION_NAME: &str = "application.name";
    pub const APPLICATION_BINARY: &str = "application.process.binary";
    pub const MEDIA_NAME: &str = "media.name";
    pub const MEDIA_ROLE: &str = "media.role";
    pub const FACTORY_NAME: &str = "factory.name";
    pub const OBJECT_SERIAL: &str = "object.serial";
    pub const METADATA_NAME: &str = "metadata.name";
    pub const DEVICE_ID: &str = "device.id";
    pub const CARD_PROFILE_DEVICE: &str = "card.profile.device";
    pub const LINK_OUTPUT_NODE: &str = "link.output.node";
    pub const LINK_INPUT_NODE: &str = "link.input.node";
    pub const NODE_LINK_GROUP: &str = "node.link-group";
    pub const AUDIO_CHANNELS: &str = "audio.channels";
    pub const AUDIO_POSITION: &str = "audio.position";
    /// ALSA card index behind a sink (SPEC §8.1). Node `info` only.
    pub const ALSA_CARD: &str = "alsa.card";
    /// Card name keys, best first. Node `info` only, like `alsa.card`.
    pub const ALSA_CARD_NAMES: [&str; 4] = [
        "alsa.card_name",
        "api.alsa.card.name",
        "alsa.long_card_name",
        "api.alsa.card.longname",
    ];
}

/// Pull the ALSA card index and name out of a node's props (SPEC §8.1).
///
/// None of these keys are in the registry global's property whitelist — they
/// only arrive in the node `info` event, the same trap the EQ node detection
/// fell into on 2026-09-02.
fn alsa_card_from(props: &spa::utils::dict::DictRef) -> Option<AlsaCard> {
    let index: u32 = props.get(keys::ALSA_CARD)?.trim().parse().ok()?;
    let name = keys::ALSA_CARD_NAMES
        .iter()
        .find_map(|key| props.get(key).map(str::trim).filter(|s| !s.is_empty()))
        .map_or_else(|| crate::alsa_mixer::card_device(index), str::to_owned);
    Some(AlsaCard { index, name })
}

/// `media.class` of the card objects that own routes (SPEC §6.1).
const MEDIA_CLASS_AUDIO_DEVICE: &str = "Audio/Device";

/// What kind of audio node a global turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeRole {
    Sink,
    Source,
    PlaybackStream,
    CaptureStream,
}

impl NodeRole {
    fn from_media_class(media_class: &str) -> Option<Self> {
        // Virtual sinks report `Audio/Sink/Virtual`; treat any `Audio/Sink*`
        // as a sink so filter chains and null sinks are selectable (SPEC §2.1).
        if media_class.starts_with("Audio/Sink") {
            Some(NodeRole::Sink)
        } else if media_class.starts_with("Audio/Source") {
            Some(NodeRole::Source)
        } else if media_class == "Stream/Output/Audio" {
            Some(NodeRole::PlaybackStream)
        } else if media_class == "Stream/Input/Audio" {
            Some(NodeRole::CaptureStream)
        } else {
            None
        }
    }

    fn device_kind(self) -> Option<DeviceKind> {
        match self {
            NodeRole::Sink => Some(DeviceKind::Sink),
            NodeRole::Source => Some(DeviceKind::Source),
            _ => None,
        }
    }

    fn is_stream(self) -> bool {
        matches!(self, NodeRole::PlaybackStream | NodeRole::CaptureStream)
    }
}

/// A bound node plus the last values we saw for it.
struct NodeEntry {
    // Declared before the listener so the proxy is destroyed first, matching
    // upstream's `pw-mon` teardown order.
    proxy: Node,
    _listener: NodeListener,
    role: NodeRole,
    name: String,
    description: String,
    nick: String,
    /// `device.id` — the `Audio/Device` global this node belongs to.
    device_id: Option<u32>,
    /// `card.profile.device` — which profile-device of that card it is.
    card_profile_device: Option<i32>,
    app_name: String,
    binary: String,
    media_name: String,
    media_role: String,
    virtual_: bool,
    /// One of our own filter-chain nodes (SPEC §7.1): tracked, never published.
    hidden: bool,
    serial: Option<u64>,
    channels: usize,
    /// `audio.channels`, when the node advertises it in its props.
    audio_channels: Option<usize>,
    /// `audio.position`, split into channel names.
    audio_position: Vec<String>,
    /// The ALSA card behind this node, from `alsa.card` + a card-name key
    /// (SPEC §8.1). Both only ever arrive in the node `info` event.
    alsa_card: Option<AlsaCard>,
    volume: f64,
    mute: bool,
}

/// A bound `Audio/Device` global plus the routes it has told us about.
struct DeviceEntry {
    // Same declaration order rule as `NodeEntry`: proxy before listener.
    proxy: PwDevice,
    _listener: DeviceListener,
    routes: DeviceRoutes,
}

/// Where a routed node's volume/mute has to be written (SPEC §6.1).
struct RouteTarget {
    device_id: u32,
    card_profile_device: i32,
    index: u32,
    props: RouteProps,
}

/// An owned `libpipewire-module-filter-chain` instance.
///
/// The pointer is only ever touched on the PipeWire thread, which is also the
/// only thread that can construct one — [`Inner`] is `!Send` and never leaves it.
struct ModuleHandle(*mut pw::sys::pw_impl_module);

impl ModuleHandle {
    /// Unload the module. Idempotent: the pointer is nulled as it goes.
    fn destroy(&mut self) {
        if self.0.is_null() {
            return;
        }
        // SAFETY: the pointer came from `pw_context_load_module` on this
        // thread's context, has not been destroyed yet (it is nulled below),
        // and we are on the PipeWire thread.
        unsafe { pw::sys::pw_impl_module_destroy(self.0) };
        self.0 = std::ptr::null_mut();
    }
}

impl Drop for ModuleHandle {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// One EQ filter chain, per target sink (SPEC §7.1).
struct EqInstance {
    /// The loaded module. Held for ownership only — dropping the instance is
    /// what unloads it, exactly like `NodeEntry`'s `_listener`.
    _module: ModuleHandle,
    /// `node.name` of the sink the chain filters.
    sink_name: String,
    /// `node.name` of the chain's main (capture) node.
    node_name: String,
    /// Node id of that main node, once its global appears.
    main_node_id: Option<u32>,
    /// The preset whose controls are currently written into the chain.
    applied_preset: Option<Preset>,
    /// True while bypassed through the `filters` metadata (SPEC §7.1's "EQ
    /// off": the module stays loaded, WirePlumber just stops routing through it).
    disabled: bool,
}

/// The bound `default` metadata object.
struct MetadataEntry {
    proxy: Metadata,
    _listener: MetadataListener,
}

/// Everything the PipeWire thread owns, behind one `RefCell`.
struct Inner {
    state: Arc<RwLock<State>>,
    notify: watch::Sender<u64>,
    config: Config,
    /// The preset library, rescanned by the D-Bus side (SPEC §7.3).
    presets: Arc<RwLock<Vec<Preset>>>,
    /// Our own context, for `pw_context_load_module`. Weak so the listener
    /// closures that hold [`Inner`] cannot keep the context alive by themselves.
    context: ContextWeak,
    /// Loaded filter chains by target sink `node.name`.
    eq: HashMap<String, EqInstance>,
    /// `(sink, preset id)` pairs already reported as unknown, so the warning is
    /// logged once rather than on every graph event.
    eq_warned: HashSet<(String, String)>,
    /// The WirePlumber `filters` metadata, when it exists.
    filters_metadata: Option<Rc<MetadataEntry>>,
    nodes: HashMap<u32, NodeEntry>,
    /// `Audio/Device` globals by id, holding their route tables.
    devices: HashMap<u32, DeviceEntry>,
    /// Link id -> (output node id, input node id), used to resolve stream targets.
    links: HashMap<u32, (u32, u32)>,
    /// Raw `target.object` metadata values by subject node id.
    targets: HashMap<u32, String>,
    metadata: Option<Rc<MetadataEntry>>,
    /// Stream ids we have pointed at the notification sink, so we can undo it.
    routed: HashSet<u32>,
    /// SPEC §9.2: port switches PipeDeck has issued and not yet seen take
    /// effect, each carrying the level cap to apply when it does.
    pending_port_caps: PendingPortSwitches,
    default_sink: Option<String>,
    default_source: Option<String>,
    revision: u64,
}

impl Inner {
    fn resolve_target(&self, stream_id: u32) -> String {
        // The live link is the truth; the metadata value is the intent.
        for (out_node, in_node) in self.links.values() {
            if *out_node == stream_id {
                if let Some(entry) = self.nodes.get(in_node) {
                    return entry.name.clone();
                }
            }
        }
        let Some(raw) = self.targets.get(&stream_id) else {
            return String::new();
        };
        if let Ok(serial) = raw.parse::<u64>() {
            if let Some(entry) = self.nodes.values().find(|n| n.serial == Some(serial)) {
                return entry.name.clone();
            }
        }
        raw.clone()
    }

    /// The card, profile-device and route table behind a node, when it has one.
    ///
    /// A node is "routed" when it carries both `device.id` and
    /// `card.profile.device` and that device global is bound (SPEC §6.1).
    fn node_device(&self, entry: &NodeEntry) -> Option<(u32, i32, &DeviceRoutes)> {
        let device_id = entry.device_id?;
        let card_profile_device = entry.card_profile_device?;
        let device = self.devices.get(&device_id)?;
        Some((device_id, card_profile_device, &device.routes))
    }

    /// Where a node's volume/mute must be written: `Some` only when the card
    /// has an *active* route for it, which is exactly when the node's own
    /// `Props` writes are ignored.
    fn route_target(&self, entry: &NodeEntry) -> Option<RouteTarget> {
        let (device_id, card_profile_device, routes) = self.node_device(entry)?;
        let active = routes.active_for(card_profile_device)?;
        Some(RouteTarget {
            device_id,
            card_profile_device,
            index: active.index,
            props: active.props.clone(),
        })
    }

    /// Is there a real (non-EQ) sink by this name?
    ///
    /// SPEC §7.1: the daemon's own filter-chain nodes are never selectable, so
    /// they can never become a stream target or the notification sink.
    fn sink_exists(&self, name: &str) -> bool {
        self.nodes
            .values()
            .any(|n| n.role == NodeRole::Sink && !n.hidden && n.name == name)
    }

    fn is_notification(&self, entry: &NodeEntry) -> bool {
        entry.role == NodeRole::PlaybackStream
            && is_notification_stream(
                Some(entry.media_role.as_str()),
                Some(entry.app_name.as_str()),
                &self.config.notification_apps,
            )
    }

    /// Build a fresh snapshot and hand it to the D-Bus side.
    fn publish(&mut self) {
        let mut state = State {
            default_sink: self.default_sink.clone(),
            default_source: self.default_source.clone(),
            notification_sink: self.config.notification_sink.clone(),
            connected: true,
            ..State::default()
        };

        let mut ports: Vec<Port> = Vec::new();
        let mut eq_rows: Vec<(u32, String)> = Vec::new();
        let mut cards: BTreeMap<u32, AlsaCard> = BTreeMap::new();

        for (id, entry) in &self.nodes {
            // SPEC §7.1: our own filter-chain nodes are tracked (we write their
            // controls) but never published as devices or streams.
            if entry.hidden {
                continue;
            }
            if let Some(kind) = entry.role.device_kind() {
                // On ALSA cards WirePlumber owns volume through the device
                // `Route` param, so that is what the panel must be shown
                // (SPEC §6.1); nodes without a route keep their own `Props`.
                let mut volume = entry.volume;
                let mut mute = entry.mute;
                if let Some((_, card_profile_device, routes)) = self.node_device(entry) {
                    ports.extend(routes.ports_for(*id, kind, card_profile_device));
                    // SPEC §8.1: only port-capable *sinks* get a card row —
                    // auto-mute is an output-side control, and a node with no
                    // routes has no port for the policy to react to.
                    if kind == DeviceKind::Sink {
                        if let Some(card) = entry.alsa_card.clone() {
                            cards.insert(*id, card);
                        }
                    }
                    if let Some(active) = routes.active_for(card_profile_device) {
                        if let Some(v) = active.props.volume {
                            volume = v;
                        }
                        if let Some(m) = active.props.mute {
                            mute = m;
                        }
                    }
                }
                if kind == DeviceKind::Sink {
                    eq_rows.push((*id, self.eq_selection(&entry.name)));
                }
                state.devices.insert(
                    *id,
                    Device {
                        id: *id,
                        name: entry.name.clone(),
                        description: entry.description.clone(),
                        kind,
                        is_default: false,
                        virtual_: entry.virtual_,
                        volume,
                        mute,
                        nick: entry.nick.clone(),
                    },
                );
            } else if entry.role.is_stream() {
                state.streams.insert(
                    *id,
                    Stream {
                        id: *id,
                        app_name: entry.app_name.clone(),
                        binary: entry.binary.clone(),
                        media_name: entry.media_name.clone(),
                        role: entry.media_role.clone(),
                        target_name: self.resolve_target(*id),
                        volume: entry.volume,
                        mute: entry.mute,
                        playback: entry.role == NodeRole::PlaybackStream,
                    },
                );
            }
        }
        ports.sort_by_key(|p| (p.node_id, p.index));
        state.ports = ports;
        state.cards = cards;
        eq_rows.sort_by_key(|(id, _)| *id);
        state.eq = eq_rows;
        state.refresh_defaults();

        match self.state.write() {
            Ok(mut guard) => *guard = state,
            Err(poisoned) => {
                warn!("state lock was poisoned; recovering");
                *poisoned.into_inner() = state;
            }
        }

        self.revision = self.revision.wrapping_add(1);
        let _ = self.notify.send(self.revision);
    }

    // -----------------------------------------------------------------
    // EQ (SPEC §7.1)
    // -----------------------------------------------------------------

    /// The preset id to report for a sink in the `Eq` property.
    ///
    /// This is the *configured* selection, resolved against the preset library:
    /// an id that no longer names a preset reads as "off" (SPEC §7.1: "unknown
    /// preset name → log warn, treat as off"). Reporting the configuration
    /// rather than the instance's applied state keeps the row stable while a
    /// freshly loaded chain waits for its node to appear.
    fn eq_selection(&self, sink_name: &str) -> String {
        let Some(wanted) = self.config.eq_preset(sink_name) else {
            return String::new();
        };
        if self.preset_by_id(wanted).is_some() {
            wanted.to_owned()
        } else {
            String::new()
        }
    }

    /// Look one preset up in the shared library.
    fn preset_by_id(&self, id: &str) -> Option<Preset> {
        let guard = match self.presets.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.iter().find(|p| p.id == id).cloned()
    }

    /// Everything [`build_filter_chain_args`](crate::eq::build_filter_chain_args)
    /// needs about a target sink.
    fn eq_target(&self, sink_name: &str) -> Option<(String, usize, Vec<String>)> {
        let entry = self
            .nodes
            .values()
            .find(|n| n.role == NodeRole::Sink && !n.hidden && n.name == sink_name)?;
        let channels = entry
            .audio_channels
            .filter(|c| *c > 0)
            .unwrap_or(entry.channels)
            .max(1);
        let position = if entry.audio_position.len() == channels {
            entry.audio_position.clone()
        } else {
            Vec::new()
        };
        Some((entry.nick.clone(), channels, position))
    }

    /// Adopt a newly appeared filter-chain main node, if it is one of ours.
    fn attach_eq_node(&mut self, id: u32) {
        let Some(entry) = self.nodes.get(&id) else {
            return;
        };
        if !entry.hidden || entry.role != NodeRole::Sink {
            return;
        }
        let name = entry.name.clone();
        if let Some(instance) = self.eq.values_mut().find(|i| i.node_name == name) {
            if instance.main_node_id != Some(id) {
                debug!(node = id, sink = %instance.sink_name, "EQ filter chain node appeared");
                instance.main_node_id = Some(id);
                instance.applied_preset = None;
            }
        }
    }

    /// Forget a filter-chain main node that has gone away.
    fn detach_eq_node(&mut self, id: u32) {
        for instance in self.eq.values_mut() {
            if instance.main_node_id == Some(id) {
                instance.main_node_id = None;
                instance.applied_preset = None;
            }
        }
    }

    /// Reconcile the loaded filter chains with the `[eq]` config (SPEC §7.1).
    ///
    /// Idempotent, and safe to call on every graph change: it loads a chain for
    /// a sink that has gained a preset, bypasses one that has lost it, pushes
    /// controls when the preset (or the chain's node) changed, and unloads a
    /// chain whose sink has disappeared.
    fn apply_eq(&mut self) {
        let live: HashSet<String> = self
            .nodes
            .values()
            .filter(|n| n.role == NodeRole::Sink && !n.hidden)
            .map(|n| n.name.clone())
            .collect();

        // A sink that went away takes its chain with it.
        let stale: Vec<String> = self
            .eq
            .keys()
            .filter(|name| !live.contains(*name))
            .cloned()
            .collect();
        for name in stale {
            info!(sink = %name, "sink gone; unloading its EQ filter chain");
            self.eq.remove(&name);
        }

        let mut sinks: Vec<String> = live.into_iter().collect();
        sinks.sort();
        for sink in sinks {
            let configured = self.config.eq_preset(&sink).map(str::to_owned);
            let wanted = configured.as_ref().and_then(|id| {
                let found = self.preset_by_id(id);
                if found.is_some() {
                    self.eq_warned.remove(&(sink.clone(), id.clone()));
                } else if self.eq_warned.insert((sink.clone(), id.clone())) {
                    // SPEC §7.1: unknown preset name -> log warn, treat as off.
                    warn!(sink = %sink, preset = %id, "unknown EQ preset; treating as off");
                }
                found
            });

            match wanted {
                Some(preset) => self.eq_enable(&sink, &preset),
                None => self.eq_disable(&sink),
            }
        }
    }

    /// Make sure `sink` has a live chain running `preset`.
    fn eq_enable(&mut self, sink: &str, preset: &Preset) {
        if !self.eq.contains_key(sink) {
            let Some((nick, channels, position)) = self.eq_target(sink) else {
                return;
            };
            let Some(context) = self.context.upgrade() else {
                warn!(sink = %sink, "PipeWire context is gone; cannot load the EQ filter chain");
                return;
            };
            let args = eq::build_filter_chain_args(sink, &nick, channels, &position);
            debug!(sink = %sink, channels, "loading the EQ filter chain");
            match load_filter_chain(&context, &args) {
                Some(module) => {
                    info!(sink = %sink, preset = %preset.id, "EQ filter chain loaded");
                    self.eq.insert(
                        sink.to_owned(),
                        EqInstance {
                            _module: module,
                            sink_name: sink.to_owned(),
                            node_name: eq::eq_node_name(sink),
                            main_node_id: None,
                            applied_preset: None,
                            disabled: false,
                        },
                    );
                }
                None => {
                    error!(sink = %sink, "could not load {}", eq::FILTER_CHAIN_MODULE);
                    return;
                }
            }
        }

        // Un-bypass before writing controls, so a re-enable is one step.
        let needs_enable = self.eq.get(sink).is_some_and(|i| i.disabled);
        if needs_enable && self.set_filter_disabled(sink, false) {
            if let Some(instance) = self.eq.get_mut(sink) {
                instance.disabled = false;
            }
        }

        let (node_id, stale) = match self.eq.get(sink) {
            Some(instance) => (
                instance.main_node_id,
                instance.applied_preset.as_ref() != Some(preset),
            ),
            None => return,
        };
        if let (Some(node_id), true) = (node_id, stale) {
            if self.write_eq_params(node_id, preset) {
                if let Some(instance) = self.eq.get_mut(sink) {
                    instance.applied_preset = Some(preset.clone());
                }
            }
        }
    }

    /// Bypass `sink`'s chain, if it has one.
    ///
    /// SPEC §7.1 prefers the `filters` metadata (instant re-link, no reload).
    /// When that metadata does not exist we fall back to unloading the module,
    /// and say so — the next `SetEq` reloads it.
    fn eq_disable(&mut self, sink: &str) {
        let Some(instance) = self.eq.get(sink) else {
            return;
        };
        if instance.disabled {
            return;
        }
        if instance.main_node_id.is_none() {
            // Nothing is routed through it yet, so there is nothing to bypass.
            debug!(sink = %sink, "unloading an EQ filter chain that never came up");
            self.eq.remove(sink);
            return;
        }
        if self.set_filter_disabled(sink, true) {
            if let Some(instance) = self.eq.get_mut(sink) {
                instance.disabled = true;
                instance.applied_preset = None;
            }
        } else {
            warn!(
                sink = %sink,
                "the WirePlumber `{}` metadata is missing; unloading the EQ filter chain instead",
                eq::METADATA_NAME_FILTERS
            );
            self.eq.remove(sink);
        }
    }

    /// Write `filter.smart.disabled` for a chain. `false` when there is no
    /// `filters` metadata or no main node to key it on yet.
    fn set_filter_disabled(&self, sink: &str, disabled: bool) -> bool {
        let Some(instance) = self.eq.get(sink) else {
            return false;
        };
        let Some(node_id) = instance.main_node_id else {
            return false;
        };
        let Some(metadata) = self.filters_metadata.as_ref() else {
            return false;
        };
        debug!(sink = %sink, node = node_id, disabled, "setting filter.smart.disabled");
        metadata.proxy.set_property(
            node_id,
            eq::KEY_FILTER_SMART_DISABLED,
            Some(meta::TYPE_SPA_JSON),
            Some(if disabled { "true" } else { "false" }),
        );
        true
    }

    /// Push a preset's control values into a chain's main node.
    fn write_eq_params(&self, node_id: u32, preset: &Preset) -> bool {
        let Some(entry) = self.nodes.get(&node_id) else {
            return false;
        };
        let params = eq::preset_to_params(preset);
        let Some(bytes) = eq_params_pod(&params) else {
            error!(preset = %preset.id, "could not build the EQ params pod");
            return false;
        };
        let Some(pod) = Pod::from_bytes(&bytes) else {
            error!(preset = %preset.id, "built an invalid EQ params pod");
            return false;
        };
        debug!(node = node_id, preset = %preset.id, controls = params.len(), "applying EQ preset");
        entry.proxy.set_param(ParamType::Props, 0, pod);
        true
    }

    /// Unload every chain. Called once the main loop has stopped.
    fn teardown_eq(&mut self) {
        if !self.eq.is_empty() {
            info!(chains = self.eq.len(), "unloading EQ filter chains");
        }
        self.eq.clear();
    }

    /// Point every matching notification stream at the configured sink, and
    /// release any stream we routed before that no longer qualifies.
    fn apply_notification_routing(&mut self) {
        let Some(metadata) = self.metadata.clone() else {
            return;
        };
        let target = self.config.notification_sink.clone();
        let target_present = !target.is_empty() && self.sink_exists(&target);

        if !target.is_empty() && !target_present {
            // SPEC §2.1: leave the stream alone and re-apply when it appears.
            debug!(sink = %target, "notification sink absent; leaving streams alone");
        }

        let mut to_route: Vec<u32> = Vec::new();
        let mut to_release: Vec<u32> = Vec::new();

        for (id, entry) in &self.nodes {
            if !entry.role.is_stream() {
                continue;
            }
            let matches = self.is_notification(entry);
            if matches && target_present {
                if self.targets.get(id).map(String::as_str) != Some(target.as_str()) {
                    to_route.push(*id);
                }
            } else if self.routed.contains(id) {
                to_release.push(*id);
            }
        }

        for id in to_route {
            debug!(stream = id, sink = %target, "routing notification stream");
            metadata.proxy.set_property(
                id,
                meta::KEY_TARGET_OBJECT,
                Some(meta::TYPE_SPA_STRING),
                Some(&target),
            );
            self.targets.insert(id, target.clone());
            self.routed.insert(id);
        }
        for id in to_release {
            debug!(
                stream = id,
                "releasing notification stream back to the default"
            );
            metadata
                .proxy
                .set_property(id, meta::KEY_TARGET_OBJECT, None, None);
            self.targets.remove(&id);
            self.routed.remove(&id);
        }
    }
}

/// A cheap, cloneable handle used by the D-Bus side to post commands.
///
/// Holding only the channel (not the thread) means the D-Bus layer can be
/// constructed — and introspected — without a running graph.
#[derive(Clone)]
pub struct PwHandle {
    /// `None` means "no PipeWire thread", which every command reports as
    /// `dev.pipedeck.Error.PipeWire` instead of blocking.
    sender: Option<pw::channel::Sender<Command>>,
}

impl PwHandle {
    /// Queue a command for the PipeWire loop.
    ///
    /// # Errors
    /// `dev.pipedeck.Error.PipeWire` when the loop is gone.
    pub fn send(&self, command: Command) -> Result<()> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| Error::pipewire("PipeWire thread is not running"))?;
        sender
            .send(command)
            .map_err(|_| Error::pipewire("PipeWire thread is not running"))
    }

    /// A handle attached to nothing, for tests that only need the D-Bus surface.
    #[must_use]
    pub fn disconnected() -> Self {
        Self { sender: None }
    }
}

impl std::fmt::Debug for PwHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PwHandle")
    }
}

/// Owner of the PipeWire thread.
pub struct PwThread {
    sender: pw::channel::Sender<Command>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl PwThread {
    /// Start the PipeWire thread and wait for it to connect.
    ///
    /// # Errors
    /// Returns the connection failure message if libpipewire could not reach a
    /// running server.
    pub fn spawn(
        config: Config,
        state: Arc<RwLock<State>>,
        presets: Arc<RwLock<Vec<Preset>>>,
        notify: watch::Sender<u64>,
        exited: UnboundedSender<()>,
    ) -> std::result::Result<Self, String> {
        let (sender, receiver) = pw::channel::channel::<Command>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();

        let handle = std::thread::Builder::new()
            .name("pipedeck-pw".to_owned())
            .spawn(move || {
                let result = run(config, state, presets, notify, receiver, &ready_tx);
                // If we failed before signalling readiness, report it now.
                let _ = ready_tx.send(result.clone());
                if let Err(e) = result {
                    error!("PipeWire thread stopped: {e}");
                }
                let _ = exited.send(());
            })
            .map_err(|e| format!("could not spawn PipeWire thread: {e}"))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                sender,
                handle: Some(handle),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("PipeWire thread exited before signalling readiness".to_owned()),
        }
    }

    /// A cloneable command handle for the D-Bus side.
    #[must_use]
    pub fn handle(&self) -> PwHandle {
        PwHandle {
            sender: Some(self.sender.clone()),
        }
    }

    /// Ask the loop to quit and wait for the thread.
    pub fn shutdown(&mut self) {
        let _ = self.sender.send(Command::Terminate);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for PwThread {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The PipeWire thread body.
fn run(
    config: Config,
    state: Arc<RwLock<State>>,
    presets: Arc<RwLock<Vec<Preset>>>,
    notify: watch::Sender<u64>,
    receiver: pw::channel::Receiver<Command>,
    ready: &std::sync::mpsc::Sender<std::result::Result<(), String>>,
) -> std::result::Result<(), String> {
    pw::init();

    let main_loop = MainLoopRc::new(None).map_err(|e| format!("main loop: {e}"))?;
    let context = ContextRc::new(&main_loop, None).map_err(|e| format!("context: {e}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|e| format!("could not connect to PipeWire: {e}"))?;
    let registry = core
        .get_registry_rc()
        .map_err(|e| format!("registry: {e}"))?;

    let inner = Rc::new(RefCell::new(Inner {
        state,
        notify,
        config,
        presets,
        context: context.downgrade(),
        eq: HashMap::new(),
        eq_warned: HashSet::new(),
        filters_metadata: None,
        nodes: HashMap::new(),
        devices: HashMap::new(),
        links: HashMap::new(),
        targets: HashMap::new(),
        metadata: None,
        routed: HashSet::new(),
        pending_port_caps: PendingPortSwitches::default(),
        default_sink: None,
        default_source: None,
        revision: 0,
    }));

    let loop_weak = main_loop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, seq, res, message| {
            error!(id, seq, res, "PipeWire error: {message}");
            if id == 0 {
                if let Some(l) = loop_weak.upgrade() {
                    l.quit();
                }
            }
        })
        .register();

    let registry_weak = registry.downgrade();
    let global_inner = inner.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let Some(registry) = registry_weak.upgrade() else {
                return;
            };
            on_global(&global_inner, &registry, global);
        })
        .global_remove({
            let inner = inner.clone();
            move |id| on_global_remove(&inner, id)
        })
        .register();

    let loop_weak = main_loop.downgrade();
    let command_inner = inner.clone();
    let _receiver = receiver.attach(main_loop.loop_(), move |command| {
        if matches!(command, Command::Terminate) {
            if let Some(l) = loop_weak.upgrade() {
                l.quit();
            }
            return;
        }
        handle_command(&command_inner, command);
    });

    info!("connected to PipeWire");
    let _ = ready.send(Ok(()));

    main_loop.run();

    // SPEC §7.1: unload the filter chains on shutdown, while the context and
    // the loop they were created on are both still alive.
    inner.borrow_mut().teardown_eq();

    // Mark the snapshot stale so clients can tell the graph link is gone.
    if let Ok(mut guard) = inner.borrow().state.write() {
        guard.connected = false;
    }
    Ok(())
}

/// A new global appeared on the registry.
fn on_global(
    inner: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&spa::utils::dict::DictRef>,
) {
    match global.type_ {
        ObjectType::Node => on_node_global(inner, registry, global),
        ObjectType::Device => on_device_global(inner, registry, global),
        ObjectType::Metadata => on_metadata_global(inner, registry, global),
        ObjectType::Link => on_link_global(inner, global),
        _ => {}
    }
}

fn on_node_global(
    inner: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&spa::utils::dict::DictRef>,
) {
    let Some(props) = global.props else { return };
    let Some(media_class) = props.get(keys::MEDIA_CLASS) else {
        return;
    };
    let Some(role) = NodeRole::from_media_class(media_class) else {
        return;
    };

    let node: Node = match registry.bind(global) {
        Ok(node) => node,
        Err(e) => {
            warn!(id = global.id, "could not bind node: {e}");
            return;
        }
    };

    let id = global.id;
    let name = props.get(keys::NODE_NAME).unwrap_or_default().to_owned();
    let description = props
        .get(keys::NODE_DESCRIPTION)
        .or_else(|| props.get(keys::NODE_NICK))
        .filter(|s| !s.is_empty())
        .unwrap_or(&name)
        .to_owned();
    let nick = props
        .get(keys::NODE_NICK)
        .filter(|s| !s.is_empty())
        .unwrap_or(description.as_str())
        .to_owned();
    let virtual_ = props.get(keys::NODE_VIRTUAL) == Some("true")
        || media_class.ends_with("/Virtual")
        || props
            .get(keys::FACTORY_NAME)
            .is_some_and(|f| f.contains("null-audio-sink"));
    // SPEC §7.1: our own filter-chain nodes are hidden from Devices/Streams.
    let hidden = eq::is_eq_node(
        props.get(eq::PROP_PIPEDECK_EQ),
        props.get(keys::NODE_LINK_GROUP),
    );

    let listener = {
        let inner = inner.clone();
        node.add_listener_local()
            .info({
                let inner = inner.clone();
                move |info| {
                    if let Some(props) = info.props() {
                        on_node_info(&inner, id, props);
                    }
                    if info.change_mask().contains(NodeChangeMask::PARAMS) {
                        let reported = reported_params(info.params());
                        reenumerate_node_params(&inner, id, &reported);
                    }
                }
            })
            .param(move |_seq, param_type, _index, _next, param| {
                if param_type != ParamType::Props {
                    return;
                }
                let Some(param) = param else { return };
                on_node_props(&inner, id, param);
            })
            .register()
    };

    node.subscribe_params(&[ParamType::Props]);
    node.enum_params(0, Some(ParamType::Props), 0, u32::MAX);

    let entry = NodeEntry {
        proxy: node,
        _listener: listener,
        role,
        name,
        description,
        nick,
        device_id: props.get(keys::DEVICE_ID).and_then(|s| s.parse().ok()),
        card_profile_device: props
            .get(keys::CARD_PROFILE_DEVICE)
            .and_then(|s| s.parse().ok()),
        app_name: props
            .get(keys::APPLICATION_NAME)
            .unwrap_or_default()
            .to_owned(),
        binary: props
            .get(keys::APPLICATION_BINARY)
            .unwrap_or_default()
            .to_owned(),
        media_name: props.get(keys::MEDIA_NAME).unwrap_or_default().to_owned(),
        media_role: props.get(keys::MEDIA_ROLE).unwrap_or_default().to_owned(),
        virtual_,
        hidden,
        serial: props.get(keys::OBJECT_SERIAL).and_then(|s| s.parse().ok()),
        channels: 2,
        audio_channels: props.get(keys::AUDIO_CHANNELS).and_then(|s| s.parse().ok()),
        audio_position: props
            .get(keys::AUDIO_POSITION)
            .map(eq::parse_positions)
            .unwrap_or_default(),
        // Almost certainly `None` here — see `alsa_card_from`; `on_node_info`
        // is where this actually gets filled in.
        alsa_card: alsa_card_from(props),
        volume: 1.0,
        mute: false,
    };

    let mut guard = inner.borrow_mut();
    guard.nodes.insert(id, entry);
    guard.attach_eq_node(id);
    guard.apply_notification_routing();
    guard.apply_eq();
    guard.publish();
}

/// An `Audio/Device` global appeared: bind it and start tracking its routes.
///
/// SPEC §6.1 — this is the object that owns port switching *and*, on ALSA
/// cards, the volume the node's own `Props` param refuses to accept.
fn on_device_global(
    inner: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&spa::utils::dict::DictRef>,
) {
    let Some(props) = global.props else { return };
    if props.get(keys::MEDIA_CLASS) != Some(MEDIA_CLASS_AUDIO_DEVICE) {
        return;
    }

    let device: PwDevice = match registry.bind(global) {
        Ok(device) => device,
        Err(e) => {
            warn!(id = global.id, "could not bind audio device: {e}");
            return;
        }
    };

    let id = global.id;
    let listener = {
        let inner = inner.clone();
        device
            .add_listener_local()
            .info({
                let inner = inner.clone();
                move |info| {
                    if !info.change_mask().contains(DeviceChangeMask::PARAMS) {
                        return;
                    }
                    let reported = reported_params(info.params());
                    reenumerate_device_params(&inner, id, &reported);
                }
            })
            .param(move |_seq, param_type, index, _next, param| {
                let Some(param) = param else { return };
                on_device_param(&inner, id, param_type, index, param);
            })
            .register()
    };

    device.subscribe_params(&[ParamType::EnumRoute, ParamType::Route]);
    device.enum_params(0, Some(ParamType::EnumRoute), 0, u32::MAX);
    device.enum_params(0, Some(ParamType::Route), 0, u32::MAX);

    let mut guard = inner.borrow_mut();
    guard.devices.insert(
        id,
        DeviceEntry {
            proxy: device,
            _listener: listener,
            routes: DeviceRoutes::default(),
        },
    );
    drop(guard);
    debug!(id, "tracking an Audio/Device global");
}

/// Flatten an info event's param list into `(raw param id, is readable)`.
fn reported_params(params: &[ParamInfo]) -> Vec<(u32, bool)> {
    params
        .iter()
        .map(|p| (p.id().as_raw(), p.flags().contains(ParamInfoFlags::READ)))
        .collect()
}

/// Which of the params we track an `info` event tells us to re-read.
///
/// pipewire-pulse's `device_event_info` re-enumerates a param when its
/// `user`/serial counter moved; pipewire-rs 0.10's [`ParamInfo`] exposes only
/// `id` and `flags`, so we re-enumerate every param we track that the server
/// lists as READ-able. An info event with no param list at all falls back to
/// re-reading everything we track.
fn params_to_reenumerate(reported: &[(u32, bool)], wanted: &[ParamType]) -> Vec<ParamType> {
    if reported.is_empty() {
        return wanted.to_vec();
    }
    wanted
        .iter()
        .copied()
        .filter(|want| {
            reported
                .iter()
                .any(|(id, readable)| *readable && *id == want.as_raw())
        })
        .collect()
}

/// Re-read the route params a device's `info` event says have changed.
///
/// **`subscribe_params` does not deliver device param changes on PipeWire
/// 1.6.2** — verified live on chronos: the whole `EnumRoute` + `Route`
/// enumeration arrives twice at startup and then never again, while registry
/// events keep flowing. `wpctl` and pipewire-pulse do not rely on it either;
/// `module-protocol-pulse/manager.c`'s `device_event_info` reacts to the
/// **`info` event**, walking `info.params()` and re-enumerating the changed
/// READ-able ids itself. This is that, and it is what makes `Devices`/`Ports`
/// reflect a `SetPort` or a volume change instead of freezing at startup values.
/// The `subscribe_params` call is kept as well, in case a future server does
/// deliver them — a duplicate enumeration is idempotent.
fn reenumerate_device_params(inner: &Rc<RefCell<Inner>>, id: u32, reported: &[(u32, bool)]) {
    let wanted = params_to_reenumerate(reported, &[ParamType::EnumRoute, ParamType::Route]);
    if wanted.is_empty() {
        return;
    }
    let guard = inner.borrow();
    let Some(entry) = guard.devices.get(&id) else {
        return;
    };
    for param_type in wanted {
        debug!(id, ?param_type, "re-enumerating device param after info");
        entry.proxy.enum_params(0, Some(param_type), 0, u32::MAX);
    }
}

/// The node-side twin of [`reenumerate_device_params`], for `Props`.
///
/// Node `subscribe_params` *does* work on this server (v0.1 read stream and
/// null-sink volumes back live), and the node listener already had an `info`
/// hook — but it only read the props dict, never the param list. Gating on the
/// PARAMS change mask makes this free: a `media.name` change sets PROPS, not
/// PARAMS, so the constant churn from a music player does not trigger it.
fn reenumerate_node_params(inner: &Rc<RefCell<Inner>>, id: u32, reported: &[(u32, bool)]) {
    let wanted = params_to_reenumerate(reported, &[ParamType::Props]);
    if wanted.is_empty() {
        return;
    }
    let guard = inner.borrow();
    let Some(entry) = guard.nodes.get(&id) else {
        return;
    };
    for param_type in wanted {
        entry.proxy.enum_params(0, Some(param_type), 0, u32::MAX);
    }
}

/// An `EnumRoute` or `Route` param arrived for a bound device.
///
/// PipeWire re-emits a whole enumeration starting at index 0, so index 0 is the
/// signal to drop the previous table — that is what keeps the route list honest
/// across a card-profile change.
fn on_device_param(
    inner: &Rc<RefCell<Inner>>,
    id: u32,
    param_type: ParamType,
    index: u32,
    param: &Pod,
) {
    let is_enum = param_type == ParamType::EnumRoute;
    if !is_enum && param_type != ParamType::Route {
        return;
    }
    let Some(raw) = parse_route(param) else {
        debug!(
            id,
            ?param_type,
            index,
            "device param did not parse as a route"
        );
        return;
    };
    debug!(
        id,
        ?param_type,
        index,
        route = ?raw.index,
        device = ?raw.device,
        name = ?raw.name,
        "device route param"
    );

    // SPEC §9.2: `(route index, card.profile.device, props)` of an active-route
    // arrival, carried out of the borrow so the cap's own write can take one.
    let mut active_arrival: Option<(u32, i32, RouteProps)> = None;

    {
        let mut guard = inner.borrow_mut();
        let Some(entry) = guard.devices.get_mut(&id) else {
            return;
        };
        if is_enum {
            if index == 0 {
                entry.routes.enum_routes.clear();
            }
            if let (Some(route_index), Some(direction)) = (raw.index, raw.direction) {
                entry.routes.enum_routes.insert(
                    route_index,
                    CardRoute {
                        index: route_index,
                        direction,
                        name: raw.name,
                        description: raw.description,
                        priority: raw.priority,
                        available: raw.available,
                        devices: raw.devices,
                        profiles: raw.profiles,
                    },
                );
            }
        } else {
            if index == 0 {
                entry.routes.active.clear();
            }
            if let (Some(route_index), Some(device)) = (raw.index, raw.device) {
                let props = raw.props.unwrap_or_default();
                entry.routes.active.insert(
                    device,
                    ActiveRoute {
                        index: route_index,
                        device,
                        props: props.clone(),
                    },
                );
                active_arrival = Some((route_index, device, props));
            }
        }
    }

    if let Some((route_index, card_profile_device, props)) = active_arrival {
        apply_port_switch_cap(inner, id, route_index, card_profile_device, &props);
    }

    inner.borrow_mut().publish();
}

/// SPEC §9.2's port-switch level cap.
///
/// Runs on every active-`Route` arrival and does nothing unless that arrival
/// matches a switch **PipeDeck itself** asked for, still inside the window,
/// whose level is above the cap.
///
/// The pending entry is deliberately **not** consumed by the first clamp.
/// Measured on chronos 2026-09-06: WirePlumber's own per-port restore reacts to
/// the same `Route` change and writes the port's stored level *after* our
/// clamp, so a one-shot clamp was logged and then silently overwritten. Every
/// arrival inside the window is answered instead; the echo of our own clamp
/// comes back at the cap and `port_switch_clamp`'s epsilon makes that a no-op,
/// which is what stops the two of us ping-ponging.
fn apply_port_switch_cap(
    inner: &Rc<RefCell<Inner>>,
    device_id: u32,
    route_index: u32,
    card_profile_device: i32,
    props: &RouteProps,
) {
    // No props on this arrival means nothing to compare — leave the entry
    // pending so a later enumeration inside the window can still act.
    let Some(current) = props.volume else {
        return;
    };

    let mut guard = inner.borrow_mut();
    let Some((node_id, cap_linear)) = guard
        .pending_port_caps
        .matching(device_id, card_profile_device, route_index, Instant::now())
        .map(|pending| (pending.node_id, pending.cap_linear))
    else {
        return;
    };

    // `RouteProps::volume` is already the loudest `channelVolumes` entry, which
    // is the channel §9.2 compares against the cap.
    let Some(capped) = port_switch_clamp(&[current as f32], Some(cap_linear as f32)) else {
        return;
    };

    let channels = props.channels.unwrap_or(1).max(1);
    let target = RouteTarget {
        device_id,
        card_profile_device,
        index: route_index,
        props: props.clone(),
    };
    let before = linear_to_percent(current);
    let after = linear_to_percent(f64::from(capped));
    match write_route_volume(&guard, &target, vec![capped; channels]) {
        Ok(()) => {
            // First clamp reads `clamped`; anything the session manager makes
            // us redo reads `re-clamped`, so the journal tells the story.
            let verb = if guard.pending_port_caps.note_clamp(node_id) > 1 {
                "re-clamped"
            } else {
                "clamped"
            };
            info!(
                node = node_id,
                device = device_id,
                port = route_index,
                "port switch restored {before:.0}%, above the {:.0}% cap; {verb} to {after:.0}%",
                linear_to_percent(cap_linear)
            );
        }
        Err(e) => warn!(
            node = node_id,
            device = device_id,
            port = route_index,
            "could not cap the level after a port switch: {e}"
        ),
    }
}

fn on_metadata_global(
    inner: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&spa::utils::dict::DictRef>,
) {
    let Some(props) = global.props else { return };
    let name = props.get(keys::METADATA_NAME).unwrap_or_default();
    // `default` carries the default-device selections (SPEC §2.1); `filters`
    // is where a smart filter is bypassed without unloading it (SPEC §7.1).
    let is_filters = name == eq::METADATA_NAME_FILTERS;
    if name != meta::METADATA_NAME_DEFAULT && !is_filters {
        return;
    }
    let metadata: Metadata = match registry.bind(global) {
        Ok(m) => m,
        Err(e) => {
            warn!(id = global.id, name, "could not bind metadata: {e}");
            return;
        }
    };

    let listener = {
        let inner = inner.clone();
        metadata
            .add_listener_local()
            .property(move |subject, key, _type, value| {
                on_metadata_property(&inner, subject, key, value);
                0
            })
            .register()
    };

    let entry = Rc::new(MetadataEntry {
        proxy: metadata,
        _listener: listener,
    });

    let mut guard = inner.borrow_mut();
    if is_filters {
        guard.filters_metadata = Some(entry);
        info!(id = global.id, "bound the `filters` metadata object");
        guard.apply_eq();
    } else {
        guard.metadata = Some(entry);
        info!(id = global.id, "bound the `default` metadata object");
        guard.apply_notification_routing();
    }
    guard.publish();
}

fn on_link_global(inner: &Rc<RefCell<Inner>>, global: &GlobalObject<&spa::utils::dict::DictRef>) {
    let Some(props) = global.props else { return };
    let out_node = props
        .get(keys::LINK_OUTPUT_NODE)
        .and_then(|s| s.parse().ok());
    let in_node = props
        .get(keys::LINK_INPUT_NODE)
        .and_then(|s| s.parse().ok());
    let (Some(out_node), Some(in_node)) = (out_node, in_node) else {
        return;
    };
    let mut guard = inner.borrow_mut();
    guard.links.insert(global.id, (out_node, in_node));
    guard.publish();
}

fn on_global_remove(inner: &Rc<RefCell<Inner>>, id: u32) {
    let mut guard = inner.borrow_mut();
    guard.detach_eq_node(id);
    let mut changed = guard.nodes.remove(&id).is_some();
    changed |= guard.devices.remove(&id).is_some();
    changed |= guard.links.remove(&id).is_some();
    guard.routed.remove(&id);
    guard.targets.remove(&id);
    // SPEC §9.2: a node that went away can never confirm its switch.
    guard.pending_port_caps.remove_node(id);
    if guard
        .metadata
        .as_ref()
        .is_some_and(|m| m.proxy.upcast_ref().id() == id)
    {
        guard.metadata = None;
        changed = true;
    }
    if guard
        .filters_metadata
        .as_ref()
        .is_some_and(|m| m.proxy.upcast_ref().id() == id)
    {
        guard.filters_metadata = None;
        changed = true;
    }
    if changed {
        guard.apply_notification_routing();
        guard.apply_eq();
    }
    guard.publish();
}

fn on_node_info(inner: &Rc<RefCell<Inner>>, id: u32, props: &spa::utils::dict::DictRef) {
    let mut guard = inner.borrow_mut();
    let Some(entry) = guard.nodes.get_mut(&id) else {
        return;
    };
    let mut changed = false;
    for (key, field) in [
        (keys::MEDIA_NAME, &mut entry.media_name),
        (keys::MEDIA_ROLE, &mut entry.media_role),
        (keys::APPLICATION_NAME, &mut entry.app_name),
        (keys::APPLICATION_BINARY, &mut entry.binary),
    ] {
        if let Some(value) = props.get(key) {
            if field.as_str() != value {
                *field = value.to_owned();
                changed = true;
            }
        }
    }
    if let Some(value) = props.get(keys::NODE_DESCRIPTION) {
        if !value.is_empty() && entry.description != value {
            entry.description = value.to_owned();
            if entry.nick.is_empty() {
                entry.nick = value.to_owned();
            }
            changed = true;
        }
    }
    if let Some(value) = props.get(keys::NODE_NICK) {
        if !value.is_empty() && entry.nick != value {
            entry.nick = value.to_owned();
            changed = true;
        }
    }
    if let Some(value) = props.get(keys::DEVICE_ID).and_then(|s| s.parse().ok()) {
        if entry.device_id != Some(value) {
            entry.device_id = Some(value);
            changed = true;
        }
    }
    if let Some(value) = props
        .get(keys::CARD_PROFILE_DEVICE)
        .and_then(|s| s.parse().ok())
    {
        if entry.card_profile_device != Some(value) {
            entry.card_profile_device = Some(value);
            changed = true;
        }
    }
    // SPEC §8.1: `alsa.card` and the card-name keys are node-`info`-only too.
    if let Some(card) = alsa_card_from(props) {
        if entry.alsa_card.as_ref() != Some(&card) {
            entry.alsa_card = Some(card);
            changed = true;
        }
    }
    // `pipedeck.eq` / `node.link-group` are NOT in the registry global's
    // property whitelist — they only show up here, in the node's own info.
    // This is where our filter-chain nodes are recognised (bit us live
    // 2026-09-02: the chain loaded but its main node was never adopted, so no
    // preset was ever written).
    let mut became_hidden = false;
    if !entry.hidden
        && eq::is_eq_node(
            props.get(eq::PROP_PIPEDECK_EQ),
            props.get(keys::NODE_LINK_GROUP),
        )
    {
        entry.hidden = true;
        became_hidden = true;
        changed = true;
    }
    if became_hidden {
        guard.attach_eq_node(id);
        guard.apply_eq();
    }
    if changed {
        guard.apply_notification_routing();
        guard.publish();
    }
}

fn on_node_props(inner: &Rc<RefCell<Inner>>, id: u32, param: &Pod) {
    let Some((volume, mute, channels)) = parse_props(param) else {
        return;
    };
    let mut guard = inner.borrow_mut();
    let Some(entry) = guard.nodes.get_mut(&id) else {
        return;
    };
    let mut changed = false;
    if let Some(volume) = volume {
        if (entry.volume - volume).abs() > f64::EPSILON {
            entry.volume = volume;
            changed = true;
        }
    }
    if let Some(channels) = channels {
        if channels > 0 && entry.channels != channels {
            entry.channels = channels;
        }
    }
    if let Some(mute) = mute {
        if entry.mute != mute {
            entry.mute = mute;
            changed = true;
        }
    }
    if changed {
        guard.publish();
    }
}

fn on_metadata_property(
    inner: &Rc<RefCell<Inner>>,
    subject: u32,
    key: Option<&str>,
    value: Option<&str>,
) {
    let Some(key) = key else { return };
    let mut guard = inner.borrow_mut();

    if subject == 0 {
        let Some(kind) = meta::kind_for_effective_key(key) else {
            return;
        };
        let name = value.and_then(meta::parse_name_value);
        match kind {
            DeviceKind::Sink => guard.default_sink = name,
            DeviceKind::Source => guard.default_source = name,
        }
        guard.publish();
        return;
    }

    if key == meta::KEY_TARGET_OBJECT {
        match value {
            Some(value) => {
                guard.targets.insert(subject, value.to_owned());
            }
            None => {
                guard.targets.remove(&subject);
                guard.routed.remove(&subject);
            }
        }
        guard.publish();
    }
}

/// Handle a command from the D-Bus side. Runs on the PipeWire thread.
fn handle_command(inner: &Rc<RefCell<Inner>>, command: Command) {
    let outcome = match &command {
        Command::SetDefault { kind, name, .. } => set_default(inner, *kind, name),
        Command::SetVolume { id, volume, .. } => set_volume(inner, *id, *volume),
        Command::SetMute { id, mute, .. } => set_mute(inner, *id, *mute),
        Command::SetPort { id, index, .. } => set_port(inner, *id, *index),
        Command::SetStreamTarget { id, name, .. } => set_stream_target(inner, *id, name),
        Command::SetConfig { config, .. } => {
            let mut guard = inner.borrow_mut();
            guard.config = (**config).clone();
            guard.apply_notification_routing();
            guard.apply_eq();
            guard.publish();
            Ok(())
        }
        Command::Refresh { .. } => {
            let mut guard = inner.borrow_mut();
            for entry in guard.nodes.values() {
                entry
                    .proxy
                    .enum_params(0, Some(ParamType::Props), 0, u32::MAX);
            }
            for entry in guard.devices.values() {
                entry
                    .proxy
                    .enum_params(0, Some(ParamType::EnumRoute), 0, u32::MAX);
                entry
                    .proxy
                    .enum_params(0, Some(ParamType::Route), 0, u32::MAX);
            }
            guard.apply_notification_routing();
            guard.apply_eq();
            guard.publish();
            Ok(())
        }
        Command::Terminate => Ok(()),
    };

    if let Some(reply) = command.into_reply() {
        let _ = reply.send(outcome);
    }
}

fn set_default(inner: &Rc<RefCell<Inner>>, kind: DeviceKind, name: &str) -> Result<()> {
    let guard = inner.borrow();
    let metadata = guard
        .metadata
        .as_ref()
        .ok_or_else(|| Error::pipewire("the `default` metadata object is not available"))?;
    let wanted_role = match kind {
        DeviceKind::Sink => NodeRole::Sink,
        DeviceKind::Source => NodeRole::Source,
    };
    if !guard
        .nodes
        .values()
        .any(|n| n.role == wanted_role && n.name == name)
    {
        return Err(Error::not_found(format!("no {kind} named `{name}`")));
    }
    metadata.proxy.set_property(
        0,
        meta::configured_key(kind),
        Some(meta::TYPE_SPA_JSON),
        Some(&meta::format_name_value(name)),
    );
    Ok(())
}

fn set_volume(inner: &Rc<RefCell<Inner>>, id: u32, volume: f64) -> Result<()> {
    let mut guard = inner.borrow_mut();
    // SPEC §9.2: an explicit level for this node means the user is taking over,
    // so a port switch still inside its window must stop clamping on top of it.
    guard.pending_port_caps.remove_node(id);

    let entry = guard
        .nodes
        .get(&id)
        .ok_or_else(|| Error::not_found(format!("no node with id {id}")))?;
    let volume = clamp_volume(volume);

    // SPEC §6.1: on a routed (ALSA) node the node's own `Props` write is
    // silently dropped — the card's `Route` param is the only thing that takes.
    // SPEC §9.1: `mute` is deliberately absent, here and below. Carrying the
    // daemon's cached value would undo a mute made a few ms ago by the keyboard
    // key or the GNOME slider.
    if let Some(target) = guard.route_target(entry) {
        let channels = target.props.channels.unwrap_or(entry.channels).max(1);
        return write_route_volume(&guard, &target, vec![volume as f32; channels]);
    }

    let channels = entry.channels.max(1);
    let values = vec![volume as f32; channels];
    let pod = props_pod(volume_properties(values))
        .ok_or_else(|| Error::pipewire("could not build the channelVolumes pod"))?;
    let param = Pod::from_bytes(&pod)
        .ok_or_else(|| Error::pipewire("built an invalid channelVolumes pod"))?;
    entry.proxy.set_param(ParamType::Props, 0, param);
    Ok(())
}

fn set_mute(inner: &Rc<RefCell<Inner>>, id: u32, mute: bool) -> Result<()> {
    let mut guard = inner.borrow_mut();
    // SPEC §9.2: same hand-off rule as `set_volume` — the user is driving now.
    guard.pending_port_caps.remove_node(id);

    let entry = guard
        .nodes
        .get(&id)
        .ok_or_else(|| Error::not_found(format!("no node with id {id}")))?;

    // SPEC §9.1: no `channelVolumes` rides along — a stale cached level would
    // silently undo a volume change made elsewhere.
    if let Some(target) = guard.route_target(entry) {
        return write_route_mute(&guard, &target, mute);
    }

    let pod = props_pod(mute_properties(mute))
        .ok_or_else(|| Error::pipewire("could not build the mute pod"))?;
    let param =
        Pod::from_bytes(&pod).ok_or_else(|| Error::pipewire("built an invalid mute pod"))?;
    entry.proxy.set_param(ParamType::Props, 0, param);
    Ok(())
}

/// SPEC §6.1's `SetPort`: select a card route for a node.
///
/// SPEC §9.2: a switch PipeDeck itself issued is also recorded as *pending*, so
/// the `Route` re-enumeration that confirms it can clamp whatever level
/// WirePlumber restored for the new port.
fn set_port(inner: &Rc<RefCell<Inner>>, id: u32, index: u32) -> Result<()> {
    let mut guard = inner.borrow_mut();

    let (kind, device_id, card_profile_device) = {
        let entry = guard
            .nodes
            .get(&id)
            .ok_or_else(|| Error::not_found(format!("no node with id {id}")))?;
        let kind = entry
            .role
            .device_kind()
            .ok_or_else(|| Error::invalid(format!("node {id} is not a sink or source")))?;
        let (device_id, card_profile_device, routes) = guard
            .node_device(entry)
            .ok_or_else(|| Error::invalid(route::SetPortError::NoPorts.message(id, index)))?;

        route::validate_set_port(routes, kind, card_profile_device, index)
            .map_err(|e| Error::invalid(e.message(id, index)))?;
        (kind, device_id, card_profile_device)
    };

    {
        let device = guard
            .devices
            .get(&device_id)
            .ok_or_else(|| Error::pipewire(format!("device {device_id} is no longer bound")))?;
        let bytes = route_pod(index, card_profile_device, None)
            .ok_or_else(|| Error::pipewire("could not build the Route pod"))?;
        let param =
            Pod::from_bytes(&bytes).ok_or_else(|| Error::pipewire("built an invalid Route pod"))?;
        device.proxy.set_param(ParamType::Route, 0, param);
    }

    // SPEC §9.2: outputs only. The rule is about never making an *output*
    // louder than the user meant; silently pulling a capture level down would
    // be a surprise, not a safety net. Nothing is recorded while the cap is
    // off, so a switch made with `cap off` cannot be clamped by a
    // `pipedeck cap 60` issued a moment later.
    let cap = guard.config.port_switch_cap();
    match (kind, cap) {
        (DeviceKind::Sink, Some(cap_linear)) => guard.pending_port_caps.record(PendingPortSwitch {
            node_id: id,
            device_id,
            card_profile_device,
            route_index: index,
            cap_linear,
            issued: Instant::now(),
            clamps: 0,
        }),
        // A fresh switch always supersedes an older pending one for this node.
        _ => guard.pending_port_caps.remove_node(id),
    }
    Ok(())
}

/// SPEC §9.1: the `Props` body of a volume write — `channelVolumes` and
/// nothing else, so a `mute` set elsewhere survives.
fn volume_properties(channel_volumes: Vec<f32>) -> Vec<Property> {
    vec![Property::new(
        spa::sys::SPA_PROP_channelVolumes,
        Value::ValueArray(ValueArray::Float(channel_volumes)),
    )]
}

/// SPEC §9.1: the `Props` body of a mute write — `mute` and nothing else, so a
/// level set elsewhere survives.
fn mute_properties(mute: bool) -> Vec<Property> {
    vec![Property::new(spa::sys::SPA_PROP_mute, Value::Bool(mute))]
}

/// Write `channelVolumes` — and only that — through a card's active `Route`.
fn write_route_volume(
    inner: &Inner,
    target: &RouteTarget,
    channel_volumes: Vec<f32>,
) -> Result<()> {
    write_route_props(inner, target, volume_properties(channel_volumes))
}

/// Write `mute` — and only that — through a card's active `Route`.
fn write_route_mute(inner: &Inner, target: &RouteTarget, mute: bool) -> Result<()> {
    write_route_props(inner, target, mute_properties(mute))
}

/// Write one `Props` object through a card's active `Route`.
///
/// SPEC §9.1: PipeWire/ACP apply each Route property independently, so the
/// object carries exactly the keys the caller is changing and never a cached
/// value of anything else. Every caller goes through [`write_route_volume`] or
/// [`write_route_mute`], which is what keeps that true.
fn write_route_props(inner: &Inner, target: &RouteTarget, props: Vec<Property>) -> Result<()> {
    let device = inner.devices.get(&target.device_id).ok_or_else(|| {
        Error::pipewire(format!("device {} is no longer bound", target.device_id))
    })?;
    let bytes = route_pod(target.index, target.card_profile_device, Some(props))
        .ok_or_else(|| Error::pipewire("could not build the Route pod"))?;
    let param =
        Pod::from_bytes(&bytes).ok_or_else(|| Error::pipewire("built an invalid Route pod"))?;
    device.proxy.set_param(ParamType::Route, 0, param);
    Ok(())
}

fn set_stream_target(inner: &Rc<RefCell<Inner>>, id: u32, name: &str) -> Result<()> {
    let mut guard = inner.borrow_mut();
    if !guard.nodes.contains_key(&id) {
        return Err(Error::not_found(format!("no stream with id {id}")));
    }
    if !name.is_empty() && !guard.sink_exists(name) {
        return Err(Error::not_found(format!("no sink named `{name}`")));
    }
    let metadata = guard
        .metadata
        .clone()
        .ok_or_else(|| Error::pipewire("the `default` metadata object is not available"))?;

    if name.is_empty() {
        metadata
            .proxy
            .set_property(id, meta::KEY_TARGET_OBJECT, None, None);
        guard.targets.remove(&id);
        guard.routed.remove(&id);
    } else {
        metadata.proxy.set_property(
            id,
            meta::KEY_TARGET_OBJECT,
            Some(meta::TYPE_SPA_STRING),
            Some(name),
        );
        guard.targets.insert(id, name.to_owned());
        guard.routed.remove(&id);
    }
    guard.publish();
    Ok(())
}

/// Load `libpipewire-module-filter-chain` into the daemon's own context
/// (SPEC §7.1).
///
/// pipewire-rs 0.10 has no safe wrapper for module loading, so this is the one
/// `unsafe` call in the crate besides the pod helpers. It **must** run on the
/// PipeWire thread: `pw_context_load_module` builds nodes on the context's loop
/// and is not thread-safe. Every caller is reached from a listener or a command
/// handler, both of which the loop invokes on that thread.
///
/// The returned handle owns the module and unloads it on drop.
fn load_filter_chain(context: &ContextRc, args: &str) -> Option<ModuleHandle> {
    let name = CString::new(eq::FILTER_CHAIN_MODULE).ok()?;
    let args = CString::new(args)
        .inspect_err(|_| error!("EQ module arguments contained a NUL byte"))
        .ok()?;
    // SAFETY: `context` is this thread's live context; both strings outlive the
    // call; a null `properties` is the documented "no extra properties" value.
    let module = unsafe {
        pw::sys::pw_context_load_module(
            context.as_raw_ptr(),
            name.as_ptr(),
            args.as_ptr(),
            std::ptr::null_mut(),
        )
    };
    if module.is_null() {
        None
    } else {
        Some(ModuleHandle(module))
    }
}

/// Build the `Props` pod that sets a filter chain's named controls.
///
/// SPEC §7.1: `Object(Props){ params: Struct[ "pre:Mult", <f>, "ls:Freq", … ] }`
/// — filter-chain's documented runtime-control interface, a Struct of
/// alternating string and float.
fn eq_params_pod(params: &[(String, f32)]) -> Option<Vec<u8>> {
    let mut fields: Vec<Value> = Vec::with_capacity(params.len() * 2);
    for (name, value) in params {
        fields.push(Value::String(name.clone()));
        fields.push(Value::Float(*value));
    }
    props_pod(vec![Property::new(
        spa::sys::SPA_PROP_params,
        Value::Struct(fields),
    )])
}

/// Read an EQ params pod back into `(control, value)` pairs.
///
/// Only used by the round-trip test — nothing on the graph sends these to us —
/// but it is the only way to prove the pod shape without a server.
#[cfg(test)]
fn parse_eq_params(param: &Pod) -> Option<Vec<(String, f32)>> {
    let (_rest, value) = PodDeserializer::deserialize_any_from(param.as_bytes()).ok()?;
    let Value::Object(object) = value else {
        return None;
    };
    let fields = object.properties.iter().find_map(|p| {
        (p.key == spa::sys::SPA_PROP_params).then(|| match &p.value {
            Value::Struct(fields) => Some(fields.clone()),
            _ => None,
        })
    })??;
    let mut out = Vec::with_capacity(fields.len() / 2);
    for pair in fields.chunks(2) {
        let [Value::String(name), Value::Float(value)] = pair else {
            return None;
        };
        out.push((name.clone(), *value));
    }
    Some(out)
}

/// Wrap `Props` properties into a serialised `SPA_TYPE_OBJECT_Props` pod.
fn props_pod(properties: Vec<Property>) -> Option<Vec<u8>> {
    let value = Value::Object(Object {
        type_: SpaTypes::ObjectParamProps.as_raw(),
        id: ParamType::Props.as_raw(),
        properties,
    });
    let (cursor, _len) =
        PodSerializer::serialize(std::io::Cursor::new(Vec::<u8>::new()), &value).ok()?;
    Some(cursor.into_inner())
}

/// Build a `SPA_TYPE_OBJECT_ParamRoute` pod for `Device::set_param(Route)`.
///
/// SPEC §6.1: `{ index, device, [props], save: true }`. `pulse-server` builds
/// the nested props object as `SPA_TYPE_OBJECT_Props` with object id
/// `SPA_PARAM_Route`, and so do we.
fn route_pod(index: u32, device: i32, props: Option<Vec<Property>>) -> Option<Vec<u8>> {
    let mut properties = vec![
        Property::new(
            spa::sys::SPA_PARAM_ROUTE_index,
            Value::Int(i32::try_from(index).ok()?),
        ),
        Property::new(spa::sys::SPA_PARAM_ROUTE_device, Value::Int(device)),
    ];
    if let Some(props) = props {
        properties.push(Property::new(
            spa::sys::SPA_PARAM_ROUTE_props,
            Value::Object(Object {
                type_: SpaTypes::ObjectParamProps.as_raw(),
                id: ParamType::Route.as_raw(),
                properties: props,
            }),
        ));
    }
    properties.push(Property::new(
        spa::sys::SPA_PARAM_ROUTE_save,
        Value::Bool(true),
    ));
    route_object_pod(properties)
}

/// Serialise route properties into a `SPA_TYPE_OBJECT_ParamRoute` pod.
fn route_object_pod(properties: Vec<Property>) -> Option<Vec<u8>> {
    let value = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::Route.as_raw(),
        properties,
    });
    let (cursor, _len) =
        PodSerializer::serialize(std::io::Cursor::new(Vec::<u8>::new()), &value).ok()?;
    Some(cursor.into_inner())
}

/// A `SPA_TYPE_OBJECT_ParamRoute` object as it arrives from either `EnumRoute`
/// (the catalogue) or `Route` (the active selection). Both share one shape;
/// which fields are present is what tells them apart.
struct RawRoute {
    index: Option<u32>,
    direction: Option<RouteDirection>,
    device: Option<i32>,
    name: String,
    description: String,
    priority: u32,
    available: Availability,
    devices: Vec<i32>,
    profiles: Vec<i32>,
    props: Option<RouteProps>,
}

/// Parse a route object pod. `None` when the pod is not an object at all.
fn parse_route(param: &Pod) -> Option<RawRoute> {
    let (_rest, value) = PodDeserializer::deserialize_any_from(param.as_bytes()).ok()?;
    let Value::Object(object) = value else {
        return None;
    };

    let mut raw = RawRoute {
        index: None,
        direction: None,
        device: None,
        name: String::new(),
        description: String::new(),
        priority: 0,
        available: Availability::Unknown,
        devices: Vec::new(),
        profiles: Vec::new(),
        props: None,
    };

    for property in &object.properties {
        match property.key {
            spa::sys::SPA_PARAM_ROUTE_index => {
                raw.index = value_int(&property.value).and_then(|v| u32::try_from(v).ok());
            }
            spa::sys::SPA_PARAM_ROUTE_direction => {
                raw.direction = value_id(&property.value).and_then(RouteDirection::from_raw);
            }
            spa::sys::SPA_PARAM_ROUTE_device => raw.device = value_int(&property.value),
            spa::sys::SPA_PARAM_ROUTE_name => {
                if let Some(v) = value_str(&property.value) {
                    raw.name = v.to_owned();
                }
            }
            spa::sys::SPA_PARAM_ROUTE_description => {
                if let Some(v) = value_str(&property.value) {
                    raw.description = v.to_owned();
                }
            }
            spa::sys::SPA_PARAM_ROUTE_priority => {
                raw.priority = value_int(&property.value)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0);
            }
            spa::sys::SPA_PARAM_ROUTE_available => {
                raw.available = value_id(&property.value)
                    .map(Availability::from_raw)
                    .unwrap_or(Availability::Unknown);
            }
            spa::sys::SPA_PARAM_ROUTE_devices => raw.devices = value_int_array(&property.value),
            spa::sys::SPA_PARAM_ROUTE_profiles => raw.profiles = value_int_array(&property.value),
            spa::sys::SPA_PARAM_ROUTE_props => {
                if let Value::Object(props) = &property.value {
                    let (volume, mute, channels) = parse_props_object(props);
                    raw.props = Some(RouteProps {
                        volume,
                        mute,
                        channels,
                    });
                }
            }
            _ => {}
        }
    }

    // A route with no description still deserves a label in the panel.
    if raw.description.is_empty() {
        raw.description = raw.name.clone();
    }
    Some(raw)
}

/// An enumerated value, tolerating the `Int` and `Choice` spellings PipeWire is
/// free to use for any property.
fn value_id(value: &Value) -> Option<u32> {
    match value {
        Value::Id(id) => Some(id.0),
        Value::Int(v) => u32::try_from(*v).ok(),
        Value::Choice(ChoiceValue::Id(Choice(_, ChoiceEnum::None(id)))) => Some(id.0),
        _ => None,
    }
}

/// A 32-bit integer, tolerating `Id` and `Choice` spellings.
fn value_int(value: &Value) -> Option<i32> {
    match value {
        Value::Int(v) => Some(*v),
        Value::Id(id) => i32::try_from(id.0).ok(),
        Value::Choice(ChoiceValue::Int(Choice(_, ChoiceEnum::None(v)))) => Some(*v),
        _ => None,
    }
}

/// A string property.
fn value_str(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// An array of 32-bit integers; a bare scalar counts as a one-element array.
fn value_int_array(value: &Value) -> Vec<i32> {
    match value {
        Value::ValueArray(ValueArray::Int(values)) => values.clone(),
        Value::ValueArray(ValueArray::Id(values)) => values
            .iter()
            .filter_map(|id| i32::try_from(id.0).ok())
            .collect(),
        Value::Int(v) => vec![*v],
        _ => Vec::new(),
    }
}

/// Pull volume, mute and channel count out of a `Props` param pod.
///
/// Returns `None` when the pod is not a `Props` object at all.
fn parse_props(param: &Pod) -> Option<(Option<f64>, Option<bool>, Option<usize>)> {
    let (_rest, value) = PodDeserializer::deserialize_any_from(param.as_bytes()).ok()?;
    let Value::Object(object) = value else {
        return None;
    };
    Some(parse_props_object(&object))
}

/// The `Props` object body, shared by the node `Props` param and the `props`
/// sub-object of a device `Route` (SPEC §6.1 — same shape, two carriers).
fn parse_props_object(object: &Object) -> (Option<f64>, Option<bool>, Option<usize>) {
    let mut volume = None;
    let mut mute = None;
    let mut channels = None;

    for property in &object.properties {
        if property.key == spa::sys::SPA_PROP_channelVolumes {
            if let Value::ValueArray(ValueArray::Float(values)) = &property.value {
                channels = Some(values.len());
                // All channels are kept equal by this daemon; take the loudest
                // so an externally-set per-channel volume still reads sensibly.
                volume = values
                    .iter()
                    .copied()
                    .fold(None::<f32>, |acc, v| Some(acc.map_or(v, |a| a.max(v))))
                    .map(f64::from);
            }
        } else if property.key == spa::sys::SPA_PROP_volume {
            if volume.is_none() {
                if let Value::Float(v) = property.value {
                    volume = Some(f64::from(v));
                }
            }
        } else if property.key == spa::sys::SPA_PROP_mute {
            if let Value::Bool(v) = property.value {
                mute = Some(v);
            }
        }
    }

    (volume.map(clamp_volume), mute, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_class_mapping() {
        assert_eq!(
            NodeRole::from_media_class("Audio/Sink"),
            Some(NodeRole::Sink)
        );
        assert_eq!(
            NodeRole::from_media_class("Audio/Sink/Virtual"),
            Some(NodeRole::Sink)
        );
        assert_eq!(
            NodeRole::from_media_class("Audio/Source"),
            Some(NodeRole::Source)
        );
        assert_eq!(
            NodeRole::from_media_class("Audio/Source/Virtual"),
            Some(NodeRole::Source)
        );
        assert_eq!(
            NodeRole::from_media_class("Stream/Output/Audio"),
            Some(NodeRole::PlaybackStream)
        );
        assert_eq!(
            NodeRole::from_media_class("Stream/Input/Audio"),
            Some(NodeRole::CaptureStream)
        );
        assert_eq!(NodeRole::from_media_class("Video/Sink"), None);
        assert_eq!(NodeRole::from_media_class("Midi/Bridge"), None);
    }

    #[test]
    fn role_classification_helpers() {
        assert_eq!(NodeRole::Sink.device_kind(), Some(DeviceKind::Sink));
        assert_eq!(NodeRole::Source.device_kind(), Some(DeviceKind::Source));
        assert_eq!(NodeRole::PlaybackStream.device_kind(), None);
        assert!(NodeRole::PlaybackStream.is_stream());
        assert!(NodeRole::CaptureStream.is_stream());
        assert!(!NodeRole::Sink.is_stream());
    }

    /// The pod round-trip is pure libspa: no graph needed, so it runs in CI.
    #[test]
    fn props_pod_round_trips_volume_and_mute() {
        let bytes = props_pod(vec![
            Property::new(
                spa::sys::SPA_PROP_channelVolumes,
                Value::ValueArray(ValueArray::Float(vec![0.25, 0.5])),
            ),
            Property::new(spa::sys::SPA_PROP_mute, Value::Bool(true)),
        ])
        .expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        let (volume, mute, channels) = parse_props(pod).expect("parses");
        assert_eq!(channels, Some(2));
        assert_eq!(mute, Some(true));
        assert!((volume.expect("volume") - 0.5).abs() < 1e-6);
    }

    #[test]
    fn props_pod_falls_back_to_scalar_volume() {
        let bytes = props_pod(vec![Property::new(
            spa::sys::SPA_PROP_volume,
            Value::Float(0.75),
        )])
        .expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        let (volume, mute, channels) = parse_props(pod).expect("parses");
        assert!((volume.expect("volume") - 0.75).abs() < 1e-6);
        assert_eq!(mute, None);
        assert_eq!(channels, None);
    }

    /// Devices: `subscribe_params` is dead on PipeWire 1.6.2, so the `info`
    /// event's param list is what drives re-reads. Only READ-able params we
    /// actually track may come back.
    #[test]
    fn info_param_list_selects_what_to_reenumerate() {
        let device_wanted = [ParamType::EnumRoute, ParamType::Route];

        // The real chronos shape: a card lists Profile/EnumProfile/Route/
        // EnumRoute/... — we must pick out exactly our two.
        let reported = [
            (ParamType::EnumProfile.as_raw(), true),
            (ParamType::Profile.as_raw(), true),
            (ParamType::EnumRoute.as_raw(), true),
            (ParamType::Route.as_raw(), true),
        ];
        assert_eq!(
            params_to_reenumerate(&reported, &device_wanted),
            vec![ParamType::EnumRoute, ParamType::Route]
        );

        // A param the server does not mark readable is not re-enumerated.
        let write_only = [
            (ParamType::EnumRoute.as_raw(), true),
            (ParamType::Route.as_raw(), false),
        ];
        assert_eq!(
            params_to_reenumerate(&write_only, &device_wanted),
            vec![ParamType::EnumRoute]
        );

        // A card with no routes at all asks for nothing.
        let no_routes = [(ParamType::EnumProfile.as_raw(), true)];
        assert!(params_to_reenumerate(&no_routes, &device_wanted).is_empty());

        // An info event carrying no param list falls back to everything.
        assert_eq!(
            params_to_reenumerate(&[], &device_wanted),
            vec![ParamType::EnumRoute, ParamType::Route]
        );

        // The node side tracks only Props.
        let node_reported = [
            (ParamType::Props.as_raw(), true),
            (ParamType::Format.as_raw(), true),
        ];
        assert_eq!(
            params_to_reenumerate(&node_reported, &[ParamType::Props]),
            vec![ParamType::Props]
        );
        assert!(
            params_to_reenumerate(&[(ParamType::Format.as_raw(), true)], &[ParamType::Props])
                .is_empty()
        );
    }

    /// The `Route` write of SPEC §6.1, round-tripped through libspa without a
    /// server — same style as the `Props` pod test above.
    #[test]
    fn route_pod_round_trips_index_device_props_and_save() {
        let bytes = route_pod(
            4,
            4,
            Some(vec![
                Property::new(
                    spa::sys::SPA_PROP_channelVolumes,
                    Value::ValueArray(ValueArray::Float(vec![0.064, 0.064])),
                ),
                Property::new(spa::sys::SPA_PROP_mute, Value::Bool(false)),
            ]),
        )
        .expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");

        let raw = parse_route(pod).expect("parses");
        assert_eq!(raw.index, Some(4));
        assert_eq!(raw.device, Some(4));
        let props = raw.props.expect("props survived the nesting");
        assert_eq!(props.channels, Some(2));
        assert_eq!(props.mute, Some(false));
        assert!((props.volume.expect("volume") - 0.064).abs() < 1e-6);

        // `save: true` must be on the wire, or WirePlumber forgets the choice.
        let (_rest, value) =
            PodDeserializer::deserialize_any_from(pod.as_bytes()).expect("deserialises");
        let Value::Object(object) = value else {
            panic!("not an object");
        };
        assert_eq!(object.type_, SpaTypes::ObjectParamRoute.as_raw());
        assert_eq!(object.id, ParamType::Route.as_raw());
        let save = object
            .properties
            .iter()
            .find(|p| p.key == spa::sys::SPA_PARAM_ROUTE_save)
            .expect("save property");
        assert_eq!(save.value, Value::Bool(true));
    }

    /// Every `SPA_PROP_*` key present in a pod's top-level `Props` object.
    fn props_keys(bytes: &[u8]) -> Vec<u32> {
        let (_rest, value) = PodDeserializer::deserialize_any_from(bytes).expect("deserialises");
        let Value::Object(object) = value else {
            panic!("not an object");
        };
        object.properties.iter().map(|p| p.key).collect()
    }

    /// Every `SPA_PROP_*` key inside a `Route` pod's nested `props` object.
    fn route_props_keys(bytes: &[u8]) -> Vec<u32> {
        let (_rest, value) = PodDeserializer::deserialize_any_from(bytes).expect("deserialises");
        let Value::Object(object) = value else {
            panic!("not an object");
        };
        let props = object
            .properties
            .iter()
            .find(|p| p.key == spa::sys::SPA_PARAM_ROUTE_props)
            .expect("props field");
        let Value::Object(nested) = &props.value else {
            panic!("props is not an object");
        };
        nested.properties.iter().map(|p| p.key).collect()
    }

    /// SPEC §9.1: a volume write through the card `Route` carries
    /// `channelVolumes` and **no** `mute`, so it cannot undo a mute set by the
    /// keyboard key or the GNOME slider a few milliseconds earlier.
    #[test]
    fn route_volume_write_carries_no_mute() {
        let bytes =
            route_pod(4, 4, Some(volume_properties(vec![0.216, 0.216]))).expect("pod builds");
        assert_eq!(
            route_props_keys(&bytes),
            vec![spa::sys::SPA_PROP_channelVolumes]
        );

        // ... and it still parses as a route whose props carry only a volume.
        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        let props = parse_route(pod).expect("parses").props.expect("props");
        assert_eq!(props.mute, None);
        assert_eq!(props.channels, Some(2));
        assert!((props.volume.expect("volume") - 0.216).abs() < 1e-6);
    }

    /// SPEC §9.1, the other direction: a mute write carries `mute` and **no**
    /// `channelVolumes`, so a stale cached level cannot undo a volume change.
    #[test]
    fn route_mute_write_carries_no_volume() {
        let bytes = route_pod(4, 4, Some(mute_properties(true))).expect("pod builds");
        assert_eq!(route_props_keys(&bytes), vec![spa::sys::SPA_PROP_mute]);

        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        let props = parse_route(pod).expect("parses").props.expect("props");
        assert_eq!(props.mute, Some(true));
        assert_eq!(props.volume, None);
        assert_eq!(props.channels, None);
    }

    /// SPEC §9.1 applies to the node-`Props` path too — streams and any sink
    /// with no card route behind it.
    #[test]
    fn node_props_writes_are_independent_too() {
        let volume = props_pod(volume_properties(vec![0.5, 0.5])).expect("pod builds");
        assert_eq!(props_keys(&volume), vec![spa::sys::SPA_PROP_channelVolumes]);
        let (_v, mute, channels) =
            parse_props(Pod::from_bytes(&volume).expect("valid pod")).expect("parses");
        assert_eq!(mute, None);
        assert_eq!(channels, Some(2));

        let muted = props_pod(mute_properties(false)).expect("pod builds");
        assert_eq!(props_keys(&muted), vec![spa::sys::SPA_PROP_mute]);
        let (volume, mute, channels) =
            parse_props(Pod::from_bytes(&muted).expect("valid pod")).expect("parses");
        assert_eq!(volume, None);
        assert_eq!(mute, Some(false));
        assert_eq!(channels, None);
    }

    /// `SetPort` sends no props at all (SPEC §6.1).
    #[test]
    fn route_pod_without_props_omits_the_props_field() {
        let bytes = route_pod(3, 4, None).expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        let raw = parse_route(pod).expect("parses");
        assert_eq!(raw.index, Some(3));
        assert_eq!(raw.device, Some(4));
        assert!(raw.props.is_none());
    }

    /// The `EnumRoute` catalogue shape, as `pw-dump 53` shows it on chronos.
    #[test]
    fn enum_route_pod_round_trips_the_catalogue_fields() {
        use spa::utils::Id;

        let bytes = route_object_pod(vec![
            Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(4)),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_direction,
                Value::Id(Id(RouteDirection::OUTPUT_RAW)),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_name,
                Value::String("analog-output-headphones".to_owned()),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_description,
                Value::String("Headphones".to_owned()),
            ),
            Property::new(spa::sys::SPA_PARAM_ROUTE_priority, Value::Int(9900)),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_available,
                Value::Id(Id(Availability::Yes.as_raw())),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_devices,
                Value::ValueArray(ValueArray::Int(vec![4, 5])),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_profiles,
                Value::ValueArray(ValueArray::Int(vec![1, 2])),
            ),
        ])
        .expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");

        let raw = parse_route(pod).expect("parses");
        assert_eq!(raw.index, Some(4));
        assert_eq!(raw.direction, Some(RouteDirection::Output));
        assert_eq!(raw.name, "analog-output-headphones");
        assert_eq!(raw.description, "Headphones");
        assert_eq!(raw.priority, 9900);
        assert_eq!(raw.available, Availability::Yes);
        assert_eq!(raw.devices, vec![4, 5]);
        assert_eq!(raw.profiles, vec![1, 2]);
        assert!(raw.props.is_none());
        assert!(raw.device.is_none());
    }

    /// A route with no `description` still gets a usable label.
    #[test]
    fn route_description_falls_back_to_the_name() {
        use spa::utils::Id;

        let bytes = route_object_pod(vec![
            Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(0)),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_direction,
                Value::Id(Id(RouteDirection::INPUT_RAW)),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_name,
                Value::String("analog-input-front-mic".to_owned()),
            ),
        ])
        .expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        let raw = parse_route(pod).expect("parses");
        assert_eq!(raw.description, "analog-input-front-mic");
        assert_eq!(raw.available, Availability::Unknown);
        assert!(raw.devices.is_empty());
    }

    /// A scalar where an array is expected still yields one element.
    #[test]
    fn value_helpers_tolerate_scalar_and_choice_spellings() {
        use spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Id};

        assert_eq!(value_int_array(&Value::Int(4)), vec![4]);
        assert_eq!(value_int_array(&Value::Bool(true)), Vec::<i32>::new());
        assert_eq!(value_int(&Value::Id(Id(7))), Some(7));
        assert_eq!(value_id(&Value::Int(2)), Some(2));
        assert_eq!(value_str(&Value::Int(1)), None);
        assert_eq!(
            value_int(&Value::Choice(ChoiceValue::Int(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::None(9)
            )))),
            Some(9)
        );
        assert_eq!(
            value_id(&Value::Choice(ChoiceValue::Id(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::None(Id(2))
            )))),
            Some(2)
        );
    }

    /// SPEC §7.1's preset write: `Object(Props){ params: Struct[ String, Float,
    /// … ] }`, round-tripped through libspa without a server — same style as the
    /// `Route` pod test above.
    #[test]
    fn eq_params_pod_round_trips_the_control_struct() {
        use crate::eq::{Band, BandKind, Preset};

        let preset = Preset {
            id: "hd650".to_owned(),
            name: "HD 650".to_owned(),
            preamp_db: -6.4,
            bands: vec![
                Band {
                    kind: BandKind::Lowshelf,
                    freq: 105.0,
                    q: 0.7,
                    gain_db: 5.1,
                },
                Band {
                    kind: BandKind::Peaking,
                    freq: 1030.0,
                    q: 1.44,
                    gain_db: -2.4,
                },
                Band {
                    kind: BandKind::Highshelf,
                    freq: 10_000.0,
                    q: 0.7,
                    gain_db: -1.2,
                },
            ],
        };
        let params = crate::eq::preset_to_params(&preset);
        let bytes = eq_params_pod(&params).expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");

        // It really is a Props object carrying SPA_PROP_params.
        let (_rest, value) =
            PodDeserializer::deserialize_any_from(pod.as_bytes()).expect("deserialises");
        let Value::Object(object) = value else {
            panic!("not an object");
        };
        assert_eq!(object.type_, SpaTypes::ObjectParamProps.as_raw());
        assert_eq!(object.id, ParamType::Props.as_raw());
        assert_eq!(object.properties.len(), 1);
        assert_eq!(object.properties[0].key, spa::sys::SPA_PROP_params);

        let read_back = parse_eq_params(pod).expect("parses");
        assert_eq!(read_back.len(), params.len());
        for ((a_name, a_value), (b_name, b_value)) in read_back.iter().zip(params.iter()) {
            assert_eq!(a_name, b_name);
            assert!((a_value - b_value).abs() < 1e-6, "{a_name}");
        }
        assert_eq!(read_back[0].0, "pre:Mult");
        assert!((read_back[0].1 - crate::eq::preamp_mult(-6.4)).abs() < 1e-6);
    }

    /// An empty control list is still a well-formed pod, so a preset with no
    /// bands cannot produce garbage on the wire.
    #[test]
    fn eq_params_pod_handles_an_empty_control_list() {
        let bytes = eq_params_pod(&[]).expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        assert_eq!(parse_eq_params(pod), Some(Vec::new()));
    }

    #[test]
    fn props_parsing_clamps_out_of_range_volumes() {
        let bytes = props_pod(vec![Property::new(
            spa::sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![9.0, 9.0])),
        )])
        .expect("pod builds");
        let pod = Pod::from_bytes(&bytes).expect("valid pod");
        let (volume, _, _) = parse_props(pod).expect("parses");
        assert!((volume.expect("volume") - crate::volume::MAX_VOLUME).abs() < 1e-9);
    }
}
