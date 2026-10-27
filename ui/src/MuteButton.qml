import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material

ToolButton {
    property bool muted
    property bool input: false
    icon.source: input ? (muted ? "qrc:/icons/mic-muted.svg" : "qrc:/icons/mic.svg")
                       : (muted ? "qrc:/icons/volume-muted.svg" : "qrc:/icons/volume.svg")
    icon.color: muted ? Material.accent : Material.foreground
    implicitWidth: 28
    implicitHeight: 28
    padding: 4
    icon.width: 18
    icon.height: 18
    Accessible.name: muted ? qsTr("Unmute") : qsTr("Mute")
    ToolTip.visible: hovered
    ToolTip.text: muted ? qsTr("Unmute") : qsTr("Mute")
    ToolTip.delay: 600
}
