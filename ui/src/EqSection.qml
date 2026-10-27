import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

// One picker per output, in its own section: per-row pickers would not fit
// beside the sliders at the window's 420 px width (SPEC-UI §4.5).
ColumnLayout {
    Layout.fillWidth: true
    visible: backend.features.eq === true
    spacing: 8

    SectionTitle { text: qsTr("Equalizer") }

    Label {
        Layout.fillWidth: true
        visible: backend.eqPresets.length === 0
        text: qsTr("No presets installed. Presets live in ~/.config/pipedeck/eq/ and /usr/share/pipedeck/presets/; import an AutoEq file with: pipedeck eq import <file>")
        wrapMode: Text.WordWrap
        font.pixelSize: 12
        opacity: 0.65
    }

    Repeater {
        model: backend.eqPresets.length > 0 ? backend.outputs : null
        delegate: ColumnLayout {
            id: eqRow
            required property var model
            Layout.fillWidth: true
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                Label {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    text: eqRow.model.nick
                    elide: Text.ElideRight
                }
                PickerBox {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    enabled: eqRow.model.eqAvailable
                    Accessible.name: qsTr("Equalizer for %1").arg(eqRow.model.nick)
                    current: eqRow.model.eqPreset
                    choices: {
                        const list = [{ value: "", text: qsTr("Off") }]
                        for (const p of backend.eqPresets)
                            list.push({ value: p.id, text: p.name })
                        return list
                    }
                    onActivated: index => {
                        const value = choices[index].value
                        if (value !== eqRow.model.eqPreset)
                            backend.setEq(eqRow.model.nodeId, value)
                    }
                }
            }
            Label {
                Layout.fillWidth: true
                visible: !eqRow.model.eqAvailable
                text: eqRow.model.isVirtual
                      ? qsTr("A virtual output: EQ goes on the physical output it plays to.")
                      : qsTr("EQ is not available for this output.")
                wrapMode: Text.WordWrap
                font.pixelSize: 12
                opacity: 0.65
            }
        }
    }
}
