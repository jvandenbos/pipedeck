#include "fakedaemon.h"

#include <QDBusConnectionInterface>
#include <QDBusVariant>
#include <QUuid>

namespace {

void sendChanged(const QDBusConnection &connection) {
    auto signal = QDBusMessage::createSignal(DaemonProxy::Path, DaemonProxy::Interface, "Changed");
    QDBusConnection(connection).send(signal);
}

} // namespace

FakeObject::FakeObject(std::shared_ptr<FakeShared> s, bool legacy) : shared(std::move(s)) {
    if (legacy)
        new LegacyAdaptor(this);
    else
        new FakeAdaptor(this);
}

FakeDaemon::FakeDaemon(bool legacy) : m_shared(std::make_shared<FakeShared>()) {
    DaemonProxy::registerTypes();
    m_shared->clock.start();
    m_connectionName = "fake-pipedeckd-" + QUuid::createUuid().toString(QUuid::WithoutBraces);
    m_shared->connection = QDBusConnection::connectToBus(QDBusConnection::SessionBus, m_connectionName);
    m_object = new FakeObject(m_shared, legacy);
    m_object->moveToThread(&m_thread);
    // Deleted in its own thread as that thread winds down, so the adaptor's
    // signal relay never sees a cross-thread destroyed().
    QObject::connect(&m_thread, &QThread::finished, m_object, &QObject::deleteLater);
    m_thread.start();
    m_shared->connection.registerObject(DaemonProxy::Path, m_object, QDBusConnection::ExportAdaptors);
}

FakeDaemon::~FakeDaemon() {
    m_shared->holdGetAll = false;
    m_shared->gate.release(1000);
    release();
    m_shared->connection.unregisterObject(DaemonProxy::Path);
    m_thread.quit();
    m_thread.wait();
    QDBusConnection::disconnectFromBus(m_connectionName);
}

bool FakeDaemon::claim() {
    return m_shared->connection.registerService(DaemonProxy::Service);
}

void FakeDaemon::release() {
    m_shared->connection.unregisterService(DaemonProxy::Service);
}

FakeState FakeDaemon::state() const {
    QMutexLocker lock(&m_shared->mutex);
    return m_shared->state;
}

void FakeDaemon::emitChanged() {
    sendChanged(m_shared->connection);
}

void FakeDaemon::emitPropertiesChanged(const QStringList &names) {
    // zbus sends the new values along; the client ignores them and re-reads,
    // but they are sent here too so the message has the daemon's shape.
    const FakeState s = state();
    QVariantMap changed;
    for (const auto &name : names) {
        if (name == "Devices") changed[name] = QVariant::fromValue(s.devices);
        else if (name == "Streams") changed[name] = QVariant::fromValue(s.streams);
        else if (name == "Ports") changed[name] = QVariant::fromValue(s.ports);
        else if (name == "Eq") changed[name] = QVariant::fromValue(s.eq);
        else if (name == "EqPresets") changed[name] = QVariant::fromValue(s.eqPresets);
        else if (name == "AutoMute") changed[name] = QVariant::fromValue(s.autoMute);
        else if (name == "StreamSinks") changed[name] = QVariant::fromValue(s.streamSinks);
        else if (name == "NotificationSink") changed[name] = s.notificationSink;
        else if (name == "PortSwitchCap") changed[name] = s.portSwitchCap;
    }
    auto signal = QDBusMessage::createSignal(DaemonProxy::Path, "org.freedesktop.DBus.Properties", "PropertiesChanged");
    signal << QString::fromLatin1(DaemonProxy::Interface) << changed << QStringList();
    m_shared->connection.send(signal);
}

QList<FakeCall> FakeDaemon::calls(const QString &member) const {
    QMutexLocker lock(&m_shared->mutex);
    if (member.isEmpty())
        return m_shared->calls;
    QList<FakeCall> out;
    for (const auto &c : m_shared->calls)
        if (c.member == member)
            out << c;
    return out;
}

void FakeDaemon::clearCalls() {
    QMutexLocker lock(&m_shared->mutex);
    m_shared->calls.clear();
}

void FakeDaemon::failDelayed(const QString &errorName, const QString &message) {
    QList<QDBusMessage> delayed;
    {
        QMutexLocker lock(&m_shared->mutex);
        delayed.swap(m_shared->delayed);
    }
    for (const auto &m : delayed)
        m_shared->connection.send(m.createErrorReply(errorName, message));
}

// Made-up but plausible: a laptop with a jack (ports on one sink), a USB
// DAC, an HDMI output, two microphones and a few players.
void FakeDaemon::populateDemo() {
    edit([](FakeState &s) {
        s.devices = {
            {58, "alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink", "Family 17h HD Audio Controller Speaker",
             "sink", true, false, 0.343, false, "Laptop speakers"},
            {71, "alsa_output.usb-Fiio_K3-00.analog-stereo", "FiiO K3 Analog Stereo", "sink", false, false, 0.125,
             false, "FiiO K3"},
            {74, "alsa_output.pci-0000_07_00.1.hdmi-stereo", "Radeon HDMI / DisplayPort 1", "sink", false, false, 1.0,
             true, "Dell U2723QE"},
            {60, "alsa_input.pci-0000_07_00.6.HiFi__Mic1__source", "Digital Microphone", "source", true, false, 0.729,
             false, "Digital microphone"},
            {82, "alsa_input.usb-Blue_Yeti-00.analog-stereo", "Yeti Stereo Microphone Analog Stereo", "source", false,
             false, 0.512, true, "Yeti microphone"},
        };
        s.ports = {
            {58, 0, "[Out] Speaker", "Speaker", true, true},
            {58, 1, "[Out] Headphones", "Headphones", true, false},
            {74, 0, "hdmi-output-0", "HDMI / DisplayPort", true, true},
        };
        s.autoMute = {{58, false}};
        s.eqPresets = {{"harman-over-ear-2019", "Harman over-ear 2019"},
                       {"bass-lift", "Bass lift"},
                       {"voice-clarity", "Voice clarity"}};
        s.eq = {{58, ""}, {71, "harman-over-ear-2019"}, {74, ""}};
        s.streams = {
            {103, "Firefox", "firefox", "Lo-fi radio — live", "", 1.0, false},
            {118, "Spotify", "spotify", "Evening playlist", "alsa_output.usb-Fiio_K3-00.analog-stereo", 0.729, false},
            {121, "mpv", "mpv", "holiday-video.mkv", "alsa_output.pci-0000_07_00.1.hdmi-stereo", 0.512, true},
        };
        s.streamSinks = {{103, "alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink"},
                         {118, "alsa_output.usb-Fiio_K3-00.analog-stereo"},
                         {121, "alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink"}};
        s.notificationSink = "";
        s.portSwitchCap = 60;
        s.version = "1.4.0";
    });
}

// ---- adaptor --------------------------------------------------------------

FakeAdaptor::FakeAdaptor(FakeObject *parent) : QDBusAbstractAdaptor(parent), m_shared(parent->shared) {
    // Properties are served by Qt from the getters below; no signal relay.
    setAutoRelaySignals(false);
}

#define FAKE_GET(type, name, field)                                                                                     \
    type FakeAdaptor::name() const {                                                                                   \
        QMutexLocker lock(&m_shared->mutex);                                                                           \
        return m_shared->state.field;                                                                                  \
    }
FAKE_GET(QList<AutoMuteRow>, autoMute, autoMute)
FAKE_GET(QList<EqRow>, eq, eq)
FAKE_GET(QList<PresetRow>, eqPresets, eqPresets)
FAKE_GET(QString, notificationSink, notificationSink)
FAKE_GET(uint, portSwitchCap, portSwitchCap)
FAKE_GET(QList<PortRow>, ports, ports)
FAKE_GET(QList<StreamSinkRow>, streamSinks, streamSinks)
FAKE_GET(QList<StreamRow>, streams, streams)
FAKE_GET(QString, version, version)
#undef FAKE_GET

QList<DeviceRow> FakeAdaptor::devices() const {
    // GetAll reads every property once, so this getter counts snapshots and
    // is where a test can hold one open.
    ++m_shared->getAllCount;
    if (m_shared->holdGetAll)
        m_shared->gate.acquire();
    QMutexLocker lock(&m_shared->mutex);
    return m_shared->state.devices;
}

void FakeAdaptor::record(const char *member, const char *signature, const QVariantList &args) {
    // The signature of the message as it arrived on the wire.
    const QString wire = object()->fromDBus() ? object()->call().signature() : QString::fromLatin1(signature);
    QMutexLocker lock(&m_shared->mutex);
    m_shared->calls << FakeCall{QString::fromLatin1(member), wire, args, m_shared->clock.elapsed()};
}

void FakeAdaptor::changed() {
    sendChanged(m_shared->connection);
}

void FakeAdaptor::SetDefault(const QString &kind, const QString &name) {
    record("SetDefault", "ss", {kind, name});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        for (auto &d : m_shared->state.devices)
            if (d.kind == kind)
                d.isDefault = d.name == name;
    }
    changed();
}

void FakeAdaptor::SetNotificationSink(const QString &name) {
    record("SetNotificationSink", "s", {name});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        m_shared->state.notificationSink = name;
    }
    changed();
}

void FakeAdaptor::SetVolume(uint id, double volume) {
    record("SetVolume", "ud", {id, volume});
    if (m_shared->delayVolumeReplies) {
        object()->delayReply();
        QMutexLocker lock(&m_shared->mutex);
        m_shared->delayed << object()->call();
        return;
    }
    bool found = false;
    {
        QMutexLocker lock(&m_shared->mutex);
        for (auto &d : m_shared->state.devices)
            if (d.id == id) {
                found = true;
                if (m_shared->applyWrites)
                    d.volume = volume;
            }
        for (auto &s : m_shared->state.streams)
            if (s.id == id) {
                found = true;
                if (m_shared->applyWrites)
                    s.volume = volume;
            }
    }
    if (!found) {
        object()->replyError("dev.pipedeck.Error.NotFound", QStringLiteral("no node %1").arg(id));
        return;
    }
    if (m_shared->applyWrites)
        changed();
}

void FakeAdaptor::SetMute(uint id, bool mute) {
    record("SetMute", "ub", {id, mute});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        for (auto &d : m_shared->state.devices)
            if (d.id == id)
                d.mute = mute;
        for (auto &s : m_shared->state.streams)
            if (s.id == id)
                s.mute = mute;
    }
    changed();
}

void FakeAdaptor::SetStreamTarget(uint id, const QString &name) {
    record("SetStreamTarget", "us", {id, name});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        auto &st = m_shared->state;
        QString defaultSink;
        for (const auto &d : st.devices)
            if (d.kind == "sink" && d.isDefault)
                defaultSink = d.name;
        for (auto &s : st.streams)
            if (s.id == id)
                s.targetName = name;
        for (auto &s : st.streamSinks)
            if (s.streamId == id)
                s.sink = name.isEmpty() ? defaultSink : name;
    }
    changed();
}

void FakeAdaptor::SetPort(uint node_id, uint route_index) {
    record("SetPort", "uu", {node_id, route_index});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        for (auto &p : m_shared->state.ports)
            if (p.nodeId == node_id)
                p.active = p.routeIndex == route_index;
    }
    changed();
}

void FakeAdaptor::SetAutoMute(uint node_id, bool enabled) {
    record("SetAutoMute", "ub", {node_id, enabled});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        for (auto &a : m_shared->state.autoMute)
            if (a.nodeId == node_id)
                a.enabled = enabled;
    }
    changed();
}

void FakeAdaptor::SetEq(uint node_id, const QString &preset) {
    record("SetEq", "us", {node_id, preset});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        for (auto &e : m_shared->state.eq)
            if (e.nodeId == node_id)
                e.preset = preset;
    }
    changed();
}

void FakeAdaptor::SetPortSwitchCap(uint percent) {
    record("SetPortSwitchCap", "u", {percent});
    if (!m_shared->applyWrites)
        return;
    {
        QMutexLocker lock(&m_shared->mutex);
        m_shared->state.portSwitchCap = percent;
    }
    changed();
}

void FakeAdaptor::Refresh() {
    record("Refresh", "", {});
    changed();
}

// ---- legacy adaptor -------------------------------------------------------

LegacyAdaptor::LegacyAdaptor(FakeObject *parent) : QDBusAbstractAdaptor(parent), m_shared(parent->shared) {
    setAutoRelaySignals(false);
}

QList<DeviceRow> LegacyAdaptor::devices() const {
    ++m_shared->getAllCount;
    QMutexLocker lock(&m_shared->mutex);
    return m_shared->state.devices;
}
QString LegacyAdaptor::notificationSink() const {
    QMutexLocker lock(&m_shared->mutex);
    return m_shared->state.notificationSink;
}
QList<StreamRow> LegacyAdaptor::streams() const {
    QMutexLocker lock(&m_shared->mutex);
    return m_shared->state.streams;
}
QString LegacyAdaptor::version() const {
    return "1.0.0";
}

#define LEGACY_RECORD(member, sig, ...)                                                                                \
    do {                                                                                                               \
        QMutexLocker lock(&m_shared->mutex);                                                                           \
        m_shared->calls << FakeCall{member, sig, {__VA_ARGS__}, m_shared->clock.elapsed()};                            \
    } while (0)
void LegacyAdaptor::SetDefault(const QString &kind, const QString &name) { LEGACY_RECORD("SetDefault", "ss", kind, name); }
void LegacyAdaptor::SetNotificationSink(const QString &name) { LEGACY_RECORD("SetNotificationSink", "s", name); }
void LegacyAdaptor::SetVolume(uint id, double volume) { LEGACY_RECORD("SetVolume", "ud", id, volume); }
void LegacyAdaptor::SetMute(uint id, bool mute) { LEGACY_RECORD("SetMute", "ub", id, mute); }
void LegacyAdaptor::SetStreamTarget(uint id, const QString &name) { LEGACY_RECORD("SetStreamTarget", "us", id, name); }
void LegacyAdaptor::Refresh() { LEGACY_RECORD("Refresh", ""); }
#undef LEGACY_RECORD
