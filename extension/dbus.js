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

// Wire types, per SPEC §2.2:
//   Devices   a(usssbbdb)  (id, name, description, kind, is_default, virtual, volume, mute)
//   Streams   a(ussssdb)   (id, app_name, binary, media_name, target_name, volume, mute)
export const DaemonInterfaceXml = `
<node>
  <interface name="dev.pipedeck.Daemon1">
    <property name="Devices" type="a(usssbbdb)" access="read"/>
    <property name="Streams" type="a(ussssdb)" access="read"/>
    <property name="NotificationSink" type="s" access="read"/>
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
