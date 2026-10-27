import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

// A compact segmented control, "Speaker · Headphones" (SPEC-UI §4.1). Drawn
// by hand: Material's buttons have no segmented look, and a checked flat
// button reads the same as an unchecked one.
RowLayout {
    id: selector
    property var ports: []
    signal picked(int index)
    spacing: 0

    Repeater {
        model: selector.ports
        delegate: AbstractButton {
            id: segment
            required property var modelData
            required property int index
            Layout.fillWidth: true
            Layout.preferredWidth: 1
            implicitHeight: 28
            focusPolicy: Qt.StrongFocus
            checkable: false
            Accessible.role: Accessible.RadioButton
            Accessible.checked: modelData.active
            Accessible.name: modelData.label
            onClicked: if (!modelData.active) selector.picked(modelData.index)

            background: Rectangle {
                color: segment.modelData.active ? segment.Material.accent
                                                : segment.hovered ? "#24242a" : "transparent"
                border.color: segment.visualFocus ? segment.Material.accent : "#3a3a42"
                border.width: segment.visualFocus ? 2 : 1
                radius: Math.min(4, theme.radius)
            }
            contentItem: Label {
                text: segment.modelData.label
                font.pixelSize: 12
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
                color: segment.modelData.active ? theme.accentForeground : segment.Material.foreground
            }
        }
    }
}
