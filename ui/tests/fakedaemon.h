#pragma once

// A stand-in pipedeckd for tests and the screenshot harness: a real
// QDBusAbstractAdaptor serving dev.pipedeck.Daemon1 on its own bus
// connection, so every call and property crosses the wire exactly as with
// the Rust daemon. The object lives in a worker thread; that lets a test hold
// a GetAll reply open (the getter blocks) while the client keeps running.

#include <QDBusAbstractAdaptor>
#include <QDBusConnection>
#include <QDBusContext>
#include <QDBusMessage>
#include <QElapsedTimer>
#include <QMutex>
#include <QObject>
#include <QSemaphore>
#include <QThread>
#include <QVariantList>
#include <QHash>
#include <atomic>
#include <memory>
#include <tuple>

#include "daemon.h"

// What the fake puts on the wire: plain tuples, filled field by field in the
// order of the Rust daemon's to_dbus() (crates/pipedeckd/src/state.rs,
// route.rs, eq.rs) and marshalled by QtDBus's own std::tuple operators. The
// proxy's row operators are never used to encode, so a field swapped in one
// of its operator>> reads back wrong and the decode test fails.
using WireDevice = std::tuple<uint, QString, QString, QString, bool, bool, double, bool, QString>;
using WireStream = std::tuple<uint, QString, QString, QString, QString, double, bool>;
using WirePort = std::tuple<uint, uint, QString, QString, bool, bool>;
using WireIdString = std::tuple<uint, QString>; // Eq, StreamSinks
using WirePreset = std::tuple<QString, QString>;
using WireAutoMute = std::tuple<uint, bool>;
using WireDevices = QList<WireDevice>;
using WireStreams = QList<WireStream>;
using WirePorts = QList<WirePort>;
using WireIdStrings = QList<WireIdString>;
using WirePresets = QList<WirePreset>;
using WireAutoMutes = QList<WireAutoMute>;

struct FakeState {
    QList<DeviceRow> devices;
    QList<StreamRow> streams;
    QList<PortRow> ports;
    QList<EqRow> eq;
    QList<PresetRow> eqPresets;
    QList<AutoMuteRow> autoMute;
    QList<StreamSinkRow> streamSinks;
    QString notificationSink;
    uint portSwitchCap = 60;
    QString version = "1.4.0-fake";
};

struct FakeCall {
    QString member;
    QString signature;
    QVariantList args;
    qint64 ms = 0;
};

// Shared between the facade (test thread) and the adaptor (worker thread).
struct FakeShared {
    QMutex mutex;
    FakeState state;
    QList<FakeCall> calls;
    QList<QDBusMessage> delayed;
    QElapsedTimer clock;
    std::atomic<int> getAllCount{0};
    std::atomic<bool> holdGetAll{false};
    std::atomic<bool> applyWrites{true};
    std::atomic<bool> delayVolumeReplies{false};
    QSemaphore gate;
    QDBusConnection connection{QString()};
    // Method name -> D-Bus error name the next calls get instead.
    QHash<QString, QString> failures;
};

// QtDBus sets the call context on the registered object, not on its
// adaptors, so the adaptors reach the message through these.
class FakeObject : public QObject, protected QDBusContext {
    Q_OBJECT
public:
    explicit FakeObject(std::shared_ptr<FakeShared> shared, bool legacy);
    std::shared_ptr<FakeShared> shared;

    bool fromDBus() const { return calledFromDBus(); }
    QDBusMessage call() const { return message(); }
    void delayReply() const { setDelayedReply(true); }
    void replyError(const QString &name, const QString &text) const { sendErrorReply(name, text); }
};

class FakeDaemon {
public:
    // `legacy` serves the 1.0 surface only (Devices, Streams,
    // NotificationSink, Version and the 1.0 methods), for the
    // older-daemon tests.
    explicit FakeDaemon(bool legacy = false);
    ~FakeDaemon();

    bool claim();   // take dev.pipedeck.Daemon
    void release(); // give it up, as a daemon exiting would
    // This connection's unique name, the one the client must address.
    QString uniqueName() const { return m_shared->connection.baseService(); }
    // Unexports the object (every call, GetAll included, then fails with
    // UnknownObject) or exports it again, while keeping the name.
    void setServing(bool serving);
    // `member` replies with `errorName` from now on; an empty name stops it.
    void failCalls(const QString &member, const QString &errorName);

    template <typename F> void edit(F &&f) {
        QMutexLocker lock(&m_shared->mutex);
        f(m_shared->state);
    }
    FakeState state() const;
    void populateDemo();

    void emitChanged();
    void emitPropertiesChanged(const QStringList &names);

    QList<FakeCall> calls(const QString &member = {}) const;
    void clearCalls();
    int getAllCount() const { return m_shared->getAllCount; }

    void setHoldGetAll(bool hold) { m_shared->holdGetAll = hold; }
    void releaseGetAll(int replies = 100) { m_shared->gate.release(replies); }
    void setApplyWrites(bool apply) { m_shared->applyWrites = apply; }
    void setDelayVolumeReplies(bool delay) { m_shared->delayVolumeReplies = delay; }
    void failDelayed(const QString &errorName, const QString &message);

    QDBusConnection connection() const { return m_shared->connection; }

private:
    std::shared_ptr<FakeShared> m_shared;
    QThread m_thread;
    FakeObject *m_object = nullptr;
    QString m_connectionName;
};

// The current interface, StreamSinks included.
class FakeAdaptor : public QDBusAbstractAdaptor {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "dev.pipedeck.Daemon1")
    Q_PROPERTY(WireAutoMutes AutoMute READ autoMute)
    Q_PROPERTY(WireDevices Devices READ devices)
    Q_PROPERTY(WireIdStrings Eq READ eq)
    Q_PROPERTY(WirePresets EqPresets READ eqPresets)
    Q_PROPERTY(QString NotificationSink READ notificationSink)
    Q_PROPERTY(uint PortSwitchCap READ portSwitchCap)
    Q_PROPERTY(WirePorts Ports READ ports)
    Q_PROPERTY(WireIdStrings StreamSinks READ streamSinks)
    Q_PROPERTY(WireStreams Streams READ streams)
    Q_PROPERTY(QString Version READ version)

public:
    explicit FakeAdaptor(FakeObject *parent);

    WireAutoMutes autoMute() const;
    WireDevices devices() const;
    WireIdStrings eq() const;
    WirePresets eqPresets() const;
    QString notificationSink() const;
    uint portSwitchCap() const;
    WirePorts ports() const;
    WireIdStrings streamSinks() const;
    WireStreams streams() const;
    QString version() const;

public slots:
    void SetDefault(const QString &kind, const QString &name);
    void SetNotificationSink(const QString &name);
    void SetVolume(uint id, double volume);
    void SetMute(uint id, bool mute);
    void SetStreamTarget(uint id, const QString &name);
    void SetPort(uint node_id, uint route_index);
    void SetAutoMute(uint node_id, bool enabled);
    void SetEq(uint node_id, const QString &preset);
    void SetPortSwitchCap(uint percent);
    void Refresh();

signals:
    // Declared for introspection; FakeDaemon::emitChanged sends it, so it
    // can be fired from the test thread.
    void Changed();

private:
    void record(const char *member, const char *signature, const QVariantList &args);
    // True when the call was answered with an injected error.
    bool rejected(const char *member);
    void changed();
    FakeObject *object() const { return static_cast<FakeObject *>(parent()); }
    std::shared_ptr<FakeShared> m_shared;
};

// The 1.0 surface: no ports, EQ, auto-mute, cap or StreamSinks.
class LegacyAdaptor : public QDBusAbstractAdaptor {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "dev.pipedeck.Daemon1")
    Q_PROPERTY(WireDevices Devices READ devices)
    Q_PROPERTY(QString NotificationSink READ notificationSink)
    Q_PROPERTY(WireStreams Streams READ streams)
    Q_PROPERTY(QString Version READ version)

public:
    explicit LegacyAdaptor(FakeObject *parent);
    WireDevices devices() const;
    QString notificationSink() const;
    WireStreams streams() const;
    QString version() const;

public slots:
    void SetDefault(const QString &kind, const QString &name);
    void SetNotificationSink(const QString &name);
    void SetVolume(uint id, double volume);
    void SetMute(uint id, bool mute);
    void SetStreamTarget(uint id, const QString &name);
    void Refresh();

signals:
    void Changed();

private:
    std::shared_ptr<FakeShared> m_shared;
};
