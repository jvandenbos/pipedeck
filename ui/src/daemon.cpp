#include "daemon.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusVariant>

// Stream operators. Field order is the wire order from the XML; a mismatch
// here is exactly what the fake-daemon round-trip tests catch.

QDBusArgument &operator<<(QDBusArgument &a, const DeviceRow &r) {
    a.beginStructure();
    a << r.id << r.name << r.description << r.kind << r.isDefault << r.isVirtual << r.volume << r.mute << r.nick;
    a.endStructure();
    return a;
}
const QDBusArgument &operator>>(const QDBusArgument &a, DeviceRow &r) {
    a.beginStructure();
    a >> r.id >> r.name >> r.description >> r.kind >> r.isDefault >> r.isVirtual >> r.volume >> r.mute >> r.nick;
    a.endStructure();
    return a;
}
QDBusArgument &operator<<(QDBusArgument &a, const StreamRow &r) {
    a.beginStructure();
    a << r.id << r.appName << r.binary << r.mediaName << r.targetName << r.volume << r.mute;
    a.endStructure();
    return a;
}
const QDBusArgument &operator>>(const QDBusArgument &a, StreamRow &r) {
    a.beginStructure();
    a >> r.id >> r.appName >> r.binary >> r.mediaName >> r.targetName >> r.volume >> r.mute;
    a.endStructure();
    return a;
}
QDBusArgument &operator<<(QDBusArgument &a, const PortRow &r) {
    a.beginStructure();
    a << r.nodeId << r.routeIndex << r.name << r.description << r.available << r.active;
    a.endStructure();
    return a;
}
const QDBusArgument &operator>>(const QDBusArgument &a, PortRow &r) {
    a.beginStructure();
    a >> r.nodeId >> r.routeIndex >> r.name >> r.description >> r.available >> r.active;
    a.endStructure();
    return a;
}
QDBusArgument &operator<<(QDBusArgument &a, const EqRow &r) {
    a.beginStructure();
    a << r.nodeId << r.preset;
    a.endStructure();
    return a;
}
const QDBusArgument &operator>>(const QDBusArgument &a, EqRow &r) {
    a.beginStructure();
    a >> r.nodeId >> r.preset;
    a.endStructure();
    return a;
}
QDBusArgument &operator<<(QDBusArgument &a, const PresetRow &r) {
    a.beginStructure();
    a << r.id << r.name;
    a.endStructure();
    return a;
}
const QDBusArgument &operator>>(const QDBusArgument &a, PresetRow &r) {
    a.beginStructure();
    a >> r.id >> r.name;
    a.endStructure();
    return a;
}
QDBusArgument &operator<<(QDBusArgument &a, const AutoMuteRow &r) {
    a.beginStructure();
    a << r.nodeId << r.enabled;
    a.endStructure();
    return a;
}
const QDBusArgument &operator>>(const QDBusArgument &a, AutoMuteRow &r) {
    a.beginStructure();
    a >> r.nodeId >> r.enabled;
    a.endStructure();
    return a;
}
QDBusArgument &operator<<(QDBusArgument &a, const StreamSinkRow &r) {
    a.beginStructure();
    a << r.streamId << r.sink;
    a.endStructure();
    return a;
}
const QDBusArgument &operator>>(const QDBusArgument &a, StreamSinkRow &r) {
    a.beginStructure();
    a >> r.streamId >> r.sink;
    a.endStructure();
    return a;
}

const QList<DaemonProxy::Member> &DaemonProxy::members() {
    static const QList<Member> list{
        {Kind::Method, "SetDefault", "ss"},
        {Kind::Method, "SetNotificationSink", "s"},
        {Kind::Method, "SetVolume", "ud"},
        {Kind::Method, "SetMute", "ub"},
        {Kind::Method, "SetStreamTarget", "us"},
        {Kind::Method, "SetPort", "uu"},
        {Kind::Method, "SetAutoMute", "ub"},
        {Kind::Method, "SetEq", "us"},
        {Kind::Method, "SetPortSwitchCap", "u"},
        {Kind::Method, "Refresh", ""},
        {Kind::Signal, "Changed", ""},
        {Kind::Property, "AutoMute", "a(ub)"},
        {Kind::Property, "Devices", "a(usssbbdbs)"},
        {Kind::Property, "Eq", "a(us)"},
        {Kind::Property, "EqPresets", "a(ss)"},
        {Kind::Property, "NotificationSink", "s"},
        {Kind::Property, "PortSwitchCap", "u"},
        {Kind::Property, "Ports", "a(uussbb)"},
        // Optional at runtime: added with SPEC-UI §6.1, absent on 1.3.0.
        {Kind::Property, "StreamSinks", "a(us)"},
        {Kind::Property, "Streams", "a(ussssdb)"},
        {Kind::Property, "Version", "s"},
    };
    return list;
}

void DaemonProxy::registerTypes() {
    static bool done = false;
    if (done)
        return;
    done = true;
    qDBusRegisterMetaType<DeviceRow>();
    qDBusRegisterMetaType<QList<DeviceRow>>();
    qDBusRegisterMetaType<StreamRow>();
    qDBusRegisterMetaType<QList<StreamRow>>();
    qDBusRegisterMetaType<PortRow>();
    qDBusRegisterMetaType<QList<PortRow>>();
    qDBusRegisterMetaType<EqRow>();
    qDBusRegisterMetaType<QList<EqRow>>();
    qDBusRegisterMetaType<PresetRow>();
    qDBusRegisterMetaType<QList<PresetRow>>();
    qDBusRegisterMetaType<AutoMuteRow>();
    qDBusRegisterMetaType<QList<AutoMuteRow>>();
    qDBusRegisterMetaType<StreamSinkRow>();
    qDBusRegisterMetaType<QList<StreamSinkRow>>();
}

DaemonProxy::DaemonProxy(const QDBusConnection &connection, QObject *parent)
    : QDBusAbstractInterface(Service, Path, Interface, connection, parent) {
    registerTypes();
    // Subscribed here, in the constructor, so the match rule reaches the bus
    // before the first GetAll does (SPEC-UI §5): both go out on this
    // connection in order, so nothing emitted after the snapshot is missed.
    QDBusConnection(connection).connect(Service, Path, "org.freedesktop.DBus.Properties", "PropertiesChanged", this,
                                        SLOT(onPropertiesChanged(QString, QVariantMap, QStringList)));
}

void DaemonProxy::onPropertiesChanged(const QString &interface, const QVariantMap &changed,
                                      const QStringList &invalidated) {
    if (interface != QLatin1String(Interface))
        return;
    emit propertiesChanged(changed.keys() + invalidated);
}

QDBusPendingCall DaemonProxy::getAll() const {
    auto message = QDBusMessage::createMethodCall(Service, Path, "org.freedesktop.DBus.Properties", "GetAll");
    message << QString::fromLatin1(Interface);
    return connection().asyncCall(message);
}

QDBusPendingCall DaemonProxy::setDefault(const QString &kind, const QString &name) {
    return asyncCall("SetDefault", kind, name);
}
QDBusPendingCall DaemonProxy::setNotificationSink(const QString &name) {
    return asyncCall("SetNotificationSink", name);
}
QDBusPendingCall DaemonProxy::setVolume(uint id, double linearVolume) {
    // (u, d) and nothing else: a volume write never carries a mute
    // (SPEC §9.1), and the signature makes that structural.
    return asyncCall("SetVolume", id, linearVolume);
}
QDBusPendingCall DaemonProxy::setMute(uint id, bool mute) {
    return asyncCall("SetMute", id, mute);
}
QDBusPendingCall DaemonProxy::setStreamTarget(uint id, const QString &name) {
    return asyncCall("SetStreamTarget", id, name);
}
QDBusPendingCall DaemonProxy::setPort(uint nodeId, uint routeIndex) {
    return asyncCall("SetPort", nodeId, routeIndex);
}
QDBusPendingCall DaemonProxy::setAutoMute(uint nodeId, bool enabled) {
    return asyncCall("SetAutoMute", nodeId, enabled);
}
QDBusPendingCall DaemonProxy::setEq(uint nodeId, const QString &preset) {
    return asyncCall("SetEq", nodeId, preset);
}
QDBusPendingCall DaemonProxy::setPortSwitchCap(uint percent) {
    return asyncCall("SetPortSwitchCap", percent);
}
QDBusPendingCall DaemonProxy::refresh() {
    return asyncCall("Refresh");
}

namespace {

// A struct-array value arrives as a QDBusArgument inside the variant; its
// signature is checked before streaming so a changed type reads as "absent".
template <typename T>
bool take(const QVariantMap &map, const QString &name, const QString &signature, T &out) {
    const auto it = map.constFind(name);
    if (it == map.constEnd())
        return false;
    QVariant value = *it;
    if (value.metaType() == QMetaType::fromType<QDBusVariant>())
        value = value.value<QDBusVariant>().variant();
    if (value.metaType() == QMetaType::fromType<QDBusArgument>()) {
        const auto argument = value.value<QDBusArgument>();
        if (argument.currentSignature() != signature)
            return false;
        argument >> out;
        return true;
    }
    if (value.metaType() == QMetaType::fromType<T>()) {
        out = value.value<T>();
        return true;
    }
    return false;
}

} // namespace

Snapshot DaemonProxy::decode(const QVariantMap &map) {
    Snapshot s;
    auto mark = [&](const QString &name, bool ok) {
        if (ok)
            s.present << name;
    };
    mark("Devices", take(map, "Devices", "a(usssbbdbs)", s.devices));
    mark("Streams", take(map, "Streams", "a(ussssdb)", s.streams));
    mark("Ports", take(map, "Ports", "a(uussbb)", s.ports));
    mark("Eq", take(map, "Eq", "a(us)", s.eq));
    mark("EqPresets", take(map, "EqPresets", "a(ss)", s.eqPresets));
    mark("AutoMute", take(map, "AutoMute", "a(ub)", s.autoMute));
    mark("StreamSinks", take(map, "StreamSinks", "a(us)", s.streamSinks));
    mark("NotificationSink", take(map, "NotificationSink", "s", s.notificationSink));
    mark("PortSwitchCap", take(map, "PortSwitchCap", "u", s.portSwitchCap));
    mark("Version", take(map, "Version", "s", s.version));
    return s;
}
