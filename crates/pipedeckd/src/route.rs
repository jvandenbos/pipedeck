//! Ports — the ALSA card **routes** behind a sink or source (SPEC §6.1).
//!
//! On chronos the motherboard codec exposes one sink with two *ports*, "Line
//! Out" and "Headphones". Switching between them is a `Route` param write on the
//! card's `Audio/Device` global, not a different node — and on the same cards
//! WirePlumber owns volume through that same param, so writing the node's
//! `Props` is silently ignored.
//!
//! This module is the pure half of that: the route tables a device advertises,
//! the rule that decides which routes apply to which node, the active-route
//! lookup, and the `Ports` D-Bus projection. It holds no PipeWire types, so all
//! of it is unit-tested without a graph. Pod parsing and writing live in
//! [`crate::pw`], the only module that links against libpipewire.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use crate::state::DeviceKind;

/// Which side of the card a route sits on (`SPA_DIRECTION_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteDirection {
    /// `SPA_DIRECTION_INPUT` — capture, i.e. sources.
    Input,
    /// `SPA_DIRECTION_OUTPUT` — playback, i.e. sinks.
    Output,
}

impl RouteDirection {
    /// Raw value of `SPA_DIRECTION_INPUT`.
    pub const INPUT_RAW: u32 = 0;
    /// Raw value of `SPA_DIRECTION_OUTPUT`.
    pub const OUTPUT_RAW: u32 = 1;

    /// Parse a raw `spa_direction`.
    #[must_use]
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            Self::INPUT_RAW => Some(RouteDirection::Input),
            Self::OUTPUT_RAW => Some(RouteDirection::Output),
            _ => None,
        }
    }

    /// The raw `spa_direction` value.
    #[must_use]
    pub fn as_raw(self) -> u32 {
        match self {
            RouteDirection::Input => Self::INPUT_RAW,
            RouteDirection::Output => Self::OUTPUT_RAW,
        }
    }

    /// The direction a node of this kind is routed by: sinks take `Output`
    /// routes, sources take `Input` routes (SPEC §6).
    #[must_use]
    pub fn for_kind(kind: DeviceKind) -> Self {
        match kind {
            DeviceKind::Sink => RouteDirection::Output,
            DeviceKind::Source => RouteDirection::Input,
        }
    }

    /// Whether this direction is the one a node of `kind` uses.
    #[must_use]
    pub fn matches(self, kind: DeviceKind) -> bool {
        self == Self::for_kind(kind)
    }
}

/// `SPA_PARAM_AVAILABILITY_*` — whether the port is physically usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Availability {
    /// The driver cannot tell (common on HDMI and on many codecs).
    Unknown,
    /// Definitely not usable right now — nothing plugged in.
    No,
    /// Usable.
    Yes,
}

impl Availability {
    /// Parse a raw `spa_param_availability`.
    #[must_use]
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Availability::No,
            2 => Availability::Yes,
            _ => Availability::Unknown,
        }
    }

    /// The raw `spa_param_availability` value.
    #[must_use]
    pub fn as_raw(self) -> u32 {
        match self {
            Availability::Unknown => 0,
            Availability::No => 1,
            Availability::Yes => 2,
        }
    }

    /// What the `available` boolean of the `Ports` tuple carries.
    ///
    /// Only an explicit `no` counts as unavailable: `unknown` is what a codec
    /// reports when it has no jack-detection, and hiding those would hide most
    /// HDMI outputs.
    #[must_use]
    pub fn is_selectable(self) -> bool {
        !matches!(self, Availability::No)
    }
}

/// One entry of a device's `EnumRoute` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// `index` — the handle `SetPort` and the `Route` param use.
    pub index: u32,
    /// `direction`.
    pub direction: RouteDirection,
    /// `name`, e.g. `analog-output-headphones`.
    pub name: String,
    /// `description`, e.g. "Headphones".
    pub description: String,
    /// `priority`; higher wins when the session manager picks for itself.
    pub priority: u32,
    /// `available`.
    pub available: Availability,
    /// `devices` — the `card.profile.device` ids this route can drive.
    pub devices: Vec<i32>,
    /// `profiles` — the card profiles this route exists in.
    pub profiles: Vec<i32>,
}

impl Route {
    /// SPEC §6.1's rule: a route applies to a node when its `devices` list
    /// contains the node's `card.profile.device` **and** its direction matches
    /// the node kind.
    #[must_use]
    pub fn applies_to(&self, kind: DeviceKind, card_profile_device: i32) -> bool {
        self.direction.matches(kind) && self.devices.contains(&card_profile_device)
    }
}

/// The `props` sub-object of a `Route`: where an ALSA sink's real volume lives.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteProps {
    /// Loudest entry of `channelVolumes`, linear.
    pub volume: Option<f64>,
    /// `mute`.
    pub mute: Option<bool>,
    /// Length of `channelVolumes`, i.e. how many floats a write must send.
    pub channels: Option<usize>,
}

/// One entry of a device's `Route` (active) param, one per profile-device.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveRoute {
    /// `index` of the route that is currently selected.
    pub index: u32,
    /// `device` — the `card.profile.device` this route is active for.
    pub device: i32,
    /// The route's `props`, when it carried any.
    pub props: RouteProps,
}

/// Everything we track for one `Audio/Device` global.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceRoutes {
    /// `EnumRoute` entries by route index.
    pub enum_routes: BTreeMap<u32, Route>,
    /// Active `Route` entries by `card.profile.device`.
    pub active: BTreeMap<i32, ActiveRoute>,
}

impl DeviceRoutes {
    /// Look one route up by index.
    #[must_use]
    pub fn route(&self, index: u32) -> Option<&Route> {
        self.enum_routes.get(&index)
    }

    /// The active route for a profile-device, if the card reported one.
    #[must_use]
    pub fn active_for(&self, card_profile_device: i32) -> Option<&ActiveRoute> {
        self.active.get(&card_profile_device)
    }

    /// Every route that applies to a node, ordered by index.
    pub fn routes_for(
        &self,
        kind: DeviceKind,
        card_profile_device: i32,
    ) -> impl Iterator<Item = &Route> {
        self.enum_routes
            .values()
            .filter(move |r| r.applies_to(kind, card_profile_device))
    }

    /// The `Ports` rows for one node: one per applicable route, unavailable
    /// ones included (SPEC §6.1 — the panel hides them, the CLI dims them).
    #[must_use]
    pub fn ports_for(&self, node_id: u32, kind: DeviceKind, card_profile_device: i32) -> Vec<Port> {
        let active = self.active_for(card_profile_device).map(|a| a.index);
        self.routes_for(kind, card_profile_device)
            .map(|route| Port {
                node_id,
                index: route.index,
                name: route.name.clone(),
                description: route.description.clone(),
                available: route.available.is_selectable(),
                active: active == Some(route.index),
            })
            .collect()
    }
}

/// D-Bus tuple for one entry of the `Ports` property: `(uussbb)`.
pub type PortTuple = (u32, u32, String, String, bool, bool);

/// One selectable port of one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// Node the port belongs to — the same id `SetVolume` takes.
    pub node_id: u32,
    /// Route index; the second argument of `SetPort`.
    pub index: u32,
    /// Route `name`, e.g. `analog-output-headphones`.
    pub name: String,
    /// Route `description`, e.g. "Headphones".
    pub description: String,
    /// False only when the card said `available: no`.
    pub available: bool,
    /// True when this is the node's current port.
    pub active: bool,
}

impl Port {
    /// Project into the D-Bus tuple shape.
    #[must_use]
    pub fn to_dbus(&self) -> PortTuple {
        (
            self.node_id,
            self.index,
            self.name.clone(),
            self.description.clone(),
            self.available,
            self.active,
        )
    }
}

/// Why a `SetPort` request was rejected.
///
/// Every variant maps to `dev.pipedeck.Error.InvalidArgument`, per SPEC §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetPortError {
    /// The node's card advertises no routes at all.
    NoPorts,
    /// No route with that index exists on the card.
    UnknownRoute,
    /// The route exists but is for the other direction, or for a different
    /// profile-device than this node's.
    NotApplicable,
    /// The route exists and applies, but the card says `available: no`.
    Unavailable,
}

impl SetPortError {
    /// A message suitable for the D-Bus error body.
    #[must_use]
    pub fn message(self, node_id: u32, route_index: u32) -> String {
        match self {
            SetPortError::NoPorts => format!("node {node_id} has no ports"),
            SetPortError::UnknownRoute => {
                format!("node {node_id} has no port with index {route_index}")
            }
            SetPortError::NotApplicable => format!(
                "port {route_index} does not apply to node {node_id} \
                 (wrong direction or profile-device)"
            ),
            SetPortError::Unavailable => {
                format!("port {route_index} of node {node_id} is not available")
            }
        }
    }
}

impl std::fmt::Display for SetPortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SetPortError::NoPorts => "no ports",
            SetPortError::UnknownRoute => "unknown route",
            SetPortError::NotApplicable => "route does not apply to this node",
            SetPortError::Unavailable => "route is not available",
        })
    }
}

/// Validate a `SetPort` request against a card's route table.
///
/// # Errors
/// [`SetPortError`] when the route is unknown, inapplicable or unavailable —
/// SPEC §6.1 wants all three as `InvalidArgument`.
pub fn validate_set_port(
    routes: &DeviceRoutes,
    kind: DeviceKind,
    card_profile_device: i32,
    route_index: u32,
) -> Result<&Route, SetPortError> {
    if routes.enum_routes.is_empty() {
        return Err(SetPortError::NoPorts);
    }
    let route = routes
        .route(route_index)
        .ok_or(SetPortError::UnknownRoute)?;
    if !route.applies_to(kind, card_profile_device) {
        return Err(SetPortError::NotApplicable);
    }
    if !route.available.is_selectable() {
        return Err(SetPortError::Unavailable);
    }
    Ok(route)
}

// ---------------------------------------------------------------------------
// SPEC §9.2 — the port-switch level cap's pending state
// ---------------------------------------------------------------------------

/// How long a PipeDeck-initiated `SetPort` stays eligible for the level cap.
///
/// The card re-enumerates its `Route` param within a few tens of milliseconds,
/// but that first arrival is **not** the last word: on chronos (2026-09-06)
/// WirePlumber's own per-port restore reacted to the same Route change and
/// wrote the stored 82 % *after* our clamp, so a one-shot clamp was silently
/// overwritten. The entry therefore stays live for the whole window and every
/// arrival above the cap is re-clamped. Two seconds is long enough to outlast
/// the session manager and short enough that a switch which never took cannot
/// cap a level the user set by hand afterwards.
pub const PORT_SWITCH_CAP_WINDOW: Duration = Duration::from_secs(2);

/// One PipeDeck-initiated port switch, live until its window closes.
///
/// Recorded by the `SetPort` write and consulted by **every** `Route`
/// re-enumeration that shows the requested route active — see SPEC §9.2. It is
/// not consumed by the first clamp; only the window closing, a `SetVolume`/
/// `SetMute` for the node, or the next `SetPort` retires it.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingPortSwitch {
    /// Node the switch was requested for; also the key it is stored under.
    pub node_id: u32,
    /// `Audio/Device` global that owns the route.
    pub device_id: u32,
    /// `card.profile.device` the route must come back active for.
    pub card_profile_device: i32,
    /// Route index that was requested.
    pub route_index: u32,
    /// The ceiling to apply, **linear** (already through `cubic_to_linear`).
    pub cap_linear: f64,
    /// When the `SetPort` write went out.
    pub issued: Instant,
    /// How many clamp writes have already gone out for this switch, so a
    /// repeat can log `re-clamped` instead of `clamped`.
    pub clamps: u32,
}

impl PendingPortSwitch {
    /// Does a `Route` re-enumeration identify this switch?
    #[must_use]
    pub fn matches(&self, device_id: u32, card_profile_device: i32, route_index: u32) -> bool {
        self.device_id == device_id
            && self.card_profile_device == card_profile_device
            && self.route_index == route_index
    }

    /// Has the window closed on this switch?
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.issued) > PORT_SWITCH_CAP_WINDOW
    }
}

/// Port switches PipeDeck has issued whose window is still open.
///
/// At most one per node: a second `SetPort` for the same node replaces the
/// first, which is exactly SPEC §9.2's "or on the next `SetPort` for that
/// node". Time is passed in rather than read so the whole thing is testable
/// without sleeping.
#[derive(Debug, Default)]
pub struct PendingPortSwitches {
    entries: HashMap<u32, PendingPortSwitch>,
}

impl PendingPortSwitches {
    /// Record a switch, replacing any earlier one for the same node.
    ///
    /// Also sweeps anything whose window closed, so entries on a card nobody
    /// touches again cannot accumulate.
    pub fn record(&mut self, pending: PendingPortSwitch) {
        self.expire(pending.issued);
        self.entries.insert(pending.node_id, pending);
    }

    /// Forget any switch pending for a node.
    ///
    /// SPEC §9.2's early cancels: a `SetVolume`/`SetMute` for the node (the
    /// user is taking over), the next `SetPort` with the cap off, or the node
    /// going away.
    pub fn remove_node(&mut self, node_id: u32) {
        self.entries.remove(&node_id);
    }

    /// Drop everything whose window has closed.
    pub fn expire(&mut self, now: Instant) {
        self.entries.retain(|_, pending| !pending.is_expired(now));
    }

    /// The still-live switch a `Route` re-enumeration confirms, if any.
    ///
    /// Expired entries are dropped first, so a switch that never took cannot
    /// cap a level minutes later. The entry is **not** consumed: WirePlumber
    /// may write the port's stored level after ours, and every arrival inside
    /// the window has to be answered.
    pub fn matching(
        &mut self,
        device_id: u32,
        card_profile_device: i32,
        route_index: u32,
        now: Instant,
    ) -> Option<&PendingPortSwitch> {
        self.expire(now);
        self.entries
            .values()
            .find(|pending| pending.matches(device_id, card_profile_device, route_index))
    }

    /// Count a clamp write against a node's pending switch, returning how many
    /// have now gone out (`1` is the first, so anything higher logs as a
    /// re-clamp).
    pub fn note_clamp(&mut self, node_id: u32) -> u32 {
        match self.entries.get_mut(&node_id) {
            Some(pending) => {
                pending.clamps += 1;
                pending.clamps
            }
            None => 0,
        }
    }

    /// How many switches are outstanding.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is nothing outstanding?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(
        index: u32,
        direction: RouteDirection,
        name: &str,
        available: Availability,
        devices: &[i32],
    ) -> Route {
        Route {
            index,
            direction,
            name: name.to_owned(),
            description: format!("{name} description"),
            priority: 9000,
            available,
            devices: devices.to_vec(),
            profiles: vec![1],
        }
    }

    /// The chronos card from SPEC §6: two output routes on profile-device 4,
    /// one of which also drives 5, plus an input route on profile-device 0.
    fn chronos() -> DeviceRoutes {
        let mut routes = DeviceRoutes::default();
        routes.enum_routes.insert(
            3,
            route(
                3,
                RouteDirection::Output,
                "analog-output-lineout",
                Availability::Yes,
                &[4, 5],
            ),
        );
        routes.enum_routes.insert(
            4,
            route(
                4,
                RouteDirection::Output,
                "analog-output-headphones",
                Availability::Yes,
                &[4],
            ),
        );
        routes.enum_routes.insert(
            0,
            route(
                0,
                RouteDirection::Input,
                "analog-input-front-mic",
                Availability::Yes,
                &[0],
            ),
        );
        routes.enum_routes.insert(
            1,
            route(
                1,
                RouteDirection::Input,
                "analog-input-linein",
                Availability::No,
                &[0],
            ),
        );
        routes.active.insert(
            4,
            ActiveRoute {
                index: 4,
                device: 4,
                props: RouteProps {
                    volume: Some(0.064),
                    mute: Some(false),
                    channels: Some(2),
                },
            },
        );
        routes.active.insert(
            0,
            ActiveRoute {
                index: 0,
                device: 0,
                props: RouteProps::default(),
            },
        );
        routes
    }

    #[test]
    fn direction_maps_to_the_node_kind() {
        assert_eq!(
            RouteDirection::for_kind(DeviceKind::Sink),
            RouteDirection::Output
        );
        assert_eq!(
            RouteDirection::for_kind(DeviceKind::Source),
            RouteDirection::Input
        );
        assert!(RouteDirection::Output.matches(DeviceKind::Sink));
        assert!(!RouteDirection::Output.matches(DeviceKind::Source));
        assert_eq!(RouteDirection::from_raw(0), Some(RouteDirection::Input));
        assert_eq!(RouteDirection::from_raw(1), Some(RouteDirection::Output));
        assert_eq!(RouteDirection::from_raw(7), None);
        assert_eq!(RouteDirection::Output.as_raw(), 1);
    }

    #[test]
    fn availability_only_hides_an_explicit_no() {
        assert_eq!(Availability::from_raw(0), Availability::Unknown);
        assert_eq!(Availability::from_raw(1), Availability::No);
        assert_eq!(Availability::from_raw(2), Availability::Yes);
        assert_eq!(Availability::from_raw(99), Availability::Unknown);
        assert!(Availability::Unknown.is_selectable());
        assert!(Availability::Yes.is_selectable());
        assert!(!Availability::No.is_selectable());
        assert_eq!(Availability::No.as_raw(), 1);
    }

    #[test]
    fn route_applies_only_on_matching_direction_and_profile_device() {
        let routes = chronos();
        let lineout = routes.route(3).expect("route 3");
        let headphones = routes.route(4).expect("route 4");
        let mic = routes.route(0).expect("route 0");

        assert!(lineout.applies_to(DeviceKind::Sink, 4));
        assert!(lineout.applies_to(DeviceKind::Sink, 5));
        // Right direction, wrong profile-device.
        assert!(!headphones.applies_to(DeviceKind::Sink, 5));
        // Right profile-device, wrong direction.
        assert!(!lineout.applies_to(DeviceKind::Source, 4));
        assert!(mic.applies_to(DeviceKind::Source, 0));
        assert!(!mic.applies_to(DeviceKind::Sink, 0));
    }

    #[test]
    fn routes_for_a_node_are_filtered_and_ordered_by_index() {
        let routes = chronos();
        let indices: Vec<u32> = routes
            .routes_for(DeviceKind::Sink, 4)
            .map(|r| r.index)
            .collect();
        assert_eq!(indices, vec![3, 4]);

        let inputs: Vec<u32> = routes
            .routes_for(DeviceKind::Source, 0)
            .map(|r| r.index)
            .collect();
        assert_eq!(inputs, vec![0, 1]);

        assert_eq!(routes.routes_for(DeviceKind::Sink, 99).count(), 0);
    }

    #[test]
    fn active_route_resolution_is_per_profile_device() {
        let routes = chronos();
        assert_eq!(routes.active_for(4).map(|a| a.index), Some(4));
        assert_eq!(routes.active_for(0).map(|a| a.index), Some(0));
        assert!(routes.active_for(5).is_none());
        let active = routes.active_for(4).expect("active");
        assert_eq!(active.props.channels, Some(2));
        assert!((active.props.volume.expect("volume") - 0.064).abs() < 1e-9);
    }

    #[test]
    fn ports_mark_exactly_the_active_route() {
        let ports = chronos().ports_for(39, DeviceKind::Sink, 4);
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].index, 3);
        assert!(!ports[0].active);
        assert!(ports[0].available);
        assert_eq!(ports[1].index, 4);
        assert!(ports[1].active);
        assert_eq!(ports[1].name, "analog-output-headphones");
        assert!(ports.iter().all(|p| p.node_id == 39));
    }

    #[test]
    fn unavailable_ports_are_still_listed() {
        let ports = chronos().ports_for(41, DeviceKind::Source, 0);
        assert_eq!(ports.len(), 2);
        let linein = ports.iter().find(|p| p.index == 1).expect("linein row");
        assert!(!linein.available);
        assert!(!linein.active);
    }

    #[test]
    fn a_node_with_no_active_route_has_no_active_port() {
        let mut routes = chronos();
        routes.active.remove(&4);
        let ports = routes.ports_for(39, DeviceKind::Sink, 4);
        assert_eq!(ports.len(), 2);
        assert!(ports.iter().all(|p| !p.active));
    }

    #[test]
    fn port_tuple_matches_signature_order() {
        let port = Port {
            node_id: 39,
            index: 4,
            name: "analog-output-headphones".to_owned(),
            description: "Headphones".to_owned(),
            available: true,
            active: true,
        };
        let (node_id, index, name, description, available, active) = port.to_dbus();
        assert_eq!(node_id, 39);
        assert_eq!(index, 4);
        assert_eq!(name, "analog-output-headphones");
        assert_eq!(description, "Headphones");
        assert!(available);
        assert!(active);
    }

    #[test]
    fn set_port_accepts_an_applicable_available_route() {
        let routes = chronos();
        let route = validate_set_port(&routes, DeviceKind::Sink, 4, 3).expect("valid");
        assert_eq!(route.name, "analog-output-lineout");
    }

    #[test]
    fn set_port_rejects_unknown_inapplicable_and_unavailable_routes() {
        let routes = chronos();
        assert_eq!(
            validate_set_port(&routes, DeviceKind::Sink, 4, 77).unwrap_err(),
            SetPortError::UnknownRoute
        );
        // Input route on a sink.
        assert_eq!(
            validate_set_port(&routes, DeviceKind::Sink, 4, 0).unwrap_err(),
            SetPortError::NotApplicable
        );
        // Right direction, wrong profile-device.
        assert_eq!(
            validate_set_port(&routes, DeviceKind::Sink, 5, 4).unwrap_err(),
            SetPortError::NotApplicable
        );
        assert_eq!(
            validate_set_port(&routes, DeviceKind::Source, 0, 1).unwrap_err(),
            SetPortError::Unavailable
        );
        assert_eq!(
            validate_set_port(&DeviceRoutes::default(), DeviceKind::Sink, 4, 3).unwrap_err(),
            SetPortError::NoPorts
        );
    }

    #[test]
    fn set_port_error_messages_name_the_node_and_route() {
        assert!(SetPortError::NoPorts.message(39, 4).contains("39"));
        assert!(SetPortError::UnknownRoute.message(39, 4).contains("4"));
        assert!(SetPortError::NotApplicable
            .message(39, 4)
            .contains("direction"));
        assert!(SetPortError::Unavailable
            .message(39, 4)
            .contains("not available"));
        assert_eq!(SetPortError::NoPorts.to_string(), "no ports");
    }

    // -- SPEC §9.2: the pending-switch bookkeeping ---------------------------

    fn pending(node_id: u32, route_index: u32, issued: Instant) -> PendingPortSwitch {
        PendingPortSwitch {
            node_id,
            device_id: 53,
            card_profile_device: 4,
            route_index,
            cap_linear: 0.216,
            issued,
            clamps: 0,
        }
    }

    /// Only the re-enumeration that actually shows the requested route active
    /// for this card's profile-device matches the pending entry.
    #[test]
    fn pending_switch_is_matched_by_its_own_route() {
        let now = Instant::now();
        let mut switches = PendingPortSwitches::default();
        assert!(switches.is_empty());
        switches.record(pending(39, 3, now));
        assert_eq!(switches.len(), 1);

        // Wrong route index, wrong profile-device, wrong card: all leave it be.
        assert!(switches.matching(53, 4, 4, now).is_none());
        assert!(switches.matching(53, 5, 3, now).is_none());
        assert!(switches.matching(54, 4, 3, now).is_none());
        assert_eq!(switches.len(), 1);

        let found = switches.matching(53, 4, 3, now).expect("matches");
        assert_eq!(found.node_id, 39);
        assert_eq!(found.clamps, 0);
        assert!((found.cap_linear - 0.216).abs() < 1e-9);
    }

    /// The chronos 2026-09-06 finding: WirePlumber's per-port restore lands
    /// *after* our clamp, so the entry must survive the first clamp and answer
    /// every later arrival inside the window.
    #[test]
    fn a_second_arrival_inside_the_window_clamps_again() {
        let issued = Instant::now();
        let mut switches = PendingPortSwitches::default();
        switches.record(pending(39, 3, issued));

        // First arrival: matched, and the clamp is counted as the first one.
        assert!(switches.matching(53, 4, 3, issued).is_some());
        assert_eq!(switches.note_clamp(39), 1);
        assert_eq!(switches.len(), 1, "the entry survives its own clamp");

        // WirePlumber writes the stored level back a moment later: the same
        // entry matches again, and the clamp logs as a repeat.
        let later = issued + Duration::from_millis(400);
        let again = switches.matching(53, 4, 3, later).expect("still live");
        assert_eq!(again.clamps, 1, "a repeat, so the journal says re-clamped");
        assert_eq!(switches.note_clamp(39), 2);

        // ... and again, right up to the edge of the window.
        let edge = issued + PORT_SWITCH_CAP_WINDOW;
        assert!(switches.matching(53, 4, 3, edge).is_some());

        // Once the window closes it stops answering, however loud the card is.
        let late = issued + PORT_SWITCH_CAP_WINDOW + Duration::from_millis(1);
        assert!(switches.matching(53, 4, 3, late).is_none());
        assert!(switches.is_empty());
        // Counting a clamp against a retired entry is a no-op, not a panic.
        assert_eq!(switches.note_clamp(39), 0);
    }

    /// A switch that never took must not cap a level set by hand later.
    #[test]
    fn pending_switch_expires_after_the_window() {
        let issued = Instant::now();
        let mut switches = PendingPortSwitches::default();
        switches.record(pending(39, 3, issued));

        // Still inside the window (and the boundary itself counts as inside).
        let inside = issued + PORT_SWITCH_CAP_WINDOW;
        assert!(!pending(39, 3, issued).is_expired(inside));

        let late = issued + PORT_SWITCH_CAP_WINDOW + Duration::from_millis(1);
        assert!(pending(39, 3, issued).is_expired(late));
        assert!(switches.matching(53, 4, 3, late).is_none());
        assert!(switches.is_empty(), "the expired entry was dropped");

        // `expire` alone sweeps without a matching re-enumeration.
        switches.record(pending(39, 3, issued));
        switches.expire(inside);
        assert_eq!(switches.len(), 1);
        switches.expire(late);
        assert!(switches.is_empty());

        // `record` sweeps too, so an entry on a card nobody touches again does
        // not sit in the map forever.
        switches.record(pending(39, 3, issued));
        let mut other = pending(41, 5, issued);
        other.card_profile_device = 5;
        switches.record(other);
        assert_eq!(switches.len(), 2);
        switches.record(pending(43, 7, late));
        assert_eq!(switches.len(), 1, "the two stale entries were swept");
    }

    /// SPEC §9.2's early cancels: the next `SetPort` for a node replaces its
    /// entry, and `SetVolume`/`SetMute` (via `remove_node`) retires it.
    #[test]
    fn a_second_switch_or_a_manual_level_retires_the_entry() {
        let now = Instant::now();
        let mut switches = PendingPortSwitches::default();
        switches.record(pending(39, 3, now));
        switches.record(pending(39, 4, now));
        assert_eq!(switches.len(), 1);
        assert!(switches.matching(53, 4, 3, now).is_none());
        assert_eq!(
            switches
                .matching(53, 4, 4, now)
                .expect("the newer switch")
                .route_index,
            4
        );

        // Different nodes on the same card coexist.
        switches.record(pending(39, 3, now));
        let mut other = pending(41, 5, now);
        other.card_profile_device = 5;
        switches.record(other);
        assert_eq!(switches.len(), 2);

        // The user taking over one node leaves the other alone.
        switches.remove_node(41);
        assert_eq!(switches.len(), 1);
        assert!(switches.matching(53, 4, 3, now).is_some());
        switches.remove_node(39);
        assert!(switches.is_empty());
        assert!(switches.matching(53, 4, 3, now).is_none());
    }
}
