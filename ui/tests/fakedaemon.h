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
#include <atomic>
#include <memory>

#include "daemon.h"

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
    Q_PROPERTY(QList<AutoMuteRow> AutoMute READ autoMute)
    Q_PROPERTY(QList<DeviceRow> Devices READ devices)
    Q_PROPERTY(QList<EqRow> Eq READ eq)
    Q_PROPERTY(QList<PresetRow> EqPresets READ eqPresets)
    Q_PROPERTY(QString NotificationSink READ notificationSink)
    Q_PROPERTY(uint PortSwitchCap READ portSwitchCap)
    Q_PROPERTY(QList<PortRow> Ports READ ports)
    Q_PROPERTY(QList<StreamSinkRow> StreamSinks READ streamSinks)
    Q_PROPERTY(QList<StreamRow> Streams READ streams)
    Q_PROPERTY(QString Version READ version)

public:
    explicit FakeAdaptor(FakeObject *parent);

    QList<AutoMuteRow> autoMute() const;
    QList<DeviceRow> devices() const;
    QList<EqRow> eq() const;
    QList<PresetRow> eqPresets() const;
    QString notificationSink() const;
    uint portSwitchCap() const;
    QList<PortRow> ports() const;
    QList<StreamSinkRow> streamSinks() const;
    QList<StreamRow> streams() const;
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
    void changed();
    FakeObject *object() const { return static_cast<FakeObject *>(parent()); }
    std::shared_ptr<FakeShared> m_shared;
};

// The 1.0 surface: no ports, EQ, auto-mute, cap or StreamSinks.
class LegacyAdaptor : public QDBusAbstractAdaptor {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "dev.pipedeck.Daemon1")
    Q_PROPERTY(QList<DeviceRow> Devices READ devices)
    Q_PROPERTY(QString NotificationSink READ notificationSink)
    Q_PROPERTY(QList<StreamRow> Streams READ streams)
    Q_PROPERTY(QString Version READ version)

public:
    explicit LegacyAdaptor(FakeObject *parent);
    QList<DeviceRow> devices() const;
    QString notificationSink() const;
    QList<StreamRow> streams() const;
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
