import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

Rectangle {
    id: overlay
    signal dismissed()
    color: "#d9000000"

    MouseArea { anchors.fill: parent; onClicked: overlay.dismissed() }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(parent.width - 32, 400)
        height: grid.implicitHeight + 40
        radius: theme.radius
        color: "#1c1c21"
        border.color: "#2c2c33"

        GridLayout {
            id: grid
            anchors { fill: parent; margins: 20 }
            columns: 2
            columnSpacing: 16
            rowSpacing: 8

            Label {
                Layout.columnSpan: 2
                text: qsTr("Keyboard shortcuts")
                font.pixelSize: 15
                font.weight: Font.DemiBold
            }

            // Flattened to key, description, key, ... so the grid sizes the
            // key column to its widest entry.
            Repeater {
                model: [
                    "?", qsTr("Show or hide these shortcuts"),
                    "Q", qsTr("Quit"),
                    "Esc", qsTr("Close an open picker, then the window"),
                    "Tab / Shift+Tab", qsTr("Move between controls"),
                    "← →", qsTr("Change the focused slider"),
                    "Space", qsTr("Press the focused button, radio or switch"),
                    "F5", qsTr("Ask the daemon to re-read the audio graph")
                ]
                delegate: Label {
                    required property string modelData
                    required property int index
                    readonly property bool isKey: index % 2 === 0
                    Layout.fillWidth: !isKey
                    Layout.alignment: Qt.AlignTop
                    text: modelData
                    wrapMode: isKey ? Text.NoWrap : Text.WordWrap
                    font.family: isKey ? "monospace" : font.family
                    color: isKey ? Material.accent : Material.foreground
                }
            }
        }
    }
}
