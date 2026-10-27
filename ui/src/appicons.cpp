#include "appicons.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QIcon>
#include <QStandardPaths>
#include <QUrl>

AppIcons::AppIcons(const QStringList &applicationDirectories, std::function<bool(const QString &)> available)
    : m_directories(applicationDirectories), m_available(std::move(available)) {
    if (!m_available)
        m_available = [](const QString &icon) {
            return icon.startsWith('/') ? QFileInfo::exists(icon) : QIcon::hasThemeIcon(icon);
        };
    if (m_directories.isEmpty())
        for (const auto &dir : QStandardPaths::standardLocations(QStandardPaths::ApplicationsLocation))
            m_directories << dir;
}

void AppIcons::load() const {
    m_loaded = true;
    // Earlier directories win (the user's own entries shadow the system's),
    // so only insert keys not seen yet.
    for (const auto &directory : m_directories) {
        const auto entries = QDir(directory).entryInfoList({"*.desktop"}, QDir::Files);
        for (const auto &info : entries) {
            QFile file(info.filePath());
            if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
                continue;
            QString icon, exec, wmClass, name;
            bool inEntry = false;
            while (!file.atEnd()) {
                const QString line = QString::fromUtf8(file.readLine()).trimmed();
                if (line.startsWith('[')) {
                    inEntry = line == "[Desktop Entry]";
                    continue;
                }
                if (!inEntry)
                    continue;
                // Name[da]= and friends are skipped: only the untranslated
                // Name can equal what a client reports as application.name.
                const QString value = line.section('=', 1).trimmed();
                if (line.startsWith("Icon="))
                    icon = value;
                else if (line.startsWith("Exec="))
                    exec = value;
                else if (line.startsWith("StartupWMClass="))
                    wmClass = value;
                else if (line.startsWith("Name="))
                    name = value;
            }
            if (icon.isEmpty())
                continue;
            const QString stem = info.completeBaseName();
            QStringList keys{stem, stem.section('.', -1), wmClass, name};
            // `env FOO=1 app --flag` and plain `/usr/bin/app %U` both name the
            // binary in the first token that is not env or an assignment.
            for (const auto &token : exec.split(' ', Qt::SkipEmptyParts)) {
                if (token == "env" || token.contains('='))
                    continue;
                keys << QFileInfo(token).fileName();
                break;
            }
            for (const auto &key : keys)
                if (!key.isEmpty() && !m_icons.contains(key.toLower()))
                    m_icons.insert(key.toLower(), icon);
        }
    }
}

QString AppIcons::iconFor(const QString &binary, const QString &appName) const {
    if (!m_loaded)
        load();
    for (const auto &key : {binary, appName}) {
        if (key.isEmpty())
            continue;
        const auto it = m_icons.constFind(key.toLower());
        if (it != m_icons.constEnd())
            return m_available(*it) ? *it : QString();
    }
    return {};
}

AppIconProvider::AppIconProvider() : QQuickImageProvider(QQuickImageProvider::Pixmap) {}

QPixmap AppIconProvider::requestPixmap(const QString &id, QSize *size, const QSize &requestedSize) {
    const QString name = QUrl::fromPercentEncoding(id.toUtf8());
    const QSize want = requestedSize.isValid() ? requestedSize : QSize(32, 32);
    QIcon icon = name.startsWith('/') ? QIcon(name) : QIcon::fromTheme(name);
    QPixmap pixmap = icon.isNull() ? QPixmap() : icon.pixmap(want);
    if (size)
        *size = pixmap.size();
    return pixmap;
}
