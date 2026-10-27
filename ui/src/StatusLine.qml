import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

// Connection state, daemon version and the last error, in the window rather
// than in dialogs (SPEC-UI §5). The error clears on the next successful call.
Rectangle {
    implicitHeight: column.implicitHeight + 16
    color: "#16161a"

    ColumnLayout {
        id: column
        anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 16 }
        spacing: 2

        Label {
            Layout.fillWidth: true
            elide: Text.ElideRight
            font.pixelSize: 12
            opacity: 0.7
            text: backend.state === "connected"
                  ? (backend.version.length > 0 ? qsTr("Connected to pipedeckd %1").arg(backend.version) : qsTr("Connected to pipedeckd"))
                  : backend.state === "absent" ? (backend.starting ? qsTr("Starting pipedeckd…") : qsTr("Daemon not running"))
                  : qsTr("Connecting…")
        }
        Label {
            Layout.fillWidth: true
            visible: backend.lastError.length > 0
            text: backend.lastError
            color: "#ff8a80"
            font.pixelSize: 12
            wrapMode: Text.WrapAnywhere
            maximumLineCount: 3
            elide: Text.ElideRight
        }
    }
}
