#include <QDBusConnectionInterface>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <atomic>
#include <functional>
#include <QDBusMetaType>
#include <QDBusPendingReply>
#include <QDir>
#include <QDirIterator>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QRegularExpression>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QThread>
#include <QXmlStreamReader>
#include <QtTest>

#include "appicons.h"
#include "backend.h"
#include "daemon.h"
#include "fakedaemon.h"
#include "rowmodel.h"
#include "singleinstance.h"
#include "theme.h"
#include "translator.h"

using Members = QMap<QString, QString>; // "kind:name" -> signature

namespace {

QString key(DaemonProxy::Kind kind, const QString &name) {
    switch (kind) {
    case DaemonProxy::Kind::Method: return "method:" + name;
    case DaemonProxy::Kind::Property: return "property:" + name;
    case DaemonProxy::Kind::Signal: return "signal:" + name;
    }
    return name;
}

// Members of dev.pipedeck.Daemon1 in an introspection document.
Members parseIntrospection(const QByteArray &xml) {
    Members members;
    QXmlStreamReader reader(xml);
    bool inInterface = false;
    QString current;
    while (!reader.atEnd()) {
        reader.readNext();
        if (reader.isStartElement()) {
            const auto attrs = reader.attributes();
            if (reader.name() == u"interface")
                inInterface = attrs.value("name") == QLatin1String(DaemonProxy::Interface);
            else if (!inInterface)
                continue;
            else if (reader.name() == u"method" || reader.name() == u"signal") {
                current = reader.name().toString() + ":" + attrs.value("name").toString();
                members[current] = QString();
            } else if (reader.name() == u"arg" && !current.isEmpty()) {
                // Signal args have no direction; method args count when "in".
                if (current.startsWith("signal:") || attrs.value("direction") != u"out")
                    members[current] += attrs.value("type").toString();
            } else if (reader.name() == u"property") {
                members["property:" + attrs.value("name").toString()] = attrs.value("type").toString();
            }
        } else if (reader.isEndElement()) {
            if (reader.name() == u"method" || reader.name() == u"signal")
                current.clear();
            else if (reader.name() == u"interface")
                inInterface = false;
        }
    }
    return members;
}

QByteArray readFile(const QString &path) {
    QFile file(path);
    return file.open(QIODevice::ReadOnly) ? file.readAll() : QByteArray();
}

void write(const QString &path, const QByteArray &contents) {
    QFile file(path);
    QVERIFY(file.open(QIODevice::WriteOnly));
    file.write(contents);
}

int rowOf(RowModel *model, uint id) {
    return model->indexOf(QVariant::fromValue(id));
}

QVariant field(RowModel *model, uint id, const QString &name) {
    const int row = rowOf(model, id);
    return row < 0 ? QVariant() : model->rows().at(row).value(name);
}

// Lets the event loop run for `ms`, so nothing is decided by a single tick.
void settle(int ms) {
    QTest::qWait(ms);
}

} // namespace

// The real Main.qml over a backend, offscreen (ui/bin/test sets the
// platform), tall enough that every control is on screen to be clicked.
struct Window {
    Translator translator;
    Theme theme;
    QQmlApplicationEngine engine;
    QQuickWindow *window = nullptr;

    Window(const QString &dir, Backend *backend)
        : translator(dir + "/language", dir), theme(dir + "/no-omarchy", nullptr, dir + "/no-hyprctl") {
        engine.addImageProvider("appicon", new AppIconProvider);
        engine.rootContext()->setContextProperty("translator", &translator);
        engine.rootContext()->setContextProperty("theme", &theme);
        engine.rootContext()->setContextProperty("backend", backend);
        engine.load(QUrl("qrc:/Main.qml"));
        window = qobject_cast<QQuickWindow *>(engine.rootObjects().value(0));
    }
    // Repeater delegates have a parent item but no QObject parent, so the
    // item tree is walked rather than findChild().
    static QQuickItem *walk(QQuickItem *item, const std::function<bool(QQuickItem *)> &match) {
        if (match(item))
            return item;
        for (QQuickItem *child : item->childItems())
            if (QQuickItem *found = walk(child, match))
                return found;
        return nullptr;
    }
    QQuickItem *find(const QString &name) const {
        return walk(window->contentItem(), [&](QQuickItem *i) { return i->objectName() == name; });
    }
    QQuickItem *flickable() const {
        return walk(window->contentItem(), [](QQuickItem *i) {
            return i->inherits("QQuickFlickable") && !i->inherits("QQuickListView") && i->height() > 100;
        });
    }
    void click(QQuickItem *item, QPointF at = {}) const {
        if (at.isNull())
            at = QPointF(item->width() / 2, item->height() / 2);
        QTest::mouseClick(window, Qt::LeftButton, {}, item->mapToScene(at).toPoint());
    }
};

class PipedeckUiTests : public QObject {
    Q_OBJECT

private:
    QTemporaryDir m_dir;
    QString settingsFile() const { return m_dir.filePath("settings.ini"); }

    // A connected backend over a claimed fake with the demo graph.
    std::unique_ptr<Backend> connectedBackend(FakeDaemon &fake) {
        if (fake.state().devices.isEmpty())
            fake.populateDemo();
        [&] { QVERIFY(fake.claim()); }();
        auto backend = std::make_unique<Backend>(QDBusConnection::sessionBus(), settingsFile(),
                                                 std::make_shared<AppIcons>(QStringList{m_dir.path()}, [](const QString &) { return true; }));
        [&] { QTRY_VERIFY(backend->connected()); }();
        return backend;
    }

private slots:
    void initTestCase() {
        QVERIFY(QDBusConnection::sessionBus().isConnected());
        // Refuse to run against a real daemon on a real session bus.
        QVERIFY2(!QDBusConnection::sessionBus().interface()->isServiceRegistered(DaemonProxy::Service),
                 "a pipedeckd is on this bus; run under dbus-run-session (ui/bin/test does)");
        DaemonProxy::registerTypes();
        QQuickStyle::setStyle("Material");
    }

    // ---- the contract -----------------------------------------------------

    void proxyCoversTheCheckedInXml() {
        const QByteArray xml = readFile(PIPEDECK_XML);
        QVERIFY2(!xml.isEmpty(), PIPEDECK_XML);
        const Members contract = parseIntrospection(xml);
        QVERIFY(contract.size() >= 20);

        Members proxy;
        for (const auto &m : DaemonProxy::members())
            proxy[key(m.kind, m.name)] = m.signature;

        for (auto it = contract.begin(); it != contract.end(); ++it) {
            QVERIFY2(proxy.contains(it.key()), qPrintable("proxy lacks " + it.key()));
            QCOMPARE(proxy.value(it.key()), it.value());
        }
        // The only member the proxy may know beyond the XML is the one added
        // with SPEC-UI §6.1, so the test passes before and after it lands.
        for (auto it = proxy.begin(); it != proxy.end(); ++it)
            if (!contract.contains(it.key()))
                QCOMPARE(it.key(), QString("property:StreamSinks"));
    }

    void rowTypesMarshalWithTheXmlSignatures() {
        // What the stream operators actually write, not what members() claims.
        const QMap<QString, QMetaType> types{
            {"AutoMute", QMetaType::fromType<QList<AutoMuteRow>>()},
            {"Devices", QMetaType::fromType<QList<DeviceRow>>()},
            {"Eq", QMetaType::fromType<QList<EqRow>>()},
            {"EqPresets", QMetaType::fromType<QList<PresetRow>>()},
            {"Ports", QMetaType::fromType<QList<PortRow>>()},
            {"StreamSinks", QMetaType::fromType<QList<StreamSinkRow>>()},
            {"Streams", QMetaType::fromType<QList<StreamRow>>()},
        };
        for (const auto &m : DaemonProxy::members()) {
            if (m.kind != DaemonProxy::Kind::Property || !types.contains(m.name))
                continue;
            QCOMPARE(QString::fromLatin1(QDBusMetaType::typeToSignature(types.value(m.name))), m.signature);
        }
    }

    void fakeDaemonServesTheXml() {
        // The fake is only evidence if it speaks the real interface.
        FakeDaemon fake;
        QVERIFY(fake.claim());
        auto message = QDBusMessage::createMethodCall(DaemonProxy::Service, DaemonProxy::Path,
                                                      "org.freedesktop.DBus.Introspectable", "Introspect");
        QDBusPendingReply<QString> reply = QDBusConnection::sessionBus().asyncCall(message);
        QTRY_VERIFY(reply.isFinished());
        QVERIFY2(!reply.isError(), qPrintable(reply.error().message()));
        const Members served = parseIntrospection(reply.value().toUtf8());
        const Members contract = parseIntrospection(readFile(PIPEDECK_XML));
        for (auto it = contract.begin(); it != contract.end(); ++it) {
            QVERIFY2(served.contains(it.key()), qPrintable("fake lacks " + it.key()));
            QCOMPARE(served.value(it.key()), it.value());
        }
    }

    // ---- decoding and refresh ----------------------------------------------

    void decodesEveryPropertyOnTheWire() {
        FakeDaemon fake;
        // Every field distinct and both bool values present, so a swapped or
        // misaligned field in an operator cannot pass.
        fake.edit([](FakeState &s) {
            s.devices = {{7, "sink.a", "Désc A", "sink", true, false, 0.25, true, "Nick A"},
                         {9, "src.b", "Desc B", "source", false, true, 2.5, false, "Nick B"}};
            s.streams = {{30, "Ap☃p", "bin", "Média", "sink.a", 0.5, true},
                         {31, "", "bin2", "", "", 3.375, false}};
            s.ports = {{7, 3, "port-x", "Port X", true, false}, {7, 4, "port-y", "Port Y", false, true}};
            s.eq = {{7, "preset-1"}, {8, ""}};
            s.eqPresets = {{"preset-1", "Preset One"}, {"p2", "Two"}};
            s.autoMute = {{7, true}, {8, false}};
            s.streamSinks = {{30, "sink.a"}, {31, ""}};
            s.notificationSink = "sink.a";
            s.portSwitchCap = 42;
            s.version = "9.8.7";
        });
        auto backend = connectedBackend(fake);
        const FakeState want = fake.state();
        const Snapshot &got = backend->snapshot();
        QCOMPARE(got.devices, want.devices);
        QCOMPARE(got.streams, want.streams);
        QCOMPARE(got.ports, want.ports);
        QCOMPARE(got.eq, want.eq);
        QCOMPARE(got.eqPresets, want.eqPresets);
        QCOMPARE(got.autoMute, want.autoMute);
        QCOMPARE(got.streamSinks, want.streamSinks);
        QCOMPARE(got.notificationSink, want.notificationSink);
        QCOMPARE(got.portSwitchCap, want.portSwitchCap);
        QCOMPARE(got.version, want.version);
        QCOMPARE(backend->version(), QString("9.8.7"));
        for (const auto &name : {"Devices", "Streams", "Ports", "Eq", "EqPresets", "AutoMute", "StreamSinks",
                                 "NotificationSink", "PortSwitchCap", "Version"})
            QVERIFY2(got.has(name), name);
    }

    void snapshotThenSignalsRefresh() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        QCOMPARE(field(backend->outputs(), 58, "nick").toString(), QString("Laptop speakers"));

        // The daemon's own Changed signal...
        fake.edit([](FakeState &s) { s.devices[0].nick = "Renamed once"; });
        fake.emitChanged();
        QTRY_COMPARE(field(backend->outputs(), 58, "nick").toString(), QString("Renamed once"));

        // ...and a bare PropertiesChanged both lead to a re-read.
        fake.edit([](FakeState &s) { s.notificationSink = "alsa_output.usb-Fiio_K3-00.analog-stereo"; });
        fake.emitPropertiesChanged({"NotificationSink"});
        QTRY_COMPARE(backend->notificationSink(), QString("alsa_output.usb-Fiio_K3-00.analog-stereo"));
    }

    void refreshIsOneInFlightWithATrailingRead() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        settle(150);
        const int before = backend->refreshCount();

        fake.setHoldGetAll(true);
        fake.emitChanged();
        QTRY_COMPARE(backend->refreshCount(), before + 1);
        QTRY_COMPARE(fake.getAllCount(), before + 1); // the daemon is now stuck in it

        // A storm of both signal kinds while that read is unanswered.
        for (int i = 0; i < 10; ++i) {
            fake.emitChanged();
            fake.emitPropertiesChanged({"Devices", "Streams"});
        }
        settle(300);
        QCOMPARE(backend->refreshCount(), before + 1);

        // The storm is folded into exactly one trailing read, which sees the
        // state as of after the storm.
        fake.edit([](FakeState &s) { s.devices[0].nick = "After the storm"; });
        fake.setHoldGetAll(false);
        fake.releaseGetAll();
        QTRY_COMPARE(field(backend->outputs(), 58, "nick").toString(), QString("After the storm"));
        settle(300);
        QCOMPARE(backend->refreshCount(), before + 2);
    }

    void refreshIsCappedAtTenPerSecond() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        settle(150);
        const int before = backend->refreshCount();
        QElapsedTimer clock;
        clock.start();
        while (clock.elapsed() < 1000) {
            fake.emitChanged();
            QTest::qWait(5);
        }
        settle(150);
        // A read at the start, then at most one per 100 ms.
        QVERIFY2(backend->refreshCount() - before <= 12, qPrintable(QString::number(backend->refreshCount() - before)));
        QVERIFY(backend->refreshCount() - before >= 3); // periodic, not one read for the whole storm
    }

    // ---- owner generations and reconnect -------------------------------------

    void staleRepliesFromAReplacedOwnerAreDropped() {
        auto first = std::make_unique<FakeDaemon>();
        first->populateDemo();
        auto backend = connectedBackend(*first);
        settle(150);
        const quint64 generation = backend->generation();

        // A read goes to the first owner and is held there...
        first->setHoldGetAll(true);
        first->edit([](FakeState &s) { s.devices[0].nick = "Stale"; });
        first->emitChanged();
        QTRY_COMPARE(first->getAllCount(), 2);

        // ...while the name moves to a second daemon with new node ids.
        first->release();
        FakeDaemon second;
        second.edit([](FakeState &s) {
            s.devices = {{500, "sink.new", "New", "sink", true, false, 1.0, false, "Fresh"}};
        });
        QVERIFY(second.claim());
        QTRY_COMPARE(field(backend->outputs(), 500, "nick").toString(), QString("Fresh"));
        QVERIFY(backend->generation() >= generation + 2);

        // The first daemon's late reply must not resurrect its graph.
        first->setHoldGetAll(false);
        first->releaseGetAll();
        settle(300);
        QCOMPARE(backend->outputs()->rowCount(), 1);
        QCOMPARE(field(backend->outputs(), 500, "nick").toString(), QString("Fresh"));
        QCOMPARE(rowOf(backend->outputs(), 58), -1);
        first.reset();
    }

    void unsentWritesAreDroppedOnLossAndNotReplayed() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        fake.setApplyWrites(false);

        backend->setVolume(58, 40, true); // sent at once
        backend->setVolume(58, 60, true); // queued behind the 50 ms gap
        fake.release();
        QTRY_COMPARE(backend->state(), QString("absent"));
        QVERIFY(!backend->connected());
        settle(200);
        QCOMPARE(fake.calls("SetVolume").size(), 1);

        // The daemon comes back with new node ids: nothing old is replayed,
        // and the new ids are what the window uses.
        fake.edit([](FakeState &s) {
            for (auto &d : s.devices)
                d.id += 1000;
        });
        QVERIFY(fake.claim());
        QTRY_VERIFY(backend->connected());
        settle(300);
        QCOMPARE(fake.calls("SetVolume").size(), 1);
        QVERIFY(rowOf(backend->outputs(), 1058) >= 0);
        QCOMPARE(rowOf(backend->outputs(), 58), -1);
        backend->setVolume(1058, 50, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 2);
        QCOMPARE(fake.calls("SetVolume").last().args.at(0).toUInt(), 1058u);
    }

    void writesToAVanishedNodeAreSilent() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        settle(150);
        fake.setApplyWrites(false);

        // Queued, then the node disappears before the queue drains.
        backend->setVolume(71, 40, true);
        backend->setVolume(71, 41, true);
        fake.edit([](FakeState &s) { s.devices.removeAt(1); });
        fake.emitChanged();
        QTRY_COMPARE(rowOf(backend->outputs(), 71), -1);
        settle(150);
        QCOMPARE(fake.calls("SetVolume").size(), 1);

        // A write already out when the node went: its NotFound is not shown.
        fake.setDelayVolumeReplies(true);
        backend->setVolume(74, 30, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 2);
        fake.edit([](FakeState &s) { s.devices.removeAt(1); });
        fake.emitChanged();
        QTRY_COMPARE(rowOf(backend->outputs(), 74), -1);
        fake.failDelayed("dev.pipedeck.Error.NotFound", "no node 74");
        settle(150);
        QCOMPARE(backend->lastError(), QString());

        // An error for a node that is still there is shown, with its name.
        backend->setVolume(58, 30, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 3);
        fake.failDelayed("dev.pipedeck.Error.PipeWire", "route write failed");
        QTRY_COMPARE(backend->lastError(), QString("dev.pipedeck.Error.PipeWire: route write failed"));
        // ...and cleared by the next successful call.
        backend->setMute(58, true);
        QTRY_COMPARE(backend->lastError(), QString());
    }

    // ---- writes -------------------------------------------------------------

    void volumeContract() {
        QCOMPARE(Backend::percentToLinear(50), 0.125);
        QCOMPARE(Backend::percentToLinear(100), 1.0);
        QCOMPARE(Backend::percentToLinear(150), 3.375);
        QCOMPARE(Backend::percentToLinear(0), 0.0);
        QVERIFY(qAbs(Backend::linearToPercent(0.125) - 50) < 1e-9);
        QVERIFY(qAbs(Backend::linearToPercent(1.0) - 100) < 1e-9);
        QVERIFY(qAbs(Backend::linearToPercent(3.375) - 150) < 1e-9);

        FakeDaemon fake;
        fake.populateDemo();
        fake.edit([](FakeState &s) { s.devices[1].volume = 3.375; }); // overdrive set elsewhere
        auto backend = connectedBackend(fake);
        // Shown as 150 % even with the slider capped at 100 %; nothing is
        // written back to clamp it.
        QCOMPARE(backend->maxPercent(), 100);
        QVERIFY(qAbs(field(backend->outputs(), 71, "percent").toDouble() - 150) < 1e-6);
        settle(200);
        QCOMPARE(fake.calls("SetVolume").size(), 0);

        backend->setVolume(58, 50, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 1);
        QCOMPARE(fake.calls("SetVolume").last().args.at(1).toDouble(), 0.125);
        backend->setAllowOverdrive(true);
        QCOMPARE(backend->maxPercent(), 150);
        settle(60);
        backend->setVolume(58, 150, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 2);
        QCOMPARE(fake.calls("SetVolume").last().args.at(1).toDouble(), 3.375);
        QCOMPARE(fake.calls("SetVolume").last().signature, QString("ud"));
        // The toggle is a UI setting that survives a restart.
        QCOMPARE(QSettings(settingsFile(), QSettings::IniFormat).value("ui/allowOverdrive").toBool(), true);
        backend->setAllowOverdrive(false);
    }

    void sliderDragIsCoalescedAndNeverWritesMute() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        QSignalSpy sent(backend.get(), &Backend::volumeSent);
        QList<qint64> times;
        QElapsedTimer clock;
        clock.start();
        connect(backend.get(), &Backend::volumeSent, this, [&] { times << clock.elapsed(); });

        // 60 Hz for a second; halfway through, a media key mutes the sink.
        double percent = 10;
        int inputs = 0;
        while (clock.elapsed() < 1000) {
            backend->setVolume(58, percent, true);
            ++inputs;
            percent += 1;
            if (clock.elapsed() > 500 && !fake.state().devices[0].mute) {
                fake.edit([](FakeState &s) { s.devices[0].mute = true; });
                fake.emitChanged();
            }
            QTest::qWait(16);
        }
        backend->setVolume(58, percent, false);
        settle(200);

        // <= 20 per second in any window, measured where the writes leave.
        // (1 ms of slack: this clock is read just after the backend's own.)
        for (int i = 0; i + 20 < times.size(); ++i)
            QVERIFY2(times.at(i + 20) - times.at(i) >= 999,
                     qPrintable(QString("21 writes in %1 ms").arg(times.at(i + 20) - times.at(i))));
        // ...and the drag is not starved: writes keep leaving while it lasts.
        // A slow machine delivers fewer inputs, and each one that comes 50 ms
        // after the last write goes out at once, so the floor scales with the
        // inputs that actually arrived instead of with the wall clock.
        QVERIFY2(sent.size() >= qMin(15, inputs / 4) && sent.size() >= 3,
                 qPrintable(QString("%1 writes for %2 inputs").arg(sent.size()).arg(inputs)));

        const auto writes = fake.calls("SetVolume");
        QCOMPARE(writes.size(), sent.size());
        for (const auto &w : writes)
            QCOMPARE(w.signature, QString("ud")); // no mute in a volume write
        QCOMPARE(writes.last().args.at(1).toDouble(), Backend::percentToLinear(percent)); // the release
        QCOMPARE(fake.calls("SetMute").size(), 0);
        QVERIFY(fake.state().devices[0].mute); // the media key's mute stands
        QTRY_VERIFY(field(backend->outputs(), 58, "mute").toBool());
    }

    void muteIsItsOwnWrite() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        backend->setMute(121, false);
        QTRY_COMPARE(fake.calls("SetMute").size(), 1);
        const auto call = fake.calls("SetMute").first();
        QCOMPARE(call.signature, QString("ub"));
        QCOMPARE(call.args, (QVariantList{121u, false}));
        QCOMPARE(fake.calls("SetVolume").size(), 0);
    }

    void releasedSliderOutlivesOlderEchoes() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        settle(150);
        fake.setApplyWrites(false);
        auto setObserved = [&](double percent) {
            fake.edit([percent](FakeState &s) { s.devices[0].volume = Backend::percentToLinear(percent); });
            fake.emitChanged();
        };
        auto shown = [&] { return field(backend->outputs(), 58, "percent").toDouble(); };
        // Every value the row shows, with the time since `clock` started.
        // Assertions are about order and lower bounds, never about how much
        // happens inside a wall-clock window, so a slow machine cannot fail
        // them; it can only make the hold expire earlier relative to the
        // checks, which the bounds account for.
        QElapsedTimer clock;
        QList<QPair<qint64, int>> seen;
        const auto recorder = connect(backend->outputs(), &QAbstractItemModel::dataChanged, this,
                                      [&] { seen << qMakePair(clock.elapsed(), qRound(shown())); });

        clock.start();
        backend->setVolume(58, 30, true);
        settle(60);
        seen.clear();
        clock.restart();
        backend->setVolume(58, 70, false); // release at 70
        QCOMPARE(shown(), 70.0);

        // The daemon reports the intermediate 30 first: it must not win
        // while the hold lasts.
        int reads = backend->refreshCount();
        setObserved(30);
        QTRY_VERIFY(backend->refreshCount() > reads);
        settle(50);
        for (const auto &[t, value] : seen)
            if (t < Backend::HoldMs)
                QCOMPARE(value, 70);
        if (clock.elapsed() < Backend::HoldMs)
            QCOMPARE(qRound(shown()), 70);

        // The echo arrives; from then on the last observed value wins at once
        // and stays.
        setObserved(70);
        QTRY_COMPARE(qRound(shown()), 70);
        settle(120);
        setObserved(20);
        QTRY_COMPARE(qRound(shown()), 20);
        QVERIFY2(!QTest::qWaitFor([&] { return qRound(shown()) != 20; }, 250), "the observed 20 did not stay");

        // No echo at all: the final value holds for HoldMs, then observed wins.
        seen.clear();
        clock.restart();
        backend->setVolume(58, 80, false);
        QCOMPARE(shown(), 80.0);
        setObserved(20);
        QTRY_COMPARE_WITH_TIMEOUT(qRound(shown()), 20, 2000);
        qint64 switched = -1;
        for (const auto &[t, value] : seen) {
            if (value == 20) {
                switched = t;
                break;
            }
            QCOMPARE(value, 80);
        }
        QVERIFY2(switched >= Backend::HoldMs, qPrintable(QString("observed won after %1 ms").arg(switched)));
        disconnect(recorder);
    }

    // Codex 1: the dedupe must not swallow a write after someone else moved
    // the level (50 -> media key 80 -> back to 50).
    void aWriteAfterAnExternalChangeIsSent() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        settle(150);
        backend->setVolume(58, 50, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 1);
        QTRY_COMPARE(qRound(field(backend->outputs(), 58, "percent").toDouble()), 50);
        settle(Backend::HoldMs + 100);

        fake.edit([](FakeState &s) { s.devices[0].volume = Backend::percentToLinear(80); });
        fake.emitChanged();
        QTRY_COMPARE(qRound(field(backend->outputs(), 58, "percent").toDouble()), 80);
        backend->setVolume(58, 50, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 2);
        QCOMPARE(fake.calls("SetVolume").last().args.at(1).toDouble(), 0.125);

        // The same while the hold is still up: the media key lands between
        // the write and its echo (the daemon does not apply ours here).
        settle(Backend::HoldMs + 100);
        fake.setApplyWrites(false);
        backend->setVolume(58, 30, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 3);
        fake.edit([](FakeState &s) { s.devices[0].volume = Backend::percentToLinear(80); });
        fake.emitChanged();
        const int reads = backend->refreshCount();
        QTRY_VERIFY(backend->refreshCount() > reads);
        settle(30);
        backend->setVolume(58, 30, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 4);
        QCOMPARE(fake.calls("SetVolume").last().args.at(1).toDouble(), Backend::percentToLinear(30));
    }

    // Opus 5: a press and release that moved nothing is not a write, and the
    // end of the slider is not a value to write back for a level beyond it.
    void aClickWithoutMovementSendsNothing() {
        FakeDaemon fake;
        fake.populateDemo();
        fake.edit([](FakeState &s) { s.devices[1].volume = 3.375; }); // 150 %, set elsewhere
        auto backend = connectedBackend(fake);
        QCOMPARE(backend->maxPercent(), 100);
        settle(150);
        backend->setVolume(71, 100, false); // where the slider draws 150 %
        backend->setVolume(71, 100, true);
        backend->setVolume(58, 70, false); // 58 is at 70 % already
        settle(200);
        QCOMPARE(fake.calls("SetVolume").size(), 0);
        QVERIFY(qAbs(field(backend->outputs(), 71, "percent").toDouble() - 150) < 1e-6);
        // A real move from there is the user's choice and is sent.
        backend->setVolume(71, 90, false);
        QTRY_COMPARE(fake.calls("SetVolume").size(), 1);
        QCOMPARE(fake.calls("SetVolume").last().args.at(1).toDouble(), Backend::percentToLinear(90));
    }

    // Opus 14: a slider move touches one role of one row.
    void aSliderMoveUpdatesOnlyItsRow() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        settle(150);
        fake.setApplyWrites(false);
        QSignalSpy outputs(backend->outputs(), &QAbstractItemModel::dataChanged);
        QSignalSpy inputs(backend->inputs(), &QAbstractItemModel::dataChanged);
        QSignalSpy streams(backend->streams(), &QAbstractItemModel::dataChanged);
        backend->setVolume(71, 40, true);
        QCOMPARE(outputs.size(), 1);
        const QModelIndex row = outputs.first().at(0).toModelIndex();
        QCOMPARE(row.row(), rowOf(backend->outputs(), 71));
        const auto roles = outputs.first().at(2).value<QList<int>>();
        QCOMPARE(roles.size(), 1);
        QCOMPARE(backend->outputs()->roleNames().value(roles.first()), QByteArray("percent"));
        QCOMPARE(inputs.size(), 0);
        QCOMPARE(streams.size(), 0);
        backend->setVolume(118, 40, true); // a stream row, same rule
        QCOMPARE(streams.size(), 1);
        QCOMPARE(outputs.size(), 1);
    }

    void destinationPicker() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        QCOMPARE(field(backend->streams(), 118, "target").toString(), QString("alsa_output.usb-Fiio_K3-00.analog-stereo"));
        QCOMPARE(field(backend->streams(), 118, "targetLabel").toString(), QString("FiiO K3"));
        QCOMPARE(field(backend->streams(), 118, "showObserved").toBool(), false);
        // mpv asked for HDMI but plays on the speakers: requested and observed.
        QCOMPARE(field(backend->streams(), 121, "showObserved").toBool(), true);
        QCOMPARE(field(backend->streams(), 121, "observedLabel").toString(), QString("Laptop speakers"));
        // Following the default and playing there: nothing extra to say.
        QCOMPARE(field(backend->streams(), 103, "target").toString(), QString());
        QCOMPARE(field(backend->streams(), 103, "showObserved").toBool(), false);

        backend->setStreamTarget(103, "alsa_output.pci-0000_07_00.1.hdmi-stereo");
        QTRY_COMPARE(fake.calls("SetStreamTarget").size(), 1);
        QCOMPARE(fake.calls("SetStreamTarget").last().signature, QString("us"));
        QCOMPARE(fake.calls("SetStreamTarget").last().args,
                 (QVariantList{103u, QString("alsa_output.pci-0000_07_00.1.hdmi-stereo")}));
        backend->followDefault(103);
        QTRY_COMPARE(fake.calls("SetStreamTarget").size(), 2);
        QCOMPARE(fake.calls("SetStreamTarget").last().args, (QVariantList{103u, QString()}));

        // The requested sink is unplugged: the request stays, marked absent,
        // and the row says where the stream really is.
        fake.edit([](FakeState &s) {
            s.devices.removeAt(1);
            s.streamSinks[1].sink = "alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink";
        });
        fake.emitChanged();
        QTRY_VERIFY(field(backend->streams(), 118, "targetAbsent").toBool());
        QCOMPARE(field(backend->streams(), 118, "targetLabel").toString(), QString("alsa_output.usb-Fiio_K3-00.analog-stereo"));
        QVERIFY(field(backend->streams(), 118, "showObserved").toBool());
    }

    void otherSettersSendWhatTheXmlSays() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        backend->setDefault("sink", "alsa_output.usb-Fiio_K3-00.analog-stereo");
        backend->setNotificationSink("");
        backend->setPort(58, 1);
        backend->setAutoMute(58, true);
        backend->setEq(58, "bass-lift");
        backend->setPortSwitchCap(75);
        backend->refresh();
        QTRY_VERIFY(fake.calls("SetPortSwitchCap").size() == 1);
        QCOMPARE(fake.calls("SetDefault").last().args, (QVariantList{"sink", "alsa_output.usb-Fiio_K3-00.analog-stereo"}));
        QCOMPARE(fake.calls("SetNotificationSink").last().args, (QVariantList{QString()}));
        QCOMPARE(fake.calls("SetPort").last().args, (QVariantList{58u, 1u}));
        QCOMPARE(fake.calls("SetAutoMute").last().args, (QVariantList{58u, true}));
        QCOMPARE(fake.calls("SetEq").last().args, (QVariantList{58u, "bass-lift"}));
        QCOMPARE(fake.calls("SetPortSwitchCap").last().args, (QVariantList{75u}));
        QCOMPARE(fake.calls("SetPortSwitchCap").last().signature, QString("u"));
        QTRY_COMPARE(fake.calls("Refresh").size(), 1);
        QTRY_COMPARE(field(backend->outputs(), 58, "eqName").toString(), QString("Bass lift"));
        QTRY_VERIFY(field(backend->outputs(), 58, "autoMute").toBool());
    }

    void modelRowsCarryWhatTheWindowShows() {
        FakeDaemon fake;
        fake.populateDemo();
        fake.edit([](FakeState &s) {
            s.streams << StreamRow{140, "", "", "", "", 1.0, false} << StreamRow{141, "", "aplay", "", "", 1.0, false}
                      << StreamRow{142, "Firefox", "firefox", "", "", 1.0, false};
        });
        write(m_dir.filePath("org.mozilla.firefox.desktop"),
              "[Desktop Entry]\nName=Firefox\nExec=/usr/lib/firefox/firefox %u\nIcon=firefox\n");
        auto backend = connectedBackend(fake);
        QCOMPARE(field(backend->streams(), 103, "label").toString(), QString("Firefox — Lo-fi radio — live"));
        QCOMPARE(field(backend->streams(), 140, "label").toString(), QString("#140"));
        QCOMPARE(field(backend->streams(), 141, "label").toString(), QString("aplay"));
        QCOMPARE(field(backend->streams(), 142, "label").toString(), QString("Firefox"));
        QCOMPARE(field(backend->streams(), 103, "icon").toString(), QString("firefox"));
        QCOMPARE(field(backend->streams(), 118, "icon").toString(), QString());
        QFile::remove(m_dir.filePath("org.mozilla.firefox.desktop")); // later windows load no icon

        // Two available ports on the speaker sink make a selector; one does not.
        QVERIFY(field(backend->outputs(), 58, "showPorts").toBool());
        QVERIFY(!field(backend->outputs(), 74, "showPorts").toBool());
        QVERIFY(field(backend->outputs(), 58, "hasAutoMute").toBool());
        QVERIFY(!field(backend->outputs(), 71, "hasAutoMute").toBool());
        QCOMPARE(field(backend->outputs(), 71, "eqName").toString(), QString("Harman over-ear 2019"));
        QCOMPARE(backend->inputs()->rowCount(), 2);
        QCOMPARE(backend->sinkChoices().size(), 3);
    }

    void olderDaemonHidesWhatItLacks() {
        FakeDaemon fake(true);
        fake.populateDemo();
        auto backend = connectedBackend(fake);
        const auto f = backend->features();
        QVERIFY(f.value("devices").toBool());
        QVERIFY(f.value("streams").toBool());
        QVERIFY(f.value("notifications").toBool());
        QVERIFY(!f.value("streamSinks").toBool());
        QVERIFY(!f.value("ports").toBool());
        QVERIFY(!f.value("eq").toBool());
        QVERIFY(!f.value("autoMute").toBool());
        QVERIFY(!f.value("portSwitchCap").toBool());
        QCOMPARE(backend->version(), QString("1.0.0"));
        QVERIFY(!field(backend->outputs(), 58, "showPorts").toBool());
        QVERIFY(!field(backend->streams(), 121, "showObserved").toBool());

        // A method the daemon does not have hides its control, quietly.
        QVERIFY(f.value("refresh").toBool());
        QTest::ignoreMessage(QtWarningMsg, QRegularExpression("couldn't handle call to SetAutoMute"));
        QTest::ignoreMessage(QtWarningMsg, QRegularExpression("couldn't handle call to SetAutoMute"));
        QTest::ignoreMessage(QtWarningMsg, QRegularExpression("Could not find slot LegacyAdaptor::SetAutoMute"));
        backend->setAutoMute(58, true);
        QTRY_VERIFY(!backend->features().value("autoMute").toBool());
        settle(100);
        QCOMPARE(backend->lastError(), QString());
    }

    // Opus 3: before StreamSinks, target_name is where the stream plays.
    void olderDaemonPickerShowsTheObservedSink() {
        FakeDaemon fake(true);
        fake.populateDemo();
        fake.edit([](FakeState &s) {
            // With EQ on, a 1.3.0 daemon reports the chain's node.
            s.streams[1].targetName = "pipedeck.eq.alsa_output.usb-Fiio_K3-00.analog-stereo";
            s.streams[2].targetName = "alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink";
        });
        auto backend = connectedBackend(fake);
        QVERIFY(!backend->features().value("streamSinks").toBool());
        QCOMPARE(field(backend->streams(), 118, "target").toString(), QString("alsa_output.usb-Fiio_K3-00.analog-stereo"));
        QCOMPARE(field(backend->streams(), 118, "targetLabel").toString(), QString("FiiO K3"));
        QCOMPARE(field(backend->streams(), 121, "target").toString(),
                 QString("alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink"));
        QCOMPARE(field(backend->streams(), 103, "target").toString(), QString());
        for (const uint id : {103u, 118u, 121u}) {
            QVERIFY(!field(backend->streams(), id, "targetAbsent").toBool());
            QVERIFY(!field(backend->streams(), id, "showObserved").toBool());
        }
        // A link to a sink the window does not list is still not "absent".
        fake.edit([](FakeState &s) { s.streams[0].targetName = "some.other.sink"; });
        fake.emitChanged();
        QTRY_COMPARE(field(backend->streams(), 103, "target").toString(), QString("some.other.sink"));
        QVERIFY(!field(backend->streams(), 103, "targetAbsent").toBool());
    }

    // Opus 4 / Codex 4: what one daemon lacked is forgotten with it.
    void missingMethodsAreForgottenOnOwnerChange() {
        auto old = std::make_unique<FakeDaemon>();
        old->failCalls("SetEq", "org.freedesktop.DBus.Error.UnknownMethod");
        auto backend = connectedBackend(*old);
        QVERIFY(backend->features().value("eq").toBool());
        backend->setEq(58, "bass-lift");
        QTRY_VERIFY(!backend->features().value("eq").toBool());
        QCOMPARE(backend->lastError(), QString());

        // Upgraded and restarted: the new owner has SetEq.
        old->release();
        FakeDaemon upgraded;
        upgraded.populateDemo();
        QVERIFY(upgraded.claim());
        QTRY_COMPARE(backend->owner(), upgraded.uniqueName());
        QTRY_VERIFY(backend->connected());
        QVERIFY(backend->features().value("eq").toBool());
        backend->setEq(58, "bass-lift");
        QTRY_COMPARE(upgraded.calls("SetEq").size(), 1);
        old.reset();
    }

    // Opus 7 + Codex 7: a failed first read is retried with backoff, and the
    // read that succeeds clears the error it left.
    void aFailedSnapshotIsRetriedAndClearsItsError() {
        FakeDaemon fake;
        fake.populateDemo();
        fake.setServing(false); // the name is owned, the object is not there
        QVERIFY(fake.claim());
        Backend backend(QDBusConnection::sessionBus(), settingsFile());
        QElapsedTimer clock;
        clock.start();
        QList<qint64> reads;
        int count = 0;
        while (reads.size() < 3 && clock.elapsed() < 10000) {
            if (backend.refreshCount() != count) {
                count = backend.refreshCount();
                reads << clock.elapsed();
            }
            QTest::qWait(5);
        }
        QCOMPARE(reads.size(), 3);
        QCOMPARE(backend.state(), QString("connecting"));
        QVERIFY2(backend.lastError().startsWith("org.freedesktop.DBus.Error.UnknownObject"),
                 qPrintable(backend.lastError()));
        // Lower bounds are the timers (a coarse QTimer may fire up to 5 %
        // early, so 10 % slack); the second gap at least doubles the first.
        const qint64 first = reads[1] - reads[0], second = reads[2] - reads[1];
        QVERIFY2(first >= Backend::RetryFirstMs * 9 / 10, qPrintable(QString::number(first)));
        QVERIFY2(second >= 2 * Backend::RetryFirstMs * 9 / 10, qPrintable(QString::number(second)));

        fake.setServing(true);
        QTRY_VERIFY_WITH_TIMEOUT(backend.connected(), 8000);
        QCOMPARE(backend.lastError(), QString());
        // A success stops the retries.
        const int settled = backend.refreshCount();
        settle(Backend::RetryFirstMs * 3);
        QCOMPARE(backend.refreshCount(), settled);
    }

    void aSuccessfulSnapshotClearsAnEarlierError() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        fake.failCalls("SetDefault", "dev.pipedeck.Error.InvalidArgument");
        backend->setDefault("sink", "nope");
        QTRY_VERIFY(backend->lastError().startsWith("dev.pipedeck.Error.InvalidArgument"));
        fake.emitChanged(); // the next GetAll is a successful call
        QTRY_COMPARE(backend->lastError(), QString());
    }

    // Codex 2 + Opus 12: calls go to the owner's unique name, so one issued
    // for a daemon cannot reach its replacement, and unsent ones are dropped.
    void callsCannotReachAReplacementDaemon() {
        auto first = std::make_unique<FakeDaemon>();
        first->populateDemo();
        auto backend = connectedBackend(*first);
        settle(150);
        QCOMPARE(backend->owner(), first->uniqueName());
        first->setApplyWrites(false);
        first->clearCalls();

        backend->setVolume(58, 40, true); // leaves at once
        backend->setVolume(58, 44, true); // queued behind the 50 ms gap: unsent
        // The name moves to a daemon that reuses node ids, and the client
        // issues a write before it has heard: no event loop runs in between.
        first->release();
        FakeDaemon second;
        second.populateDemo();
        QVERIFY(second.claim());
        QVERIFY(backend->connected());
        backend->setMute(58, true);

        QTRY_COMPARE(backend->owner(), second.uniqueName());
        QTRY_VERIFY(backend->connected());
        settle(250);
        QCOMPARE(second.calls("SetVolume").size(), 0);
        QCOMPARE(second.calls("SetMute").size(), 0);
        QVERIFY(!second.state().devices[0].mute);
        // The old daemon got what was addressed to it, and nothing after.
        QTRY_COMPARE(first->calls("SetMute").size(), 1);
        QCOMPARE(first->calls("SetVolume").size(), 1);
        QCOMPARE(first->calls("SetVolume").first().args.at(1).toDouble(), Backend::percentToLinear(40));
        QCOMPARE(backend->lastError(), QString());
        first.reset();
    }

    void absentDaemonStartsThroughActivation() {
        Backend backend(QDBusConnection::sessionBus(), settingsFile());
        QTRY_COMPARE(backend.state(), QString("absent"));
        QVERIFY(!backend.hasData());
        // No activation file on the private bus, so the bus says so, and the
        // window shows it.
        backend.startDaemon();
        QTRY_VERIFY(!backend.starting());
        QVERIFY2(backend.lastError().startsWith("org.freedesktop.DBus.Error.ServiceUnknown"),
                 qPrintable(backend.lastError()));
    }

    // ---- the window -------------------------------------------------------------

    // Opus 6: a choice the daemon rejects does not stay selected.
    void windowShowsWhatTheDaemonHasAfterARejection() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        Window w(m_dir.path(), backend.get());
        QVERIFY(w.window);
        w.window->resize(420, int(w.flickable()->property("contentHeight").toReal()) + 80);
        QTest::qWait(200);

        const QString speaker = "alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink";
        const QString fiio = "alsa_output.usb-Fiio_K3-00.analog-stereo";
        auto checked = [&](const QString &name) { return w.find(name)->property("checked").toBool(); };
        QVERIFY(checked("default:" + speaker));
        QVERIFY(!checked("default:" + fiio));
        QVERIFY(checked("notify:"));
        QVERIFY(!checked("autoMute:" + speaker));

        fake.failCalls("SetDefault", "dev.pipedeck.Error.InvalidArgument");
        fake.failCalls("SetNotificationSink", "dev.pipedeck.Error.InvalidArgument");
        fake.failCalls("SetAutoMute", "dev.pipedeck.Error.PipeWire");
        w.click(w.find("default:" + fiio));
        QTRY_COMPARE(fake.calls("SetDefault").size(), 1);
        w.click(w.find("notify:" + fiio));
        QTRY_COMPARE(fake.calls("SetNotificationSink").size(), 1);
        w.click(w.find("autoMute:" + speaker));
        QTRY_COMPARE(fake.calls("SetAutoMute").size(), 1);
        QTRY_VERIFY(backend->lastError().startsWith("dev.pipedeck.Error.PipeWire"));
        settle(150);
        QVERIFY(checked("default:" + speaker));
        QVERIFY(!checked("default:" + fiio));
        QVERIFY(checked("notify:"));
        QVERIFY(!checked("notify:" + fiio));
        QVERIFY(!checked("autoMute:" + speaker));

        // Accepted, the same clicks move the controls once the snapshot says so.
        fake.failCalls("SetDefault", {});
        fake.failCalls("SetNotificationSink", {});
        fake.failCalls("SetAutoMute", {});
        w.click(w.find("default:" + fiio));
        w.click(w.find("notify:" + fiio));
        w.click(w.find("autoMute:" + speaker));
        QTRY_VERIFY(checked("default:" + fiio));
        QVERIFY(!checked("default:" + speaker));
        QTRY_VERIFY(checked("notify:" + fiio));
        QVERIFY(!checked("notify:"));
        QTRY_VERIFY(checked("autoMute:" + speaker));
    }

    // Opus 5 in the window: a click on the handle of a slider drawn at its
    // end for a 150 % level sends nothing, nor does one on a level in range.
    void windowClickOnASliderHandleSendsNothing() {
        FakeDaemon fake;
        fake.populateDemo();
        fake.edit([](FakeState &s) { s.devices[1].volume = 3.375; });
        auto backend = connectedBackend(fake);
        Window w(m_dir.path(), backend.get());
        QVERIFY(w.window);
        w.window->resize(420, int(w.flickable()->property("contentHeight").toReal()) + 80);
        QTest::qWait(200);
        for (const uint id : {71u, 58u}) {
            QQuickItem *slider = w.find(QStringLiteral("volume:%1").arg(id));
            QVERIFY(slider);
            auto *handle = slider->property("handle").value<QQuickItem *>();
            QVERIFY(handle);
            const QPointF centre = handle->mapToItem(slider, QPointF(handle->width() / 2, handle->height() / 2));
            w.click(slider, centre);
        }
        settle(250);
        QCOMPARE(fake.calls("SetVolume").size(), 0);
        QVERIFY(qAbs(field(backend->outputs(), 71, "percent").toDouble() - 150) < 1e-6);
    }

    // Codex 6: Tab never leaves the focused control clipped by the viewport.
    void windowKeepsTheFocusedControlInView() {
        FakeDaemon fake;
        auto backend = connectedBackend(fake);
        Window w(m_dir.path(), backend.get());
        QVERIFY(w.window);
        w.window->show();
        w.window->requestActivate();
        QTest::qWait(200);
        QQuickItem *flick = w.flickable();
        QQuickItem *contentItem = flick->property("contentItem").value<QQuickItem *>();
        QSet<QQuickItem *> visited;
        int checkedItems = 0;
        for (int i = 0; i < 120; ++i) {
            QTest::keyClick(w.window, Qt::Key_Tab);
            QTest::qWait(20);
            QQuickItem *item = w.window->activeFocusItem();
            if (!item || visited.contains(item))
                continue;
            visited.insert(item);
            bool inside = false;
            for (QQuickItem *p = item; p; p = p->parentItem())
                inside = inside || p == contentItem;
            if (!inside || item->height() > flick->height())
                continue;
            const QPointF top = item->mapToItem(flick, QPointF(0, 0));
            QVERIFY2(top.y() >= -0.5 && top.y() + item->height() <= flick->height() + 0.5,
                     qPrintable(QString("%1 at %2..%3 in a %4 px viewport")
                                    .arg(item->metaObject()->className())
                                    .arg(top.y())
                                    .arg(top.y() + item->height())
                                    .arg(flick->height())));
            ++checkedItems;
        }
        QVERIFY2(checkedItems > 20, qPrintable(QString::number(checkedItems)));
    }

    // ---- small pieces ---------------------------------------------------------

    void rowModelUpdatesInPlace() {
        RowModel model({"nodeId", "v"});
        model.setRows({{{"nodeId", 1}, {"v", "a"}}, {{"nodeId", 2}, {"v", "b"}}});
        QSignalSpy inserted(&model, &QAbstractItemModel::rowsInserted);
        QSignalSpy removed(&model, &QAbstractItemModel::rowsRemoved);
        QSignalSpy changed(&model, &QAbstractItemModel::dataChanged);
        model.setRows({{{"nodeId", 1}, {"v", "a"}}, {{"nodeId", 2}, {"v", "c"}}});
        QCOMPARE(inserted.size(), 0);
        QCOMPARE(removed.size(), 0);
        QCOMPARE(changed.size(), 1);
        model.setRows({{{"nodeId", 2}, {"v", "c"}}, {{"nodeId", 3}, {"v", "d"}}});
        QCOMPARE(removed.size(), 1);
        QCOMPARE(inserted.size(), 1);
        QCOMPARE(model.get(0).value("nodeId").toInt(), 2);
        QCOMPARE(model.get(1).value("v").toString(), QString("d"));
    }

    void appIconsMatchDesktopEntries() {
        QTemporaryDir dir;
        write(dir.filePath("spotify.desktop"), "[Desktop Entry]\nName=Spotify\nExec=spotify %U\nIcon=spotify-client\n");
        write(dir.filePath("com.example.Player.desktop"),
              "[Desktop Entry]\nName=Example Player\nExec=env GDK_BACKEND=x11 /opt/ex/explayer\nIcon=/opt/ex/icon.png\n"
              "StartupWMClass=ExPlayer\n[Desktop Action new]\nIcon=wrong\n");
        AppIcons icons({dir.path()}, [](const QString &icon) { return icon != "missing-icon"; });
        QCOMPARE(icons.iconFor("spotify", ""), QString("spotify-client"));
        QCOMPARE(icons.iconFor("", "Spotify"), QString("spotify-client"));
        QCOMPARE(icons.iconFor("explayer", ""), QString("/opt/ex/icon.png"));
        QCOMPARE(icons.iconFor("", "explayer"), QString("/opt/ex/icon.png"));
        QCOMPARE(icons.iconFor("player", ""), QString("/opt/ex/icon.png")); // last id segment
        QCOMPARE(icons.iconFor("nothing", "Nobody"), QString());
        // An entry whose icon cannot be drawn falls back to the glyph.
        write(dir.filePath("ghost.desktop"), "[Desktop Entry]\nName=Ghost\nExec=ghost\nIcon=missing-icon\n");
        AppIcons fresh({dir.path()}, [](const QString &icon) { return icon != "missing-icon"; });
        QCOMPARE(fresh.iconFor("ghost", ""), QString());
    }

    void singleInstanceHandsOverToThePrimary() {
        QTemporaryDir dir;
        // A socket file left by a crashed instance must not block the next.
        write(dir.filePath("pipedeck-ui.sock"), "");
        SingleInstance primary(dir.path());
        QVERIFY(primary.becomePrimary());
        QSignalSpy shown(&primary, &SingleInstance::showRequested);

        SingleInstance second(dir.path());
        QVERIFY(!second.becomePrimary());
        // The second launch blocks while it waits for "ok", so it runs on a
        // thread of its own and the primary's event loop answers.
        bool answered = false;
        qputenv("XDG_ACTIVATION_TOKEN", "token-123");
        std::unique_ptr<QThread> thread(QThread::create([&] {
            SingleInstance launcher(dir.path());
            answered = launcher.signalPrimary(1000);
        }));
        thread->start();
        QTRY_VERIFY(thread->isFinished());
        qunsetenv("XDG_ACTIVATION_TOKEN");
        QVERIFY(answered);
        QCOMPARE(shown.size(), 1);
        QCOMPARE(shown.first().first().toString(), QString("token-123"));
    }

    // Opus 11: a primary that never answers (stuck in a cold start, or on
    // its way out) does not make the launch fail: once the lock is free the
    // launch runs as the primary itself.
    void singleInstanceTakesOverFromASilentPrimary() {
        QTemporaryDir dir;
        auto silent = std::make_unique<SingleInstance>(dir.path());
        QVERIFY(silent->acquire()); // holds the lock, never listens
        std::atomic<int> claim{-1};
        std::atomic<bool> listened{false};
        std::unique_ptr<QThread> thread(QThread::create([&] {
            SingleInstance launcher(dir.path());
            claim = int(launcher.claim(20, 200));
            if (claim == int(SingleInstance::Claim::Primary))
                listened = launcher.listen();
        }));
        thread->start();
        QTest::qWait(500);
        QVERIFY(!thread->isFinished());
        silent.reset();
        QTRY_VERIFY_WITH_TIMEOUT(thread->isFinished(), 5000);
        QCOMPARE(claim.load(), int(SingleInstance::Claim::Primary));
        QVERIFY(listened.load());
    }

    void singleInstanceGivesUpWithoutAPrimary() {
        QTemporaryDir dir;
        SingleInstance lonely(dir.path());
        QElapsedTimer clock;
        clock.start();
        QVERIFY(!lonely.signalPrimary(300));
        QVERIFY(clock.elapsed() < 1000);
    }

    // ---- theme and translator (from the Omarchy template) ----------------------

    void themeFallsBack() {
        QTemporaryDir dir;
        Theme theme(dir.path());
        QCOMPARE(theme.accent(), QString("#FFD60A"));
        QCOMPARE(theme.accentForeground(), QString("black"));
    }

    void themeReadsAccent() {
        QTemporaryDir dir;
        QDir(dir.path()).mkpath("theme");
        write(dir.path() + "/theme/colors.toml", "accent = \"#112233\"\n");
        Theme theme(dir.path());
        QCOMPARE(theme.accent(), QString("#112233"));
        QCOMPARE(theme.accentForeground(), QString("white"));
    }

    void themeFollowsAThemeSwitch() {
        QTemporaryDir dir;
        QDir(dir.path()).mkpath("theme");
        write(dir.path() + "/theme/colors.toml", "accent = \"#112233\"\n");
        Theme theme(dir.path());
        // omarchy theme set replaces the file rather than editing it.
        QFile::remove(dir.path() + "/theme/colors.toml");
        write(dir.path() + "/theme/colors.toml", "accent = \"#AABBCC\"\n");
        QTRY_COMPARE(theme.accent(), QString("#AABBCC"));
    }

    void themeFollowsHyprlandRounding() {
        QTemporaryDir dir;
        const QString response = dir.filePath("response.json"), command = dir.filePath("hyprctl");
        write(command, "#!/bin/sh\ncat '" + response.toUtf8() + "'\n");
        QVERIFY(QFile::setPermissions(command, QFile::ReadOwner | QFile::WriteOwner | QFile::ExeOwner));
        write(response, "{\"int\":0}");
        Theme theme(dir.path(), nullptr, command);
        QCOMPARE(theme.radius(), 0);
        write(response, "{\"int\":6}");
        QTRY_COMPARE(theme.radius(), 6);
    }

    void translatorFallsBackToEnglish() {
        QTemporaryDir dir;
        Translator translator(dir.path() + "/language", dir.path());
        QCOMPARE(translator.language(), QString("en"));
        QVERIFY(translator.translate("Main", "Save").isNull());
    }

    void translatorFollowsTheLanguageFile() {
        QTemporaryDir dir;
        write(dir.path() + "/language", "pt-BR\n");
        write(dir.path() + "/pt.json", R"({"Save": "Guardar", "Open": "Abrir"})");
        write(dir.path() + "/pt-BR.json", R"({"Save": "Salvar"})");
        Translator translator(dir.path() + "/language", dir.path());
        QCOMPARE(translator.language(), QString("pt-BR"));
        QCOMPARE(translator.translate("Main", "Save"), QString("Salvar"));
        QCOMPARE(translator.translate("Main", "Open"), QString("Abrir"));
        QVERIFY(translator.translate("Main", "Quit").isNull());

        // A live switch, as `omarchy language da` does it.
        write(dir.path() + "/da.json", R"({"Save": "Gem"})");
        QSignalSpy changed(&translator, &Translator::changed);
        QFile::remove(dir.path() + "/language");
        write(dir.path() + "/language", "da\n");
        QTRY_COMPARE(translator.language(), QString("da"));
        QCOMPARE(translator.translate("Main", "Save"), QString("Gem"));
    }

    void translatorPicksPluralForms() {
        QTemporaryDir dir;
        write(dir.path() + "/language", "ru\n");
        write(dir.path() + "/en.json", R"({"%n clips": {"one": "%n clip", "other": "%n clips"}})");
        write(dir.path() + "/ru.json",
              R"({"%n clips": {"one": "%n клип", "few": "%n клипа", "many": "%n клипов", "other": "%n клипа"}})");
        Translator translator(dir.path() + "/language", dir.path());
        QCOMPARE(translator.translate("Main", "%n clips", nullptr, 21), QString("%n клип"));
        QCOMPARE(translator.translate("Main", "%n clips", nullptr, 22), QString("%n клипа"));
        QCOMPARE(translator.translate("Main", "%n clips", nullptr, 25), QString("%n клипов"));
        QCOMPARE(Translator::pluralCategory("ar", 11), QString("many"));
        QCOMPARE(Translator::pluralCategory("ja", 1), QString("other"));
    }

    void shippedCatalogsTranslateEveryString() {
        // Every qsTr()/tr() literal in the sources, in every catalog we ship,
        // so a new string cannot slip into a release untranslated.
        QSet<QString> strings;
        // Not a raw string: moc's tokenizer loses the class after one with
        // quotes inside a custom delimiter.
        const QRegularExpression call("\\b(?:qsTr|tr)\\(\\s*\"((?:[^\"\\\\]|\\\\.)*)\"");
        QDirIterator it(PIPEDECK_UI_SRC, {"*.qml", "*.cpp"}, QDir::Files);
        while (it.hasNext()) {
            const QString text = QString::fromUtf8(readFile(it.next()));
            for (auto m = call.globalMatch(text); m.hasNext();)
                strings.insert(m.next().captured(1).replace("\\\"", "\"").replace("\\\\", "\\"));
        }
        QVERIFY(strings.size() > 40);

        const auto catalogs = QDir(QString(PIPEDECK_UI_SRC) + "/i18n").entryList({"*.json"});
        QVERIFY(catalogs.contains("da.json"));
        for (const auto &name : catalogs) {
            const QJsonObject catalog =
                QJsonDocument::fromJson(readFile(QString(PIPEDECK_UI_SRC) + "/i18n/" + name)).object();
            QVERIFY2(!catalog.isEmpty(), qPrintable(name));
            for (const auto &s : strings)
                QVERIFY2(catalog.value(s).isString() && !catalog.value(s).toString().isEmpty(),
                         qPrintable(name + " lacks: " + s));
            for (auto c = catalog.begin(); c != catalog.end(); ++c) {
                QVERIFY2(strings.contains(c.key()), qPrintable(name + " has a stale key: " + c.key()));
                // Placeholders survive translation.
                for (const auto &ph : {"%1", "%2"})
                    QCOMPARE(c.value().toString().contains(ph), c.key().contains(ph));
            }
        }
    }
};

QTEST_MAIN(PipedeckUiTests)
#include "pipedeck-ui_tests.moc"
