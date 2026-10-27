import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import "Columns.js" as Cols

// The Output row's disclosure (SPEC-UI §4.4): the EQ preset for the default
// sink and, where the card has one, the auto-mute switch.
Rectangle {
    id: fx
    required property var model // a device row
    implicitHeight: column.implicitHeight + 16
    color: Qt.rgba(Material.accent.r, Material.accent.g, Material.accent.b, 0.08)

    ColumnLayout {
        id: column
        anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter }
        anchors.leftMargin: Cols.pad + Cols.lead + Cols.spacing + 4
        anchors.rightMargin: Cols.pad
        spacing: 6

        RowLayout {
            Layout.fillWidth: true
            visible: backend.features.eq === true
            spacing: 8
            Label {
                Layout.fillWidth: true
                text: qsTr("Equalizer")
                font.pixelSize: 13
            }
            PickerBox {
                objectName: "eq:" + fx.model.name
                Layout.preferredWidth: 220
                visible: backend.eqPresets.length > 0 && fx.model.eqAvailable
                Accessible.name: qsTr("Equalizer for %1").arg(fx.model.nick)
                current: fx.model.eqPreset
                choices: {
                    const list = [{ value: "", text: qsTr("Off") }]
                    for (const p of backend.eqPresets)
                        list.push({ value: p.id, text: p.name })
                    return list
                }
                onActivated: index => {
                    const value = choices[index].value
                    if (value !== fx.model.eqPreset)
                        backend.setEq(fx.model.nodeId, value)
                }
            }
        }
        Label {
            Layout.fillWidth: true
            visible: backend.features.eq === true && (backend.eqPresets.length === 0 || !fx.model.eqAvailable)
            text: fx.model.isVirtual
                  ? qsTr("A virtual output: EQ goes on the physical output it plays to.")
                  : !fx.model.eqAvailable ? qsTr("EQ is not available for this output.")
                  : qsTr("No presets installed. Presets live in ~/.config/pipedeck/eq/ and /usr/share/pipedeck/presets/; import an AutoEq file with: pipedeck eq import <file>")
            wrapMode: Text.WordWrap
            font.pixelSize: 12
            opacity: 0.6
        }
        RowLayout {
            Layout.fillWidth: true
            visible: fx.model.hasAutoMute
            spacing: 8
            Label {
                Layout.fillWidth: true
                text: qsTr("Auto-mute speakers when headphones are plugged in")
                wrapMode: Text.WordWrap
                font.pixelSize: 13
            }
            Switch {
                objectName: "autoMute:" + fx.model.name
                // Shows the model only, so a rejected call leaves the switch
                // where the daemon has it.
                checkable: false
                checked: fx.model.autoMute
                implicitHeight: 32
                Accessible.name: qsTr("Auto-mute speakers when headphones are plugged in")
                onClicked: backend.setAutoMute(fx.model.nodeId, !fx.model.autoMute)
            }
        }
        Label {
            Layout.fillWidth: true
            visible: backend.features.eq !== true && !fx.model.hasAutoMute
            text: qsTr("This daemon has no effects for this output.")
            font.pixelSize: 12
            opacity: 0.6
        }
    }
}
