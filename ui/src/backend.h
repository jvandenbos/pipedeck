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
    Q_PROPERTY(QVariantList eqPresets READ eqPresets NOTIFY snapshotChanged)
    // Every output as {value: node.name, text: nick}, for the pickers.
    Q_PROPERTY(QVariantList sinkChoices READ sinkChoices NOTIFY snapshotChanged)
    Q_PROPERTY(QString notificationSink READ notificationSink NOTIFY snapshotChanged)
    Q_PROPERTY(int portSwitchCap READ portSwitchCap NOTIFY snapshotChanged)
    Q_PROPERTY(QVariantMap features READ features NOTIFY featuresChanged)
    Q_PROPERTY(bool allowOverdrive READ allowOverdrive WRITE setAllowOverdrive NOTIFY allowOverdriveChanged)
    Q_PROPERTY(int maxPercent READ maxPercent NOTIFY allowOverdriveChanged)

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
    QVariantList eqPresets() const;
    QVariantList sinkChoices() const;
    QString notificationSink() const { return m_snapshot.notificationSink; }
    int portSwitchCap() const;
    QVariantMap features() const;
    bool allowOverdrive() const { return m_allowOverdrive; }
    void setAllowOverdrive(bool on);
    int maxPercent() const { return m_allowOverdrive ? 150 : 100; }

    // §4.1 volume contract: the slider is cubic percent, the wire is linear.
    Q_INVOKABLE static double percentToLinear(double percent);
    Q_INVOKABLE static double linearToPercent(double linear);

    Q_INVOKABLE void setDefault(const QString &kind, const QString &name);
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
    void allowOverdriveChanged();
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

    bool m_allowOverdrive = false;

    RowModel *m_outputs;
    RowModel *m_inputs;
    RowModel *m_streams;
};
