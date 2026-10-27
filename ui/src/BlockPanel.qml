import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import "Columns.js" as Cols

// A SoundSource block: a rounded panel a shade lighter than the window, a
// header with a collapse chevron, the title and dim column captions, then
// the rows (SPEC-UI §4.2). `captions` is [{text, width}] from the right
// edge's columns, left to right.
Rectangle {
    id: panel
    property string title
    property string titleTip
    property var captions: []
    property bool expanded: true
    property string key
    default property alias rows: body.data

    implicitHeight: column.implicitHeight
    radius: Math.min(10, theme.radius)
    color: "#17171b"
    border.color: "#24242a"
    clip: true

    ColumnLayout {
        id: column
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 40
            Layout.leftMargin: Cols.pad - 4
            Layout.rightMargin: Cols.pad
            spacing: Cols.spacing

            ToolButton {
                objectName: "collapse:" + panel.key
                implicitWidth: 28
                implicitHeight: 28
                padding: 4
                icon.source: "qrc:/icons/expand.svg"
                icon.width: 18
                icon.height: 18
                rotation: panel.expanded ? 0 : -90
                Accessible.name: panel.expanded ? qsTr("Collapse %1").arg(panel.title) : qsTr("Expand %1").arg(panel.title)
                background: Rectangle {
                    radius: Math.min(height / 2, theme.radius)
                    color: parent.hovered ? "#2c2c33" : "#222228"
                    border.width: parent.visualFocus ? 2 : 0
                    border.color: panel.Material.accent
                }
                onClicked: panel.expanded = !panel.expanded
            }
            Label {
                Layout.fillWidth: true
                Layout.preferredWidth: 0
                Layout.leftMargin: 4
                text: panel.title
                font.pixelSize: 14
                font.weight: Font.DemiBold
                elide: Text.ElideRight
                HoverHandler { id: titleHover }
                ToolTip.visible: titleHover.hovered && panel.titleTip.length > 0
                ToolTip.text: panel.titleTip
                ToolTip.delay: 400
            }
            Repeater {
                model: panel.captions
                delegate: Label {
                    required property var modelData
                    Layout.preferredWidth: modelData.width
                    Layout.minimumWidth: modelData.width
                    Layout.maximumWidth: modelData.width
                    horizontalAlignment: Text.AlignHCenter
                    text: modelData.text
                    font.pixelSize: 11
                    opacity: 0.5
                    elide: Text.ElideRight
                }
            }
        }

        ColumnLayout {
            id: body
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.bottomMargin: 6
            visible: panel.expanded
            spacing: 0
        }
    }
}
