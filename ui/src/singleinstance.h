#pragma once

#include <QLocalServer>
#include <QLockFile>
#include <QObject>
#include <memory>

// One window per session (SPEC-UI §2). The QLockFile elects the primary; only
// the lock holder listens on the socket, so a stale socket is removed only
// once we know no other instance owns it. A second launch connects, sends
// "show" (with its xdg-activation token, if the launcher gave it one) and
// exits as soon as the primary answers "ok".
class SingleInstance : public QObject {
    Q_OBJECT

public:
    // Empty means $XDG_RUNTIME_DIR (tests pass a temp directory).
    explicit SingleInstance(const QString &directory = {}, QObject *parent = nullptr);
    ~SingleInstance() override;

    // Takes the lock: true when this process is now the primary. It does not
    // listen yet; call listen() once there is a window to show.
    bool acquire();
    // Starts answering second launches. Only after acquire() succeeded.
    bool listen();
    // acquire() and listen() at once.
    bool becomePrimary() { return acquire() && listen(); }

    enum class Claim { Primary, Signalled, Failed };
    // A launch's whole election: take the lock, or hand "show" to the
    // primary; when the primary does not answer within `timeoutMs`, try for
    // the lock again, up to `rounds` times.
    Claim claim(int rounds = 5, int timeoutMs = 1000);
    // Asks the running primary to show itself; true when it answered "ok"
    // within the timeout.
    bool signalPrimary(int timeoutMs = 1000);

    QString socketPath() const { return m_socketPath; }

signals:
    // The activation token may be empty.
    void showRequested(const QString &activationToken);

private:
    void onConnection();

    QString m_socketPath;
    std::unique_ptr<QLockFile> m_lock;
    QLocalServer m_server;
};
