import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ColumnLayout {
    property string text
    property string help
    Layout.fillWidth: true
    spacing: 4

    Label {
        Layout.fillWidth: true
        text: parent.text
        font.pixelSize: 13
        font.weight: Font.DemiBold
        font.capitalization: Font.AllUppercase
        color: Material.accent
        elide: Text.ElideRight
    }
    Label {
        Layout.fillWidth: true
        visible: text.length > 0
        text: parent.help
        wrapMode: Text.WordWrap
        font.pixelSize: 12
        opacity: 0.65
    }
}
