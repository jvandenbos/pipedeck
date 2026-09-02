//! Everything that touches libpipewire.
//!
//! The rest of the crate is pure data so it can be unit-tested without a graph;
//! this module is the only place that binds registry globals, reads `Props`
//! params and writes WirePlumber metadata. It runs on its own `std::thread`
//! driving a libpipewire `MainLoop`, per SPEC §2.1 — nothing here is ever
//! called from the tokio side.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use pipewire as pw;
use pw::spa;

use spa::param::ParamType;
use spa::pod::deserialize::PodDeserializer;
use spa::pod::serialize::PodSerializer;
use spa::pod::{ChoiceValue, Object, Pod, Property, Value, ValueArray};
use spa::utils::{Choice, ChoiceEnum, SpaTypes};

use pw::context::ContextRc;
use pw::device::{Device as PwDevice, DeviceListener};
use pw::main_loop::MainLoopRc;
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeListener};
use pw::proxy::ProxyT;
use pw::registry::GlobalObject;
use pw::types::ObjectType;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::command::Command;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::matching::is_notification_stream;
use crate::meta;
use crate::route::{
    self, ActiveRoute, Availability, DeviceRoutes, Port, Route as CardRoute, RouteDirection,
    RouteProps,
};
use crate::state::{Device, DeviceKind, State, Stream};
use crate::volume::clamp_volume;

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
    serial: Option<u64>,
    channels: usize,
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

    fn sink_exists(&self, name: &str) -> bool {
        self.nodes
            .values()
            .any(|n| n.role == NodeRole::Sink && n.name == name)
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

        for (id, entry) in &self.nodes {
            if let Some(kind) = entry.role.device_kind() {
                // On ALSA cards WirePlumber owns volume through the device
                // `Route` param, so that is what the panel must be shown
                // (SPEC §6.1); nodes without a route keep their own `Props`.
                let mut volume = entry.volume;
                let mut mute = entry.mute;
                if let Some((_, card_profile_device, routes)) = self.node_device(entry) {
                    ports.extend(routes.ports_for(*id, kind, card_profile_device));
                    if let Some(active) = routes.active_for(card_profile_device) {
                        if let Some(v) = active.props.volume {
                            volume = v;
                        }
                        if let Some(m) = active.props.mute {
                            mute = m;
                        }
                    }
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
        notify: watch::Sender<u64>,
        exited: UnboundedSender<()>,
    ) -> std::result::Result<Self, String> {
        let (sender, receiver) = pw::channel::channel::<Command>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();

        let handle = std::thread::Builder::new()
            .name("pipedeck-pw".to_owned())
            .spawn(move || {
                let result = run(config, state, notify, receiver, &ready_tx);
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
        nodes: HashMap::new(),
        devices: HashMap::new(),
        links: HashMap::new(),
        targets: HashMap::new(),
        metadata: None,
        routed: HashSet::new(),
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

    let listener = {
        let inner = inner.clone();
        node.add_listener_local()
            .info({
                let inner = inner.clone();
                move |info| {
                    if let Some(props) = info.props() {
                        on_node_info(&inner, id, props);
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
        serial: props.get(keys::OBJECT_SERIAL).and_then(|s| s.parse().ok()),
        channels: 2,
        volume: 1.0,
        mute: false,
    };

    let mut guard = inner.borrow_mut();
    guard.nodes.insert(id, entry);
    guard.apply_notification_routing();
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
        debug!(id, ?param_type, index, "device param did not parse as a route");
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
                entry.routes.active.insert(
                    device,
                    ActiveRoute {
                        index: route_index,
                        device,
                        props: raw.props.unwrap_or_default(),
                    },
                );
            }
        }
    }

    inner.borrow_mut().publish();
}

fn on_metadata_global(
    inner: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&spa::utils::dict::DictRef>,
) {
    let Some(props) = global.props else { return };
    if props.get(keys::METADATA_NAME) != Some(meta::METADATA_NAME_DEFAULT) {
        return;
    }
    let metadata: Metadata = match registry.bind(global) {
        Ok(m) => m,
        Err(e) => {
            warn!(id = global.id, "could not bind default metadata: {e}");
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

    let mut guard = inner.borrow_mut();
    guard.metadata = Some(Rc::new(MetadataEntry {
        proxy: metadata,
        _listener: listener,
    }));
    guard.apply_notification_routing();
    guard.publish();
    info!(id = global.id, "bound the `default` metadata object");
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
    let mut changed = guard.nodes.remove(&id).is_some();
    changed |= guard.devices.remove(&id).is_some();
    changed |= guard.links.remove(&id).is_some();
    guard.routed.remove(&id);
    guard.targets.remove(&id);
    if guard
        .metadata
        .as_ref()
        .is_some_and(|m| m.proxy.upcast_ref().id() == id)
    {
        guard.metadata = None;
        changed = true;
    }
    if changed {
        guard.apply_notification_routing();
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
    let guard = inner.borrow();
    let entry = guard
        .nodes
        .get(&id)
        .ok_or_else(|| Error::not_found(format!("no node with id {id}")))?;
    let volume = clamp_volume(volume);

    // SPEC §6.1: on a routed (ALSA) node the node's own `Props` write is
    // silently dropped — the card's `Route` param is the only thing that takes.
    if let Some(target) = guard.route_target(entry) {
        let channels = target.props.channels.unwrap_or(entry.channels).max(1);
        let mute = target.props.mute.unwrap_or(entry.mute);
        return write_route_props(&guard, &target, vec![volume as f32; channels], mute);
    }

    let channels = entry.channels.max(1);
    let values = vec![volume as f32; channels];
    let pod = props_pod(vec![Property::new(
        spa::sys::SPA_PROP_channelVolumes,
        Value::ValueArray(ValueArray::Float(values)),
    )])
    .ok_or_else(|| Error::pipewire("could not build the channelVolumes pod"))?;
    let param = Pod::from_bytes(&pod)
        .ok_or_else(|| Error::pipewire("built an invalid channelVolumes pod"))?;
    entry.proxy.set_param(ParamType::Props, 0, param);
    Ok(())
}

fn set_mute(inner: &Rc<RefCell<Inner>>, id: u32, mute: bool) -> Result<()> {
    let guard = inner.borrow();
    let entry = guard
        .nodes
        .get(&id)
        .ok_or_else(|| Error::not_found(format!("no node with id {id}")))?;

    if let Some(target) = guard.route_target(entry) {
        let channels = target.props.channels.unwrap_or(entry.channels).max(1);
        let volume = target.props.volume.unwrap_or(entry.volume);
        return write_route_props(&guard, &target, vec![volume as f32; channels], mute);
    }

    let pod = props_pod(vec![Property::new(
        spa::sys::SPA_PROP_mute,
        Value::Bool(mute),
    )])
    .ok_or_else(|| Error::pipewire("could not build the mute pod"))?;
    let param =
        Pod::from_bytes(&pod).ok_or_else(|| Error::pipewire("built an invalid mute pod"))?;
    entry.proxy.set_param(ParamType::Props, 0, param);
    Ok(())
}

/// SPEC §6.1's `SetPort`: select a card route for a node.
fn set_port(inner: &Rc<RefCell<Inner>>, id: u32, index: u32) -> Result<()> {
    let guard = inner.borrow();
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

    let device = guard
        .devices
        .get(&device_id)
        .ok_or_else(|| Error::pipewire(format!("device {device_id} is no longer bound")))?;
    let bytes = route_pod(index, card_profile_device, None)
        .ok_or_else(|| Error::pipewire("could not build the Route pod"))?;
    let param =
        Pod::from_bytes(&bytes).ok_or_else(|| Error::pipewire("built an invalid Route pod"))?;
    device.proxy.set_param(ParamType::Route, 0, param);
    Ok(())
}

/// Write `channelVolumes` + `mute` through a card's active `Route`.
fn write_route_props(
    inner: &Inner,
    target: &RouteTarget,
    channel_volumes: Vec<f32>,
    mute: bool,
) -> Result<()> {
    let device = inner.devices.get(&target.device_id).ok_or_else(|| {
        Error::pipewire(format!("device {} is no longer bound", target.device_id))
    })?;
    let props = vec![
        Property::new(
            spa::sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(channel_volumes)),
        ),
        Property::new(spa::sys::SPA_PROP_mute, Value::Bool(mute)),
    ];
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
