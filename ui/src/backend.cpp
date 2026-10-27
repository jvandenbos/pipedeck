#include "backend.h"

#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusServiceWatcher>
#include <QUrl>
#include <cmath>

namespace {

const QByteArrayList DeviceRoles{"nodeId",    "name",     "nick",       "description", "isDefault", "isVirtual",
                                 "percent", "mute",   "ports",      "showPorts",   "hasAutoMute", "autoMute",
                                 "eqPreset", "eqName", "eqAvailable"};
const QByteArrayList StreamRoles{"nodeId",          "label",        "icon",        "percent",  "mute",
                                 "target",      "targetLabel",  "targetAbsent", "observed", "observedLabel",
                                 "showObserved"};

// Half a cubic percent: below what the label shows, above f32 round-trips in
// the daemon, so the echo of our own write is recognised as such.
constexpr double EchoTolerancePercent = 0.5;

} // namespace

Backend::Backend(const QDBusConnection &connection, const QString &settingsFile, std::shared_ptr<AppIcons> icons,
                 QObject *parent)
    : QObject(parent), m_connection(connection), m_icons(icons ? std::move(icons) : std::make_shared<AppIcons>()),
      m_outputs(new RowModel(DeviceRoles, "nodeId", this)), m_inputs(new RowModel(DeviceRoles, "nodeId", this)),
      m_streams(new RowModel(StreamRoles, "nodeId", this)) {
    m_clock.start();
    m_settings = settingsFile.isEmpty()
                     ? std::make_unique<QSettings>(QSettings::IniFormat, QSettings::UserScope, "pipedeck-ui",
                                                   "pipedeck-ui")
                     : std::make_unique<QSettings>(settingsFile, QSettings::IniFormat);
    m_allowOverdrive = m_settings->value("ui/allowOverdrive", false).toBool();

    m_refreshTimer.setSingleShot(true);
    connect(&m_refreshTimer, &QTimer::timeout, this, &Backend::startRefresh);
    m_holdSweep.setInterval(50);
    connect(&m_holdSweep, &QTimer::timeout, this, &Backend::sweepHolds);
    m_capTimer.setSingleShot(true);
    m_capTimer.setInterval(CapDebounceMs);
    connect(&m_capTimer, &QTimer::timeout, this, &Backend::flushCap);

    // Order matters (SPEC-UI §5): the owner watch and both change signals are
    // subscribed before the first snapshot is asked for.
    m_watcher = new QDBusServiceWatcher(DaemonProxy::Service, m_connection,
                                        QDBusServiceWatcher::WatchForOwnerChange, this);
    connect(m_watcher, &QDBusServiceWatcher::serviceOwnerChanged, this,
            [this](const QString &, const QString &, const QString &now) { setOwner(now); });
    m_proxy = new DaemonProxy(m_connection, this);
    connect(m_proxy, &DaemonProxy::Changed, this, &Backend::requestRefresh);
    connect(m_proxy, &DaemonProxy::propertiesChanged, this, &Backend::requestRefresh);

    lookUpOwner();
}

Backend::~Backend() = default;

QString Backend::state() const {
    switch (m_state) {
    case State::Connecting: return "connecting";
    case State::Absent: return "absent";
    case State::Connected: return "connected";
    }
    return {};
}

void Backend::setState(State state) {
    if (state == m_state)
        return;
    m_state = state;
    emit stateChanged();
}

double Backend::percentToLinear(double percent) {
    const double p = std::clamp(percent, 0.0, 150.0) / 100.0;
    return p * p * p;
}

double Backend::linearToPercent(double linear) {
    return std::cbrt(std::max(0.0, linear)) * 100.0;
}

// ---- owner tracking -------------------------------------------------------

void Backend::lookUpOwner() {
    auto message = QDBusMessage::createMethodCall("org.freedesktop.DBus", "/org/freedesktop/DBus",
                                                  "org.freedesktop.DBus", "GetNameOwner");
    message << QString::fromLatin1(DaemonProxy::Service);
    auto *watcher = new QDBusPendingCallWatcher(m_connection.asyncCall(message), this);
    const quint64 generation = m_generation;
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, generation](QDBusPendingCallWatcher *w) {
        w->deleteLater();
        // A NameOwnerChanged that overtook this reply is newer; trust it.
        if (generation != m_generation)
            return;
        QDBusPendingReply<QString> reply = *w;
        setOwner(reply.isError() ? QString() : reply.value());
    });
}

void Backend::setOwner(const QString &owner) {
    // Every owner change, including one unique name replacing another, is a
    // new generation: node ids from the old daemon mean nothing to the new
    // one, so in-flight replies are ignored and unsent writes are dropped.
    ++m_generation;
    m_owner = owner;
    dropUnsentWrites();
    m_refreshInFlight = false;
    m_refreshDirty = false;
    m_refreshTimer.stop();
    m_lastRefreshMs = -1;
    if (owner.isEmpty()) {
        setState(State::Absent);
        return;
    }
    m_starting = false;
    setState(State::Connecting);
    requestRefresh();
}

void Backend::dropUnsentWrites() {
    for (auto &write : m_volumeWrites)
        if (write.timer)
            write.timer->deleteLater();
    m_volumeWrites.clear();
    m_holds.clear();
    m_holdSweep.stop();
    m_capPending.reset();
    m_capTimer.stop();
    m_notFound.clear();
}

// ---- refresh state machine ------------------------------------------------

void Backend::requestRefresh() {
    if (m_owner.isEmpty())
        return;
    if (m_refreshInFlight) {
        m_refreshDirty = true;
        return;
    }
    if (m_refreshTimer.isActive())
        return;
    const qint64 since = m_lastRefreshMs < 0 ? RefreshIntervalMs : now() - m_lastRefreshMs;
    if (since >= RefreshIntervalMs)
        startRefresh();
    else
        m_refreshTimer.start(int(RefreshIntervalMs - since));
}

void Backend::startRefresh() {
    if (m_owner.isEmpty() || m_refreshInFlight)
        return;
    m_refreshInFlight = true;
    m_lastRefreshMs = now();
    ++m_refreshCount;
    const quint64 generation = m_generation;
    auto *watcher = new QDBusPendingCallWatcher(m_proxy->getAll(), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, generation](QDBusPendingCallWatcher *w) {
        w->deleteLater();
        if (generation != m_generation)
            return; // a reply from a daemon that has since gone or been replaced
        m_refreshInFlight = false;
        QDBusPendingReply<QVariantMap> reply = *w;
        if (reply.isError())
            setLastError(QStringLiteral("%1: %2").arg(reply.error().name(), reply.error().message()));
        else
            applySnapshot(DaemonProxy::decode(reply.value()));
        if (m_refreshDirty) {
            m_refreshDirty = false;
            requestRefresh();
        }
    });
}

void Backend::applySnapshot(const Snapshot &snapshot) {
    m_snapshot = snapshot;
    m_hasData = true;

    // A queued write for a node that vanished goes nowhere, silently.
    for (auto it = m_volumeWrites.begin(); it != m_volumeWrites.end();) {
        if (!nodeKnown(it.key())) {
            if (it->timer)
                it->timer->deleteLater();
            it = m_volumeWrites.erase(it);
        } else {
            ++it;
        }
    }

    // §5 slider rule: a released slider keeps its final value until that
    // value comes back. An older observed value in between is ignored.
    auto observedPercent = [this](uint id) -> std::optional<double> {
        for (const auto &d : m_snapshot.devices)
            if (d.id == id)
                return linearToPercent(d.volume);
        for (const auto &s : m_snapshot.streams)
            if (s.id == id)
                return linearToPercent(s.volume);
        return std::nullopt;
    };
    for (auto it = m_holds.begin(); it != m_holds.end();) {
        const auto observed = observedPercent(it.key());
        if (!observed || (!it->dragging && std::abs(*observed - it->percent) < EchoTolerancePercent))
            it = m_holds.erase(it);
        else
            ++it;
    }

    // Only a NotFound for a node that is still there is worth showing.
    for (auto it = m_notFound.cbegin(); it != m_notFound.cend(); ++it)
        if (nodeKnown(it.key()))
            setLastError(it.value());
    m_notFound.clear();

    rebuildModels();
    setState(State::Connected);
    emit stateChanged(); // hasData may have flipped without a state change
    emit snapshotChanged();
    emit featuresChanged();
}

bool Backend::nodeKnown(uint id) const {
    for (const auto &d : m_snapshot.devices)
        if (d.id == id)
            return true;
    for (const auto &s : m_snapshot.streams)
        if (s.id == id)
            return true;
    return false;
}

QString Backend::sinkNick(const QString &name) const {
    for (const auto &d : m_snapshot.devices)
        if (d.kind == "sink" && d.name == name)
            return !d.nick.isEmpty() ? d.nick : !d.description.isEmpty() ? d.description : d.name;
    return {};
}

void Backend::rebuildModels() {
    const auto f = features();
    auto shown = [this](uint id, double linear) {
        const auto it = m_holds.constFind(id);
        return it != m_holds.constEnd() ? it->percent : linearToPercent(linear);
    };

    QList<QVariantMap> outputs, inputs, streams;
    QString defaultSink;
    for (const auto &d : m_snapshot.devices) {
        if (d.kind == "sink" && d.isDefault)
            defaultSink = d.name;

        QVariantList ports;
        for (const auto &p : m_snapshot.ports)
            if (p.nodeId == d.id && p.available)
                ports << QVariantMap{{"index", p.routeIndex},
                                     {"label", p.description.isEmpty() ? p.name : p.description},
                                     {"active", p.active}};
        int autoMute = -1;
        for (const auto &a : m_snapshot.autoMute)
            if (a.nodeId == d.id)
                autoMute = a.enabled ? 1 : 0;
        bool eqAvailable = false;
        QString eqPreset, eqName;
        for (const auto &e : m_snapshot.eq)
            if (e.nodeId == d.id) {
                eqAvailable = true;
                eqPreset = e.preset;
            }
        for (const auto &p : m_snapshot.eqPresets)
            if (!eqPreset.isEmpty() && p.id == eqPreset)
                eqName = p.name;
        if (!eqPreset.isEmpty() && eqName.isEmpty())
            eqName = eqPreset;

        const QVariantMap row{
            {"nodeId", d.id},
            {"name", d.name},
            {"nick", !d.nick.isEmpty() ? d.nick : !d.description.isEmpty() ? d.description : d.name},
            {"description", d.description},
            {"isDefault", d.isDefault},
            {"isVirtual", d.isVirtual},
            {"percent", shown(d.id, d.volume)},
            {"mute", d.mute},
            {"ports", ports},
            {"showPorts", f.value("ports").toBool() && ports.size() >= 2},
            {"hasAutoMute", f.value("autoMute").toBool() && d.kind == "sink" && autoMute >= 0},
            {"autoMute", autoMute == 1},
            {"eqPreset", eqPreset},
            {"eqName", f.value("eq").toBool() ? eqName : QString()},
            {"eqAvailable", eqAvailable && !d.isVirtual},
        };
        if (d.kind == "sink")
            outputs << row;
        else if (d.kind == "source")
            inputs << row;
    }

    QHash<uint, QString> observed;
    for (const auto &s : m_snapshot.streamSinks)
        observed.insert(s.streamId, s.sink);
    const bool haveObserved = f.value("streamSinks").toBool();
    for (const auto &s : m_snapshot.streams) {
        QString head = !s.appName.isEmpty() ? s.appName : s.binary;
        QStringList parts;
        if (!head.isEmpty())
            parts << head;
        if (!s.mediaName.isEmpty() && s.mediaName != head)
            parts << s.mediaName;
        const QString label = parts.isEmpty() ? QStringLiteral("#%1").arg(s.id) : parts.join(QStringLiteral(" — "));
        const QString icon = m_icons->iconFor(s.binary, s.appName);

        const QString targetNick = s.targetName.isEmpty() ? QString() : sinkNick(s.targetName);
        const bool absent = !s.targetName.isEmpty() && targetNick.isEmpty();
        const QString seen = observed.value(s.id);
        // "playing on" only says something when it differs from where the
        // stream should be: its request, or the default when following it.
        const QString expected = s.targetName.isEmpty() ? defaultSink : s.targetName;
        const QString seenNick = sinkNick(seen);
        streams << QVariantMap{
            {"nodeId", s.id},
            {"label", label},
            {"icon", icon.isEmpty() ? QString() : QString::fromLatin1(QUrl::toPercentEncoding(icon))},
            {"percent", shown(s.id, s.volume)},
            {"mute", s.mute},
            {"target", s.targetName},
            {"targetLabel", absent ? s.targetName : targetNick},
            {"targetAbsent", absent},
            {"observed", haveObserved ? seen : QString()},
            {"observedLabel", haveObserved ? (seenNick.isEmpty() ? seen : seenNick) : QString()},
            {"showObserved", haveObserved && !seen.isEmpty() && seen != expected},
        };
    }

    m_outputs->setRows(outputs);
    m_inputs->setRows(inputs);
    m_streams->setRows(streams);
}

QVariantList Backend::eqPresets() const {
    QVariantList list;
    for (const auto &p : m_snapshot.eqPresets)
        list << QVariantMap{{"id", p.id}, {"name", p.name.isEmpty() ? p.id : p.name}};
    return list;
}

QVariantList Backend::sinkChoices() const {
    QVariantList list;
    for (const auto &d : m_snapshot.devices)
        if (d.kind == "sink")
            list << QVariantMap{{"value", d.name}, {"text", sinkNick(d.name)}};
    return list;
}

int Backend::portSwitchCap() const {
    return m_capPending ? int(*m_capPending) : int(m_snapshot.portSwitchCap);
}

QVariantMap Backend::features() const {
    const auto &s = m_snapshot;
    auto can = [this](const char *method) { return !m_missingMethods.contains(QLatin1String(method)); };
    return {
        {"devices", s.has("Devices")},
        {"defaults", s.has("Devices") && can("SetDefault")},
        {"ports", s.has("Ports") && can("SetPort")},
        {"autoMute", s.has("AutoMute") && can("SetAutoMute")},
        {"eq", s.has("Eq") && s.has("EqPresets") && can("SetEq")},
        {"notifications", s.has("NotificationSink") && can("SetNotificationSink")},
        {"portSwitchCap", s.has("PortSwitchCap") && can("SetPortSwitchCap")},
        {"streams", s.has("Streams")},
        {"streamTarget", s.has("Streams") && can("SetStreamTarget")},
        {"streamSinks", s.has("StreamSinks")},
        {"refresh", can("Refresh")},
    };
}

void Backend::setAllowOverdrive(bool on) {
    if (on == m_allowOverdrive)
        return;
    m_allowOverdrive = on;
    m_settings->setValue("ui/allowOverdrive", on);
    m_settings->sync();
    emit allowOverdriveChanged();
}

// ---- writes ---------------------------------------------------------------

void Backend::track(const QDBusPendingCall &call, const QString &method, std::optional<uint> node) {
    auto *watcher = new QDBusPendingCallWatcher(call, this);
    const quint64 generation = m_generation;
    connect(watcher, &QDBusPendingCallWatcher::finished, this,
            [this, generation, method, node](QDBusPendingCallWatcher *w) {
                w->deleteLater();
                if (generation != m_generation)
                    return; // dispatched to a daemon that is gone: ignore
                const QDBusPendingReply<> reply = *w;
                if (reply.isError())
                    handleError(method, node, reply.error());
                else
                    setLastError({});
            });
}

void Backend::handleError(const QString &method, std::optional<uint> node, const QDBusError &error) {
    if (error.type() == QDBusError::UnknownMethod) {
        // An older daemon: hide the control rather than nag (SPEC-UI §5).
        m_missingMethods.insert(method);
        rebuildModels();
        emit featuresChanged();
        return;
    }
    const QString text = QStringLiteral("%1: %2").arg(error.name(), error.message());
    if (node && error.name() == QLatin1String("dev.pipedeck.Error.NotFound")) {
        // The node may have vanished under the write, with the snapshot that
        // says so still on its way. Decide once a fresh one is in.
        if (nodeKnown(*node)) {
            m_notFound.insert(*node, text);
            requestRefresh();
        }
        return;
    }
    setLastError(text);
}

void Backend::setLastError(const QString &text) {
    if (text == m_lastError)
        return;
    m_lastError = text;
    emit lastErrorChanged();
}

void Backend::setDefault(const QString &kind, const QString &name) {
    if (connected())
        track(m_proxy->setDefault(kind, name), "SetDefault");
}

void Backend::setVolume(uint id, double percent, bool dragging) {
    if (!connected() || !nodeKnown(id))
        return;
    percent = std::clamp(percent, 0.0, double(maxPercent()));
    Hold &hold = m_holds[id];
    hold.percent = percent;
    hold.dragging = dragging;
    hold.lastInputMs = now();
    if (!m_holdSweep.isActive())
        m_holdSweep.start();
    rebuildModels();
    queueVolume(id, percentToLinear(percent));
}

void Backend::queueVolume(uint id, double linear) {
    VolumeWrite &write = m_volumeWrites[id];
    if (!write.queued && write.lastSentLinear == linear)
        return; // a release on the value the drag already sent
    write.linear = linear;
    write.queued = true;
    // Nanoseconds and a precise timer: millisecond truncation and a coarse
    // timer firing early would each let a gap come in under 50 ms.
    constexpr qint64 interval = qint64(VolumeIntervalMs) * 1000000;
    const qint64 since = write.lastSentNs < 0 ? interval : nowNs() - write.lastSentNs;
    if (since >= interval) {
        flushVolume(id);
        return;
    }
    if (!write.timer) {
        write.timer = new QTimer(this);
        write.timer->setSingleShot(true);
        write.timer->setTimerType(Qt::PreciseTimer);
        connect(write.timer, &QTimer::timeout, this, [this, id] {
            const auto it = m_volumeWrites.find(id);
            if (it == m_volumeWrites.end())
                return;
            // A precise timer still rounds to whole milliseconds; never early.
            const qint64 left = qint64(VolumeIntervalMs) * 1000000 - (nowNs() - it->lastSentNs);
            if (left > 0)
                it->timer->start(int((left + 999999) / 1000000));
            else
                flushVolume(id);
        });
    }
    if (!write.timer->isActive())
        write.timer->start(int((interval - since + 999999) / 1000000));
}

void Backend::flushVolume(uint id) {
    const auto it = m_volumeWrites.find(id);
    if (it == m_volumeWrites.end() || !it->queued || !connected())
        return;
    it->queued = false;
    it->lastSentNs = nowNs();
    it->lastSentLinear = it->linear;
    track(m_proxy->setVolume(id, it->linear), "SetVolume", id);
    emit volumeSent(id, it->linear);
}

void Backend::sweepHolds() {
    bool expired = false;
    for (auto it = m_holds.begin(); it != m_holds.end();) {
        if (!it->dragging && now() - it->lastInputMs >= HoldMs) {
            it = m_holds.erase(it);
            expired = true;
        } else {
            ++it;
        }
    }
    if (m_holds.isEmpty())
        m_holdSweep.stop();
    if (expired)
        rebuildModels(); // last observed wins from here on
}

void Backend::setMute(uint id, bool mute) {
    // Its own write, never folded into a volume write (SPEC §9.1).
    if (connected())
        track(m_proxy->setMute(id, mute), "SetMute", id);
}

void Backend::setStreamTarget(uint id, const QString &sinkName) {
    if (connected())
        track(m_proxy->setStreamTarget(id, sinkName), "SetStreamTarget", id);
}

void Backend::setNotificationSink(const QString &sinkName) {
    if (connected())
        track(m_proxy->setNotificationSink(sinkName), "SetNotificationSink");
}

void Backend::setPort(uint nodeId, uint routeIndex) {
    if (connected())
        track(m_proxy->setPort(nodeId, routeIndex), "SetPort", nodeId);
}

void Backend::setAutoMute(uint nodeId, bool enabled) {
    if (connected())
        track(m_proxy->setAutoMute(nodeId, enabled), "SetAutoMute", nodeId);
}

void Backend::setEq(uint nodeId, const QString &preset) {
    if (connected())
        track(m_proxy->setEq(nodeId, preset), "SetEq", nodeId);
}

void Backend::setPortSwitchCap(int percent) {
    if (!connected())
        return;
    // A spin box held down steps fast; only the value it settles on is sent.
    m_capPending = uint(std::clamp(percent, 0, 150));
    m_capTimer.start();
    emit snapshotChanged();
}

void Backend::flushCap() {
    if (!m_capPending || !connected())
        return;
    const uint percent = *m_capPending;
    m_capPending.reset();
    track(m_proxy->setPortSwitchCap(percent), "SetPortSwitchCap");
}

void Backend::refresh() {
    if (connected())
        track(m_proxy->refresh(), "Refresh");
}

void Backend::startDaemon() {
    if (m_starting || !m_owner.isEmpty())
        return;
    m_starting = true;
    emit stateChanged();
    // D-Bus activation: the bus starts pipedeckd through
    // dev.pipedeck.Daemon.service (and so its systemd user unit).
    auto message = QDBusMessage::createMethodCall("org.freedesktop.DBus", "/org/freedesktop/DBus",
                                                  "org.freedesktop.DBus", "StartServiceByName");
    message << QString::fromLatin1(DaemonProxy::Service) << 0u;
    auto *watcher = new QDBusPendingCallWatcher(m_connection.asyncCall(message), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this](QDBusPendingCallWatcher *w) {
        w->deleteLater();
        const QDBusPendingReply<uint> reply = *w;
        m_starting = false;
        emit stateChanged();
        if (reply.isError())
            setLastError(QStringLiteral("%1: %2").arg(reply.error().name(), reply.error().message()));
        else
            setLastError({});
    });
}
