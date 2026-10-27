#pragma once

// Hand-written client proxy for dev.pipedeck.Daemon1 (SPEC-UI §2). The build
// host has no qt6-tools, so there is no qdbusxml2cpp; the checked-in
// crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml stays the contract and a unit
// test compares it member by member with members() below.

#include <QDBusAbstractInterface>
#include <QDBusArgument>
#include <QDBusPendingCall>
#include <QList>
#include <QMetaType>
#include <QString>
#include <QStringList>
#include <QVariantMap>

// One struct per struct-array property, field order exactly as on the wire.

struct DeviceRow { // a(usssbbdbs)
    uint id = 0;
    QString name, description, kind;
    bool isDefault = false, isVirtual = false;
    double volume = 0;
    bool mute = false;
    QString nick;
    bool operator==(const DeviceRow &) const = default;
};

struct StreamRow { // a(ussssdb)
    uint id = 0;
    QString appName, binary, mediaName, targetName;
    double volume = 0;
    bool mute = false;
    bool operator==(const StreamRow &) const = default;
};

struct PortRow { // a(uussbb)
    uint nodeId = 0, routeIndex = 0;
    QString name, description;
    bool available = false, active = false;
    bool operator==(const PortRow &) const = default;
};

struct EqRow { // a(us)
    uint nodeId = 0;
    QString preset;
    bool operator==(const EqRow &) const = default;
};

struct PresetRow { // a(ss)
    QString id, name;
    bool operator==(const PresetRow &) const = default;
};

struct AutoMuteRow { // a(ub)
    uint nodeId = 0;
    bool enabled = false;
    bool operator==(const AutoMuteRow &) const = default;
};

struct StreamSinkRow { // a(us)
    uint streamId = 0;
    QString sink;
    bool operator==(const StreamSinkRow &) const = default;
};

Q_DECLARE_METATYPE(DeviceRow)
Q_DECLARE_METATYPE(StreamRow)
Q_DECLARE_METATYPE(PortRow)
Q_DECLARE_METATYPE(EqRow)
Q_DECLARE_METATYPE(PresetRow)
Q_DECLARE_METATYPE(AutoMuteRow)
Q_DECLARE_METATYPE(StreamSinkRow)

QDBusArgument &operator<<(QDBusArgument &, const DeviceRow &);
const QDBusArgument &operator>>(const QDBusArgument &, DeviceRow &);
QDBusArgument &operator<<(QDBusArgument &, const StreamRow &);
const QDBusArgument &operator>>(const QDBusArgument &, StreamRow &);
QDBusArgument &operator<<(QDBusArgument &, const PortRow &);
const QDBusArgument &operator>>(const QDBusArgument &, PortRow &);
QDBusArgument &operator<<(QDBusArgument &, const EqRow &);
const QDBusArgument &operator>>(const QDBusArgument &, EqRow &);
QDBusArgument &operator<<(QDBusArgument &, const PresetRow &);
const QDBusArgument &operator>>(const QDBusArgument &, PresetRow &);
QDBusArgument &operator<<(QDBusArgument &, const AutoMuteRow &);
const QDBusArgument &operator>>(const QDBusArgument &, AutoMuteRow &);
QDBusArgument &operator<<(QDBusArgument &, const StreamSinkRow &);
const QDBusArgument &operator>>(const QDBusArgument &, StreamSinkRow &);

// Everything one GetAll can carry. `present` lists the properties the daemon
// actually returned with the expected signature, so an older daemon hides the
// controls it cannot back (SPEC-UI §5 "Older daemon").
struct Snapshot {
    QList<DeviceRow> devices;
    QList<StreamRow> streams;
    QList<PortRow> ports;
    QList<EqRow> eq;
    QList<PresetRow> eqPresets;
    QList<AutoMuteRow> autoMute;
    QList<StreamSinkRow> streamSinks;
    QString notificationSink;
    uint portSwitchCap = 0;
    QString version;
    QStringList present;
    bool has(const QString &property) const { return present.contains(property); }
};

class DaemonProxy : public QDBusAbstractInterface {
    Q_OBJECT

public:
    static constexpr const char *Service = "dev.pipedeck.Daemon";
    static constexpr const char *Path = "/dev/pipedeck/Daemon";
    static constexpr const char *Interface = "dev.pipedeck.Daemon1";

    enum class Kind { Method, Property, Signal };
    struct Member {
        Kind kind;
        QString name;
        QString signature; // in-args for a method or signal, the type for a property
    };
    // The interface as this client speaks it. The XML test requires every XML
    // member here with the same signature.
    static const QList<Member> &members();

    // Registers every row type with QtDBus. Idempotent; call before any use.
    static void registerTypes();

    explicit DaemonProxy(const QDBusConnection &connection, QObject *parent = nullptr);

    QDBusPendingCall getAll() const;
    QDBusPendingCall setDefault(const QString &kind, const QString &name);
    QDBusPendingCall setNotificationSink(const QString &name);
    QDBusPendingCall setVolume(uint id, double linearVolume);
    QDBusPendingCall setMute(uint id, bool mute);
    QDBusPendingCall setStreamTarget(uint id, const QString &name);
    QDBusPendingCall setPort(uint nodeId, uint routeIndex);
    QDBusPendingCall setAutoMute(uint nodeId, bool enabled);
    QDBusPendingCall setEq(uint nodeId, const QString &preset);
    QDBusPendingCall setPortSwitchCap(uint percent);
    QDBusPendingCall refresh();

    // Decodes a GetAll reply. A property with an unexpected wire signature is
    // treated as absent rather than half-decoded.
    static Snapshot decode(const QVariantMap &properties);

signals:
    // The daemon's own coalesced nudge; QDBusAbstractInterface subscribes to
    // it on the bus as soon as something connects here.
    void Changed();
    // Standard PropertiesChanged for this interface, reduced to the names.
    void propertiesChanged(const QStringList &names);

private slots:
    void onPropertiesChanged(const QString &interface, const QVariantMap &changed, const QStringList &invalidated);
};
