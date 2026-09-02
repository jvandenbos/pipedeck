//! Generated client side of `dev.pipedeck.Daemon1`.
//!
//! Kept byte-for-byte in step with the daemon's interface and with
//! `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml`.

use pipedeckd::route::PortTuple;
use pipedeckd::state::{DeviceTuple, EqPresetTuple, EqTuple, StreamTuple};

#[zbus::proxy(
    interface = "dev.pipedeck.Daemon1",
    default_service = "dev.pipedeck.Daemon",
    default_path = "/dev/pipedeck/Daemon"
)]
pub trait Daemon {
    /// `(id, name, description, kind, is_default, virtual, volume, mute, nick)`.
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<DeviceTuple>>;

    /// `(node_id, route_index, name, description, available, active)`.
    #[zbus(property)]
    fn ports(&self) -> zbus::Result<Vec<PortTuple>>;

    /// `(id, app_name, binary, media_name, target_name, volume, mute)`.
    #[zbus(property)]
    fn streams(&self) -> zbus::Result<Vec<StreamTuple>>;

    /// Available EQ presets: `(id, name)`.
    #[zbus(property)]
    fn eq_presets(&self) -> zbus::Result<Vec<EqPresetTuple>>;

    /// The EQ preset on each output device: `(node_id, preset id or "")`.
    #[zbus(property)]
    fn eq(&self) -> zbus::Result<Vec<EqTuple>>;

    /// `node.name` of the notification sink, or "" to follow the default output.
    #[zbus(property)]
    fn notification_sink(&self) -> zbus::Result<String>;

    /// Daemon version.
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;

    /// Make `name` the default sink or source.
    fn set_default(&self, kind: &str, name: &str) -> zbus::Result<()>;

    /// Set the notification sink; "" follows the default output.
    fn set_notification_sink(&self, name: &str) -> zbus::Result<()>;

    /// Set the linear volume (0.0-3.375, i.e. 0-150 % cubic) of a device or stream node.
    fn set_volume(&self, id: u32, volume: f64) -> zbus::Result<()>;

    /// Mute or unmute a device or stream node.
    fn set_mute(&self, id: u32, mute: bool) -> zbus::Result<()>;

    /// Route a stream at a named sink; "" restores the default.
    fn set_stream_target(&self, id: u32, name: &str) -> zbus::Result<()>;

    /// Select a card route (port) for a sink or source node.
    fn set_port(&self, node_id: u32, route_index: u32) -> zbus::Result<()>;

    /// Apply an EQ preset to an output device; "" turns EQ off.
    fn set_eq(&self, node_id: u32, preset: &str) -> zbus::Result<()>;

    /// Re-read the graph.
    fn refresh(&self) -> zbus::Result<()>;

    /// Cheap "re-read the properties" nudge.
    #[zbus(signal)]
    fn changed(&self) -> zbus::Result<()>;
}
