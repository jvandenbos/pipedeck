QT += core gui qml quick quickcontrols2 dbus network svg

CONFIG += c++20 release
TARGET = pipedeck-ui
TEMPLATE = app

# One version for the whole project: the workspace's, from ../Cargo.toml.
CARGO_TOML = $$cat($$PWD/../Cargo.toml, lines)
for(line, CARGO_TOML) {
    isEmpty(PIPEDECK_VERSION):contains(line, "^version *= *\".*\"") {
        PIPEDECK_VERSION = $$section(line, '"', 1, 1)
    }
}
isEmpty(PIPEDECK_VERSION): PIPEDECK_VERSION = 0.0.0
DEFINES += PIPEDECK_UI_VERSION=\\\"$$PIPEDECK_VERSION\\\"

HEADERS += \
    src/appicons.h \
    src/backend.h \
    src/daemon.h \
    src/rowmodel.h \
    src/singleinstance.h \
    src/theme.h \
    src/translator.h

SOURCES += \
    src/main.cpp \
    src/appicons.cpp \
    src/backend.cpp \
    src/daemon.cpp \
    src/rowmodel.cpp \
    src/singleinstance.cpp \
    src/theme.cpp \
    src/translator.cpp

RESOURCES += src/resources.qrc
