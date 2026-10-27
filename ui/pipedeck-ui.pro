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

# The version goes in a generated header rather than DEFINES: make does not
# rebuild an object when only a define changes, so a bump in Cargo.toml left
# main.o with the old version. The header is rewritten only when the version
# differs, so an unchanged version rebuilds nothing.
VERSION_HEADER = $$OUT_PWD/pipedeck_version.h
VERSION_OLD = $$cat($$VERSION_HEADER, lines)
VERSION_OLD = $$section(VERSION_OLD, '"', 1, 1)
!equals(VERSION_OLD, $$PIPEDECK_VERSION) {
    VERSION_LINE = "$${LITERAL_HASH}define PIPEDECK_UI_VERSION \"$$PIPEDECK_VERSION\""
    write_file($$VERSION_HEADER, VERSION_LINE)|error("cannot write $$VERSION_HEADER")
}
INCLUDEPATH += $$OUT_PWD

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
