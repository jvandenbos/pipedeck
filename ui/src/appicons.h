#pragma once

#include <QHash>
#include <QQuickImageProvider>
#include <QStringList>
#include <functional>

// Finds a stream's icon the way a launcher would: through the desktop entry
// whose file name, Exec binary, StartupWMClass or Name matches the stream's
// `binary` or `app_name` (SPEC-UI §4.3). Entries are read once, lazily.
class AppIcons {
public:
    // Empty directories mean $XDG_DATA_HOME and $XDG_DATA_DIRS, each +
    // /applications. `available` says whether an Icon= value can be drawn;
    // by default, a theme icon that exists or an absolute path to a file.
    explicit AppIcons(const QStringList &applicationDirectories = {},
                      std::function<bool(const QString &)> available = {});

    // The desktop entry's Icon= value (a theme name or an absolute path), or
    // an empty string when nothing matches or the icon cannot be drawn, so
    // the window shows its generic glyph instead of a blank.
    QString iconFor(const QString &binary, const QString &appName) const;

private:
    void load() const;

    QStringList m_directories;
    std::function<bool(const QString &)> m_available;
    mutable bool m_loaded = false;
    mutable QHash<QString, QString> m_icons;
};

// Serves image://appicon/<theme name or url-encoded path> to QML; an unknown
// icon yields an empty image, which the QML replaces with a generic glyph.
class AppIconProvider : public QQuickImageProvider {
public:
    AppIconProvider();
    QPixmap requestPixmap(const QString &id, QSize *size, const QSize &requestedSize) override;
};
