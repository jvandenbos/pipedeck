//! The graph snapshot the daemon publishes: pure data, no PipeWire types.

use std::collections::BTreeMap;
use std::fmt;

use crate::route::{Port, PortTuple};

/// Which side of the graph a device sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceKind {
    /// `media.class = Audio/Sink` — an output.
    Sink,
    /// `media.class = Audio/Source` — an input.
    Source,
}

impl DeviceKind {
    /// The wire representation used on D-Bus and by the CLI.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Sink => "sink",
            DeviceKind::Source => "source",
        }
    }

    /// Parse the D-Bus / CLI spelling. Accepts a couple of friendly aliases.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sink" | "output" => Some(DeviceKind::Sink),
            "source" | "input" => Some(DeviceKind::Source),
            _ => None,
        }
    }
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// D-Bus tuple for one entry of the `Devices` property: `(usssbbdbs)`.
pub type DeviceTuple = (u32, String, String, String, bool, bool, f64, bool, String);
/// D-Bus tuple for one entry of the `Streams` property: `(ussssdb)`.
pub type StreamTuple = (u32, String, String, String, String, f64, bool);

/// A sink or source node.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// PipeWire global (node) id. Not stable across sessions.
    pub id: u32,
    /// `node.name` — stable, and what config and metadata refer to.
    pub name: String,
    /// `node.description` — the human label.
    pub description: String,
    /// Sink or source.
    pub kind: DeviceKind,
    /// True when this node is the effective default for its kind.
    pub is_default: bool,
    /// True for null sinks, filter chains and other non-hardware nodes.
    pub virtual_: bool,
    /// Linear volume, 0.0–3.375 (150 % cubic).
    pub volume: f64,
    /// Mute flag.
    pub mute: bool,
    /// `node.nick` — the short label, falling back to `description`. ALSA
    /// descriptions ("Starship/Matisse HD Audio Controller Analog Stereo")
    /// truncate in the panel; the nick ("ALC892 Analog") does not.
    pub nick: String,
}

impl Device {
    /// Project into the D-Bus tuple shape.
    #[must_use]
    pub fn to_dbus(&self) -> DeviceTuple {
        (
            self.id,
            self.name.clone(),
            self.description.clone(),
            self.kind.as_str().to_owned(),
            self.is_default,
            self.virtual_,
            self.volume,
            self.mute,
            self.nick.clone(),
        )
    }
}

/// A playback (or capture) stream node.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    /// PipeWire global (node) id.
    pub id: u32,
    /// `application.name`.
    pub app_name: String,
    /// `application.process.binary`.
    pub binary: String,
    /// `media.name` — usually the track or tab title.
    pub media_name: String,
    /// `media.role`, kept for notification matching (not on the wire).
    pub role: String,
    /// `node.name` of the sink this stream is routed to, or "" when unknown.
    pub target_name: String,
    /// Linear volume, 0.0–3.375 (150 % cubic).
    pub volume: f64,
    /// Mute flag.
    pub mute: bool,
    /// False for `Stream/Input/Audio` (capture) nodes.
    pub playback: bool,
}

impl Stream {
    /// Project into the D-Bus tuple shape.
    #[must_use]
    pub fn to_dbus(&self) -> StreamTuple {
        (
            self.id,
            self.app_name.clone(),
            self.binary.clone(),
            self.media_name.clone(),
            self.target_name.clone(),
            self.volume,
            self.mute,
        )
    }
}

/// Everything the D-Bus layer serves, written only by the PipeWire thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    /// Sinks and sources by node id.
    pub devices: BTreeMap<u32, Device>,
    /// Streams by node id.
    pub streams: BTreeMap<u32, Stream>,
    /// Selectable ports, ordered by `(node id, route index)` (SPEC §6.1).
    pub ports: Vec<Port>,
    /// Effective default sink `node.name`.
    pub default_sink: Option<String>,
    /// Effective default source `node.name`.
    pub default_source: Option<String>,
    /// Configured notification sink `node.name`; empty = follow default output.
    pub notification_sink: String,
    /// True once the initial registry sync has completed.
    pub connected: bool,
}

impl State {
    /// `Devices` property payload, ordered by node id.
    #[must_use]
    pub fn devices_dbus(&self) -> Vec<DeviceTuple> {
        self.devices.values().map(Device::to_dbus).collect()
    }

    /// `Streams` property payload — playback streams only, per SPEC §2.1.
    #[must_use]
    pub fn streams_dbus(&self) -> Vec<StreamTuple> {
        self.streams
            .values()
            .filter(|s| s.playback)
            .map(Stream::to_dbus)
            .collect()
    }

    /// `Ports` property payload, ordered by node id then route index.
    #[must_use]
    pub fn ports_dbus(&self) -> Vec<PortTuple> {
        self.ports.iter().map(Port::to_dbus).collect()
    }

    /// Ports belonging to one node, in listing order.
    pub fn ports_of(&self, node_id: u32) -> impl Iterator<Item = &Port> {
        self.ports.iter().filter(move |p| p.node_id == node_id)
    }

    /// The active port of a node, when it has one.
    #[must_use]
    pub fn active_port(&self, node_id: u32) -> Option<&Port> {
        self.ports_of(node_id).find(|p| p.active)
    }

    /// Look up a device by `node.name`.
    #[must_use]
    pub fn device_by_name(&self, name: &str) -> Option<&Device> {
        self.devices.values().find(|d| d.name == name)
    }

    /// Look up a device of a given kind by `node.name`.
    #[must_use]
    pub fn device_by_name_of_kind(&self, name: &str, kind: DeviceKind) -> Option<&Device> {
        self.devices
            .values()
            .find(|d| d.name == name && d.kind == kind)
    }

    /// Devices of one kind, ordered by node id.
    pub fn devices_of_kind(&self, kind: DeviceKind) -> impl Iterator<Item = &Device> {
        self.devices.values().filter(move |d| d.kind == kind)
    }

    /// Recompute `is_default` from the effective defaults.
    pub fn refresh_defaults(&mut self) {
        for device in self.devices.values_mut() {
            let expected = match device.kind {
                DeviceKind::Sink => self.default_sink.as_deref(),
                DeviceKind::Source => self.default_source.as_deref(),
            };
            device.is_default = expected == Some(device.name.as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: u32, name: &str, kind: DeviceKind) -> Device {
        Device {
            id,
            name: name.to_owned(),
            description: format!("{name} description"),
            kind,
            is_default: false,
            virtual_: false,
            volume: 0.5,
            mute: false,
            nick: format!("{name} nick"),
        }
    }

    fn stream(id: u32, app: &str, playback: bool) -> Stream {
        Stream {
            id,
            app_name: app.to_owned(),
            binary: "firefox".to_owned(),
            media_name: "Playback".to_owned(),
            role: String::new(),
            target_name: "sink-a".to_owned(),
            volume: 1.0,
            mute: false,
            playback,
        }
    }

    #[test]
    fn kind_parsing_and_display() {
        assert_eq!(DeviceKind::parse("sink"), Some(DeviceKind::Sink));
        assert_eq!(DeviceKind::parse("  SOURCE "), Some(DeviceKind::Source));
        assert_eq!(DeviceKind::parse("output"), Some(DeviceKind::Sink));
        assert_eq!(DeviceKind::parse("input"), Some(DeviceKind::Source));
        assert_eq!(DeviceKind::parse("nonsense"), None);
        assert_eq!(DeviceKind::Sink.to_string(), "sink");
    }

    #[test]
    fn device_tuple_matches_signature_order() {
        let mut d = device(42, "sink-a", DeviceKind::Sink);
        d.is_default = true;
        d.virtual_ = true;
        d.mute = true;
        d.volume = 1.25;
        let (id, name, description, kind, is_default, virtual_, volume, mute, nick) = d.to_dbus();
        assert_eq!(id, 42);
        assert_eq!(name, "sink-a");
        assert_eq!(description, "sink-a description");
        assert_eq!(kind, "sink");
        assert!(is_default);
        assert!(virtual_);
        assert!((volume - 1.25).abs() < f64::EPSILON);
        assert!(mute);
        assert_eq!(nick, "sink-a nick");
    }

    #[test]
    fn ports_project_and_index_by_node() {
        use crate::route::Port;

        let state = State {
            ports: vec![
                Port {
                    node_id: 39,
                    index: 3,
                    name: "analog-output-lineout".to_owned(),
                    description: "Line Out".to_owned(),
                    available: true,
                    active: false,
                },
                Port {
                    node_id: 39,
                    index: 4,
                    name: "analog-output-headphones".to_owned(),
                    description: "Headphones".to_owned(),
                    available: true,
                    active: true,
                },
                Port {
                    node_id: 41,
                    index: 0,
                    name: "analog-input-front-mic".to_owned(),
                    description: "Front Microphone".to_owned(),
                    available: true,
                    active: true,
                },
            ],
            ..State::default()
        };
        let tuples = state.ports_dbus();
        assert_eq!(tuples.len(), 3);
        assert_eq!(
            tuples[0],
            (
                39,
                3,
                "analog-output-lineout".to_owned(),
                "Line Out".to_owned(),
                true,
                false
            )
        );
        assert_eq!(state.ports_of(39).count(), 2);
        assert_eq!(state.active_port(39).map(|p| p.index), Some(4));
        assert_eq!(state.active_port(99), None);
    }

    #[test]
    fn stream_tuple_matches_signature_order() {
        let s = stream(7, "Firefox", true);
        let (id, app, binary, media, target, volume, mute) = s.to_dbus();
        assert_eq!(id, 7);
        assert_eq!(app, "Firefox");
        assert_eq!(binary, "firefox");
        assert_eq!(media, "Playback");
        assert_eq!(target, "sink-a");
        assert!((volume - 1.0).abs() < f64::EPSILON);
        assert!(!mute);
    }

    #[test]
    fn streams_property_hides_capture_streams() {
        let mut state = State::default();
        state.streams.insert(1, stream(1, "Firefox", true));
        state.streams.insert(2, stream(2, "OBS", false));
        let listed = state.streams_dbus();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, 1);
    }

    #[test]
    fn refresh_defaults_marks_exactly_one_per_kind() {
        let mut state = State::default();
        state
            .devices
            .insert(1, device(1, "sink-a", DeviceKind::Sink));
        state
            .devices
            .insert(2, device(2, "sink-b", DeviceKind::Sink));
        state
            .devices
            .insert(3, device(3, "source-a", DeviceKind::Source));
        state.default_sink = Some("sink-b".to_owned());
        state.default_source = Some("source-a".to_owned());
        state.refresh_defaults();

        assert!(!state.devices[&1].is_default);
        assert!(state.devices[&2].is_default);
        assert!(state.devices[&3].is_default);

        state.default_sink = None;
        state.refresh_defaults();
        assert!(!state.devices[&2].is_default);
    }

    #[test]
    fn lookup_helpers() {
        let mut state = State::default();
        state
            .devices
            .insert(1, device(1, "shared", DeviceKind::Sink));
        state
            .devices
            .insert(2, device(2, "shared", DeviceKind::Source));
        assert_eq!(state.device_by_name("shared").map(|d| d.id), Some(1));
        assert_eq!(
            state
                .device_by_name_of_kind("shared", DeviceKind::Source)
                .map(|d| d.id),
            Some(2)
        );
        assert_eq!(state.devices_of_kind(DeviceKind::Sink).count(), 1);
        assert!(state.device_by_name("missing").is_none());
    }
}
