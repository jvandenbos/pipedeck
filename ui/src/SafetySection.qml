import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ColumnLayout {
    Layout.fillWidth: true
    spacing: 8

    SectionTitle { text: qsTr("Safety") }

    RowLayout {
        Layout.fillWidth: true
        visible: backend.features.portSwitchCap === true
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Label {
                Layout.fillWidth: true
                text: qsTr("Port-switch level cap")
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

    RowLayout {
        Layout.fillWidth: true
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Label {
                Layout.fillWidth: true
                text: qsTr("Allow up to 150 %")
                wrapMode: Text.WordWrap
            }
            Label {
                Layout.fillWidth: true
                text: qsTr("Widens the volume sliders past 100 %. Only this window's sliders change.")
                wrapMode: Text.WordWrap
                font.pixelSize: 12
                opacity: 0.65
            }
        }
        Switch {
            checked: backend.allowOverdrive
            Accessible.name: qsTr("Allow up to 150 %")
            onToggled: backend.allowOverdrive = checked
        }
    }
}
