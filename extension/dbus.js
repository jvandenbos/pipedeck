// D-Bus contract for dev.pipedeck.Daemon1, per SPEC.md §2.2.
//
// This is the extension's own copy of the introspection XML, kept here so the
// extension works standalone (before pipedeckd has ever started, and in the
// gjs -m syntax check, which has no D-Bus/PipeWire runtime at all). The
// daemon agent's canonical copy is expected at
// crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml — if that file exists, diff
// it against DaemonInterfaceXml below and reconcile any drift.

import Gio from 'gi://Gio';

export const BUS_NAME = 'dev.pipedeck.Daemon';
export const OBJECT_PATH = '/dev/pipedeck/Daemon';
export const INTERFACE_NAME = 'dev.pipedeck.Daemon1';

// Wire types, per SPEC §2.2, the §6.1 ports addendum, the §7.3 EQ addendum,
// the §8.1 auto-mute addendum, and a live-testing fix on chronos
// (2026-09-01) that added a trailing `nick` field to Devices:
//   Devices     a(usssbbdbs) (id, name, description, kind, is_default, virtual, volume, mute, nick)
//   Streams     a(ussssdb)   (id, app_name, binary, media_name, target_name, volume, mute)
//   Ports       a(uussbb)    (node_id, route_index, name, description, available, active)
//   EqPresets   a(ss)        (id, name) -- scanned from the presets dir
//   Eq          a(us)        (node_id, preset id or "") -- one row per output device
//   AutoMute    a(ub)        (node_id, enabled) -- one row per sink whose card has an
//                            Auto-Mute Mode ALSA control (SPEC §8.1)
// `unpackDevice` in extension.js destructures `nick` positionally and falls
// back to `description` when it's absent (an 8-field tuple from an older
// daemon build, or an empty string), so this is safe against either wire
// shape. `EqPresets`/`Eq`/`AutoMute` are absent entirely on an older daemon
// that predates them; the generated proxy just leaves those cached
// properties undefined, and extension.js treats that the same as "nothing
// to show" (relevant section/row hidden) rather than throwing.
export const DaemonInterfaceXml = `
<node>
  <interface name="dev.pipedeck.Daemon1">
    <property name="Devices" type="a(usssbbdbs)" access="read"/>
    <property name="Streams" type="a(ussssdb)" access="read"/>
    <property name="NotificationSink" type="s" access="read"/>
    <property name="Ports" type="a(uussbb)" access="read"/>
    <property name="EqPresets" type="a(ss)" access="read"/>
    <property name="Eq" type="a(us)" access="read"/>
    <property name="AutoMute" type="a(ub)" access="read"/>
    <property name="Version" type="s" access="read"/>

    <method name="SetDefault">
      <arg type="s" name="kind" direction="in"/>
      <arg type="s" name="name" direction="in"/>
    </method>
    <method name="SetNotificationSink">
      <arg type="s" name="name" direction="in"/>
    </method>
    <method name="SetVolume">
      <arg type="u" name="id" direction="in"/>
      <arg type="d" name="volume" direction="in"/>
    </method>
    <method name="SetMute">
      <arg type="u" name="id" direction="in"/>
      <arg type="b" name="mute" direction="in"/>
    </method>
    <method name="SetStreamTarget">
      <arg type="u" name="id" direction="in"/>
      <arg type="s" name="name" direction="in"/>
    </method>
    <method name="SetPort">
      <arg type="u" name="node_id" direction="in"/>
      <arg type="u" name="route_index" direction="in"/>
    </method>
    <method name="SetEq">
      <arg type="u" name="node_id" direction="in"/>
      <arg type="s" name="preset" direction="in"/>
    </method>
    <method name="SetAutoMute">
      <arg type="u" name="node_id" direction="in"/>
      <arg type="b" name="enabled" direction="in"/>
    </method>
    <method name="Refresh"/>

    <signal name="Changed"/>
  </interface>
</node>
`;

// Generated proxy class. Construct with:
//   new DaemonProxy(Gio.DBus.session, BUS_NAME, OBJECT_PATH, (proxy, error) => { ... })
// Properties (Devices, Streams, NotificationSink, Version) come back as plain JS
// values/arrays via the wrapper's cached-property getters (GVariant deep_unpack
// under the hood). Methods are exposed as both `<Name>Sync(...)` and
// `<Name>Remote(..., (result, error) => { ... })`; this extension always uses
// the Remote (async) form so a slow/wedged daemon can never block the Shell.
export const DaemonProxy = Gio.DBusProxy.makeProxyWrapper(DaemonInterfaceXml);
