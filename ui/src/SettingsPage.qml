import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

// The gear's page (SPEC-UI §4.6): the daemon, the port-switch cap and where
// EQ presets live. Esc or the gear goes back.
ColumnLayout {
    spacing: 16

    Label {
        text: qsTr("Settings")
        font.pixelSize: 17
        font.weight: Font.DemiBold
    }

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: daemonColumn.implicitHeight + 24
        radius: Math.min(10, theme.radius)
        color: "#17171b"
        border.color: "#24242a"
        ColumnLayout {
            id: daemonColumn
            anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 14 }
            spacing: 6
            Label {
                text: qsTr("Daemon")
                font.weight: Font.DemiBold
            }
            Label {
                objectName: "daemonStatus"
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                opacity: 0.75
                text: backend.state === "connected"
                      ? (backend.version.length > 0 ? qsTr("Connected to pipedeckd %1").arg(backend.version) : qsTr("Connected to pipedeckd"))
                      : backend.state === "absent" ? (backend.starting ? qsTr("Starting pipedeckd…") : qsTr("PipeDeck daemon not running"))
                      : qsTr("Connecting…")
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
    }

    Rectangle {
        Layout.fillWidth: true
        visible: backend.features.portSwitchCap === true
        implicitHeight: capRow.implicitHeight + 24
        radius: Math.min(10, theme.radius)
        color: "#17171b"
        border.color: "#24242a"
        RowLayout {
            id: capRow
            anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 14 }
            spacing: 12
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Label {
                    Layout.fillWidth: true
                    text: qsTr("Port-switch level cap")
                    font.weight: Font.DemiBold
                    wrapMode: Text.WordWrap
                }
                Label {
                    Layout.fillWidth: true
                    text: qsTr("When PipeDeck switches a port, a restored level above this is lowered to it.")
                    wrapMode: Text.WordWrap
                    font.pixelSize: 12
                    opacity: 0.65
                }
            }
            SpinBox {
                id: cap
                objectName: "portSwitchCap"
                from: 0
                to: 150
                stepSize: 5
                editable: true
                value: backend.portSwitchCap
                Accessible.name: qsTr("Port-switch level cap")
                Material.roundedScale: theme.radius
                // A percentage, unlike the linear SetVolume: 0 means off.
                textFromValue: (value, locale) => (translator.language, value === 0 ? qsTr("Off") : qsTr("%1 %").arg(value))
                valueFromText: (text, locale) => {
                    if (text.trim() === qsTr("Off"))
                        return 0
                    const n = parseInt(text)
                    return isNaN(n) ? cap.value : n
                }
                validator: RegularExpressionValidator { regularExpression: /.*/ }
                onValueModified: backend.setPortSwitchCap(value)
            }
        }
    }

    Label {
        Layout.fillWidth: true
        text: qsTr("EQ presets live in ~/.config/pipedeck/eq/ and /usr/share/pipedeck/presets/; import an AutoEq file with: pipedeck eq import <file>")
        wrapMode: Text.WordWrap
        font.pixelSize: 12
        opacity: 0.6
    }
}
