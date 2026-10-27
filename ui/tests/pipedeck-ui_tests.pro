QT += core gui qml quick quickcontrols2 dbus network svg testlib
CONFIG += c++20 testcase
TARGET = pipedeck-ui_tests
TEMPLATE = app

INCLUDEPATH += ../src

# The contract and the sources are read at test time: the XML for the
# coverage test, the QML and catalogs for the translation test.
DEFINES += PIPEDECK_XML=\\\"$$clean_path($$PWD/../../crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml)\\\"
DEFINES += PIPEDECK_UI_SRC=\\\"$$clean_path($$PWD/../src)\\\"

HEADERS += \
    fakedaemon.h \
    ../src/appicons.h \
    ../src/backend.h \
    ../src/daemon.h \
    ../src/rowmodel.h \
    ../src/singleinstance.h \
    ../src/theme.h \
    ../src/translator.h

SOURCES += \
    pipedeck-ui_tests.cpp \
    fakedaemon.cpp \
    ../src/appicons.cpp \
    ../src/backend.cpp \
    ../src/daemon.cpp \
    ../src/rowmodel.cpp \
    ../src/singleinstance.cpp \
    ../src/theme.cpp \
    ../src/translator.cpp

# The window tests load the real Main.qml.
RESOURCES += ../src/resources.qrc
