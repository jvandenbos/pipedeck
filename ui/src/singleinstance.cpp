#include "singleinstance.h"

#include <QDir>
#include <QElapsedTimer>
#include <QLocalSocket>
#include <QStandardPaths>
#include <QThread>

SingleInstance::SingleInstance(const QString &directory, QObject *parent) : QObject(parent) {
    QString dir = directory;
    if (dir.isEmpty())
        dir = qEnvironmentVariable("XDG_RUNTIME_DIR");
    if (dir.isEmpty())
        dir = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
    m_socketPath = QDir(dir).filePath("pipedeck-ui.sock");
    m_lock = std::make_unique<QLockFile>(QDir(dir).filePath("pipedeck-ui.lock"));
    // Never stale by age: Qt still breaks a lock whose owning PID is gone,
    // which is the only staleness that matters here.
    m_lock->setStaleLockTime(0);
    connect(&m_server, &QLocalServer::newConnection, this, &SingleInstance::onConnection);
}

SingleInstance::~SingleInstance() {
    m_server.close();
}

bool SingleInstance::becomePrimary() {
    if (!m_lock->tryLock(0))
        return false;
    // We hold the lock, so whatever socket is there belongs to a dead
    // instance. Removing it before that would cut off a live one.
    QLocalServer::removeServer(m_socketPath);
    m_server.setSocketOptions(QLocalServer::UserAccessOption);
    return m_server.listen(m_socketPath);
}

bool SingleInstance::signalPrimary(int timeoutMs) {
    // The primary may hold the lock but not be listening yet; retry
    // connecting until the deadline rather than failing on the first try.
    QElapsedTimer clock;
    clock.start();
    while (clock.elapsed() < timeoutMs) {
        QLocalSocket socket;
        socket.connectToServer(m_socketPath);
        if (socket.waitForConnected(int(qMax<qint64>(1, timeoutMs - clock.elapsed())))) {
            const QByteArray token = qgetenv("XDG_ACTIVATION_TOKEN");
            socket.write("show" + (token.isEmpty() ? QByteArray() : " " + token) + "\n");
            socket.flush();
            while (clock.elapsed() < timeoutMs) {
                if (socket.canReadLine() || socket.waitForReadyRead(int(qMax<qint64>(1, timeoutMs - clock.elapsed()))))
                    if (socket.canReadLine())
                        return socket.readLine().trimmed() == "ok";
                if (socket.state() != QLocalSocket::ConnectedState)
                    break;
            }
            return false;
        }
        QThread::msleep(50);
    }
    return false;
}

void SingleInstance::onConnection() {
    while (QLocalSocket *socket = m_server.nextPendingConnection()) {
        connect(socket, &QLocalSocket::disconnected, socket, &QObject::deleteLater);
        connect(socket, &QLocalSocket::readyRead, this, [this, socket] {
            if (!socket->canReadLine())
                return;
            const QByteArray line = socket->readLine().trimmed();
            if (line == "show" || line.startsWith("show ")) {
                emit showRequested(QString::fromUtf8(line.mid(5)));
                socket->write("ok\n");
            } else {
                socket->write("error\n");
            }
            socket->flush();
            socket->disconnectFromServer();
        });
    }
}
