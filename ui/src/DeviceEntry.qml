import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ColumnLayout {
    id: entry
    required property var model
    property string kind
    property ButtonGroup group

    Layout.fillWidth: true
    spacing: 0

    RowLayout {
        Layout.fillWidth: true
        spacing: 4

        RadioButton {
            id: radio
            Layout.fillWidth: true
            Layout.preferredHeight: 36
            text: entry.model.nick
            checked: entry.model.isDefault
            enabled: backend.features.defaults === true
            ButtonGroup.group: entry.group
            Accessible.name: entry.kind === "sink" ? qsTr("Default output: %1").arg(entry.model.nick)
                                                   : qsTr("Default input: %1").arg(entry.model.nick)
            ToolTip.visible: hovered && entry.model.description.length > 0
            ToolTip.text: entry.model.description
            ToolTip.delay: 600
            // A click on the current default is a no-op, not a re-send.
            onClicked: if (!entry.model.isDefault) backend.setDefault(entry.kind, entry.model.name)
        }
        Rectangle {
            visible: entry.model.eqName.length > 0
            implicitWidth: eqLabel.implicitWidth + 10
            implicitHeight: eqLabel.implicitHeight + 4
            radius: Math.min(4, theme.radius)
            color: "transparent"
            border.color: Material.accent
            Label {
                id: eqLabel
                anchors.centerIn: parent
                text: qsTr("EQ")
                font.pixelSize: 11
                color: Material.accent
            }
            ToolTip.visible: eqHover.hovered
            ToolTip.text: qsTr("Equalizer: %1").arg(entry.model.eqName)
            HoverHandler { id: eqHover }
        }
        PercentLabel { percent: entry.model.percent }
        MuteButton {
            muted: entry.model.mute
            input: entry.kind === "source"
            onClicked: backend.setMute(entry.model.nodeId, !entry.model.mute)
        }
    }

    LevelSlider {
        Layout.fillWidth: true
        nodeId: entry.model.nodeId
        percent: entry.model.percent
        Accessible.name: qsTr("Volume of %1").arg(entry.model.nick)
    }

    // Ports nest under the device that owns them: a selector, not peer rows.
    PortSelector {
        Layout.fillWidth: true
        Layout.leftMargin: 8
        Layout.rightMargin: 8
        Layout.bottomMargin: 4
        visible: entry.model.showPorts
        ports: entry.model.ports
        onPicked: index => backend.setPort(entry.model.nodeId, index)
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 8
        visible: entry.model.hasAutoMute
        Label {
            Layout.fillWidth: true
            text: qsTr("Auto-mute speakers when headphones are plugged in")
            wrapMode: Text.WordWrap
            font.pixelSize: 12
        }
        Switch {
            checked: entry.model.autoMute
            Accessible.name: qsTr("Auto-mute speakers when headphones are plugged in")
            onToggled: backend.setAutoMute(entry.model.nodeId, checked)
        }
    }
}
