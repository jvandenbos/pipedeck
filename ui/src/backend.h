#pragma once

#include <QDBusConnection>
#include <QDBusError>
#include <QDBusPendingCall>
#include <QElapsedTimer>
#include <QHash>
#include <QObject>
#include <QSet>
#include <QSettings>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <memory>
#include <optional>

#include "appicons.h"
#include "daemon.h"
#include "rowmodel.h"

class QDBusServiceWatcher;

// The window's whole state, and the only place that talks to pipedeckd.
// QML reads the models and calls the Q_INVOKABLEs; everything that can fail,
// race or arrive late is handled here (SPEC-UI §2, §5).
class Backend : public QObject {
    Q_OBJECT
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    Q_PROPERTY(bool connected READ connected NOTIFY stateChanged)
    Q_PROPERTY(bool hasData READ hasData NOTIFY stateChanged)
    Q_PROPERTY(bool starting READ starting NOTIFY stateChanged)
    Q_PROPERTY(QString version READ version NOTIFY snapshotChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
    Q_PROPERTY(RowModel *outputs READ outputs CONSTANT)
    Q_PROPERTY(RowModel *inputs READ inputs CONSTANT)
    Q_PROPERTY(RowModel *streams READ streams CONSTANT)
    // The default sink and source, as one-row (or empty) models with the
    // device roles: the window's Output and Input rows (SPEC-UI §4.2).
    Q_PROPERTY(RowModel *defaultOutput READ defaultOutput CONSTANT)
    Q_PROPERTY(RowModel *defaultInput READ defaultInput CONSTANT)
    Q_PROPERTY(QVariantList eqPresets READ eqPresets NOTIFY snapshotChanged)
    // Every output as {value: node.name, text: nick}, for the pickers.
    Q_PROPERTY(QVariantList sinkChoices READ sinkChoices NOTIFY snapshotChanged)
    // Places (SPEC-UI §4.3): what the device popups list. A device with two
    // or more available ports is one place per port, named by the port;
    // otherwise the device is one place, by its nick. Each entry is
    // {nodeId, name, route (-1: the device as a whole), label, description,
    // glyph, current}. sinkPlaces lists sinks only (streams target sinks,
    // not ports), with current = the default sink.
    Q_PROPERTY(QVariantList outputPlaces READ outputPlaces NOTIFY snapshotChanged)
    Q_PROPERTY(QVariantList inputPlaces READ inputPlaces NOTIFY snapshotChanged)
    Q_PROPERTY(QVariantList sinkPlaces READ sinkPlaces NOTIFY snapshotChanged)
    Q_PROPERTY(QString notificationSink READ notificationSink NOTIFY snapshotChanged)
    Q_PROPERTY(int portSwitchCap READ portSwitchCap NOTIFY snapshotChanged)
    Q_PROPERTY(QVariantMap features READ features NOTIFY featuresChanged)

public:
    // Minimum gaps between writes and reads (SPEC-UI §4.1, §5).
    static constexpr int VolumeIntervalMs = 50;   // <= 20 SetVolume/s per node
    static constexpr int RefreshIntervalMs = 100; // <= 10 GetAll/s
    static constexpr int HoldMs = 300;            // final slider value outlives older echoes
    static constexpr int CapDebounceMs = 300;
    // A failed GetAll is retried while the owner stays the same.
    static constexpr int RetryFirstMs = 500;
    static constexpr int RetryMaxMs = 8000;

    explicit Backend(const QDBusConnection &connection = QDBusConnection::sessionBus(),
                     const QString &settingsFile = {}, std::shared_ptr<AppIcons> icons = {},
                     QObject *parent = nullptr);
    ~Backend() override;

    QString state() const;
    bool connected() const { return m_state == State::Connected; }
    bool hasData() const { return m_hasData; }
    bool starting() const { return m_starting; }
    QString version() const { return m_snapshot.version; }
    QString lastError() const { return m_lastError; }
    RowModel *outputs() const { return m_outputs; }
    RowModel *inputs() const { return m_inputs; }
    RowModel *streams() const { return m_streams; }
    RowModel *defaultOutput() const { return m_defaultOutput; }
    RowModel *defaultInput() const { return m_defaultInput; }
    QVariantList eqPresets() const;
    QVariantList sinkChoices() const;
    QVariantList outputPlaces() const { return places("sink", true); }
    QVariantList inputPlaces() const { return places("source", true); }
    QVariantList sinkPlaces() const { return places("sink", false); }
    QString notificationSink() const { return m_snapshot.notificationSink; }
    int portSwitchCap() const;
    QVariantMap features() const;
    // Boost (SPEC-UI §4.6): a per-row UI setting that widens that row's
    // slider to 150 %. Kept per device node.name, and per app for streams
    // (stream ids do not survive a restart), in QSettings ui/boost.
    Q_INVOKABLE void setBoost(uint id, bool on);
    bool boosted(uint id) const;
    int maxPercentFor(uint id) const { return boosted(id) ? 150 : 100; }
    // The glyph a device popup shows for a place (speaker, headphones,
    // monitor, bluetooth, usb, mic), from node and port names.
    static QString glyphFor(const QString &kind, const QString &nodeName, const QString &portName,
                            const QString &portDescription);

    // §4.1 volume contract: the slider is cubic percent, the wire is linear.
    Q_INVOKABLE static double percentToLinear(double percent);
    Q_INVOKABLE static double linearToPercent(double linear);

    Q_INVOKABLE void setDefault(const QString &kind, const QString &name);
    // Picks a place from a device popup: SetDefault unless the device is the
    // default already, then SetPort when `route` is a port that is not
    // active. Nothing at all for the current place.
    Q_INVOKABLE void choosePlace(const QString &kind, uint nodeId, int route);
    // `dragging` is true while the slider is held; the release sends false.
    Q_INVOKABLE void setVolume(uint id, double percent, bool dragging);
    Q_INVOKABLE void setMute(uint id, bool mute);
    Q_INVOKABLE void setStreamTarget(uint id, const QString &sinkName);
    Q_INVOKABLE void followDefault(uint id) { setStreamTarget(id, QString()); }
    Q_INVOKABLE void setNotificationSink(const QString &sinkName);
    Q_INVOKABLE void setPort(uint nodeId, uint routeIndex);
    Q_INVOKABLE void setAutoMute(uint nodeId, bool enabled);
    Q_INVOKABLE void setEq(uint nodeId, const QString &preset);
    Q_INVOKABLE void setPortSwitchCap(int percent);
    Q_INVOKABLE void refresh();
    Q_INVOKABLE void startDaemon();

    // For tests: how many GetAll calls went out, and the owner generation.
    int refreshCount() const { return m_refreshCount; }
    quint64 generation() const { return m_generation; }
    QString owner() const { return m_owner; } // the unique name calls go to
    const Snapshot &snapshot() const { return m_snapshot; }

signals:
    void stateChanged();
    void snapshotChanged();
    void featuresChanged();
    void lastErrorChanged();
    void boostChanged();
    // Emitted as each SetVolume leaves, so the <= 20/s rule is measurable
    // where it is enforced rather than after bus jitter.
    void volumeSent(uint id, double linear);

private:
    enum class State { Connecting, Absent, Connected };

    struct VolumeWrite {
        double linear = 0;
        bool queued = false;
        qint64 lastSentNs = -1;
        QTimer *timer = nullptr;
    };
    // One slider gesture: from the first input until its echo arrives or
    // HoldMs pass after the release.
    struct Hold {
        double percent = 0;
        bool dragging = false;
        qint64 lastInputMs = 0;
        // The last value this gesture sent; forgotten when an observed value
        // other than it arrives, so it never suppresses a write after
        // something else changed the level.
        double sentLinear = -1;
    };

    void lookUpOwner();
    void setOwner(const QString &owner);
    void setState(State state);
    void requestRefresh();
    void startRefresh();
    void applySnapshot(const Snapshot &snapshot);
    void rebuildModels();
    void dropUnsentWrites();
    void discardWrite(QHash<uint, VolumeWrite>::iterator it);
    std::optional<double> observedLinear(uint id) const;
    void showPercent(uint id, double percent);
    void queueVolume(uint id, double linear);
    void flushVolume(uint id);
    void flushCap();
    void sweepHolds();
    void track(const QDBusPendingCall &call, const QString &method, std::optional<uint> node = {});
    void handleError(const QString &method, std::optional<uint> node, const QDBusError &error);
    void setLastError(const QString &text);
    bool nodeKnown(uint id) const;
    QString sinkNick(const QString &name) const;
    QVariantList places(const QString &kind, bool withPorts) const;
    QString boostKey(uint id) const;
    qint64 now() const { return m_clock.elapsed(); }
    qint64 nowNs() const { return m_clock.nsecsElapsed(); }

    QDBusConnection m_connection;
    DaemonProxy *m_proxy = nullptr;
    QDBusServiceWatcher *m_watcher = nullptr;
    std::shared_ptr<AppIcons> m_icons;
    std::unique_ptr<QSettings> m_settings;
    QElapsedTimer m_clock;

    State m_state = State::Connecting;
    QString m_owner;
    quint64 m_generation = 0;
    bool m_hasData = false;
    bool m_starting = false;
    QString m_lastError;
    Snapshot m_snapshot;
    QSet<QString> m_missingMethods;

    bool m_refreshInFlight = false;
    bool m_refreshDirty = false;
    qint64 m_lastRefreshMs = -1;
    int m_refreshCount = 0;
    QTimer m_refreshTimer;
    QTimer m_retryTimer;
    int m_retryDelayMs = RetryFirstMs;

    QHash<uint, VolumeWrite> m_volumeWrites;
    QHash<uint, Hold> m_holds;
    QTimer m_holdSweep;
    // A NotFound for a node write is judged against the next snapshot: if
    // the node is gone by then it simply vanished under the write.
    QHash<uint, QString> m_notFound;
    std::optional<uint> m_capPending;
    QTimer m_capTimer;

    QStringList m_boosted; // boost keys: "node:<node.name>" or "app:<binary|app name>"

    RowModel *m_outputs;
    RowModel *m_inputs;
    RowModel *m_streams;
    RowModel *m_defaultOutput;
    RowModel *m_defaultInput;
};
