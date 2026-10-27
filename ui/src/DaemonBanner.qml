import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ColumnLayout {
    visible: backend.state !== "connected"
    spacing: 10

    BusyIndicator {
        Layout.alignment: Qt.AlignHCenter
        visible: backend.state === "connecting" || backend.starting
        running: visible
    }
    Label {
        Layout.fillWidth: true
        visible: backend.state === "absent"
        text: qsTr("PipeDeck daemon not running")
        font.pixelSize: 18
        font.weight: Font.DemiBold
        wrapMode: Text.WordWrap
    }
    Label {
        Layout.fillWidth: true
        visible: backend.state === "absent"
        text: qsTr("pipedeckd does the routing, EQ and safety work, with or without this window. Start it now, or for good with: systemctl --user enable --now pipedeckd")
        wrapMode: Text.WordWrap
        opacity: 0.75
    }
    Button {
        visible: backend.state === "absent"
        enabled: !backend.starting
        text: qsTr("Start")
        highlighted: true
        Material.roundedScale: theme.radius
        onClicked: backend.startDaemon()
    }
}
