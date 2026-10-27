import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ColumnLayout {
    id: entry
    required property var model

    Layout.fillWidth: true
    spacing: 0

    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Item {
            implicitWidth: 20
            implicitHeight: 20
            Image {
                id: appIcon
                anchors.fill: parent
                sourceSize: Qt.size(40, 40)
                source: entry.model.icon.length > 0 ? "image://appicon/" + entry.model.icon : ""
                visible: status === Image.Ready && implicitWidth > 0
            }
            // The generic glyph when no desktop entry matched, or its icon
            // is not in the theme.
            Image {
                anchors.fill: parent
                source: "qrc:/icons/app.svg"
                sourceSize: Qt.size(40, 40)
                visible: !appIcon.visible
                opacity: 0.6
            }
        }
        Label {
            Layout.fillWidth: true
            Layout.preferredHeight: 36
            verticalAlignment: Text.AlignVCenter
            text: entry.model.label
            elide: Text.ElideRight
            ToolTip.visible: labelHover.hovered && truncated
            ToolTip.text: entry.model.label
            HoverHandler { id: labelHover }
        }
        PercentLabel { percent: entry.model.percent }
        MuteButton {
            muted: entry.model.mute
            onClicked: backend.setMute(entry.model.nodeId, !entry.model.mute)
        }
    }

    LevelSlider {
        Layout.fillWidth: true
        nodeId: entry.model.nodeId
        percent: entry.model.percent
        Accessible.name: qsTr("Volume of %1").arg(entry.model.label)
    }

    PickerBox {
        Layout.fillWidth: true
        Layout.topMargin: 2
        visible: backend.features.streamTarget === true
        Accessible.name: qsTr("Destination of %1").arg(entry.model.label)
        current: entry.model.target
        // A requested sink that is gone stays selectable as itself, so the
        // picker shows the request instead of silently snapping elsewhere.
        choices: [{ value: "", text: qsTr("Follow default") }]
                 .concat(backend.sinkChoices)
                 .concat(entry.model.targetAbsent
                         ? [{ value: entry.model.target, text: qsTr("%1 (absent)").arg(entry.model.targetLabel) }] : [])
        onActivated: index => {
            const value = choices[index].value
            if (value === entry.model.target)
                return
            if (value === "")
                backend.followDefault(entry.model.nodeId)
            else
                backend.setStreamTarget(entry.model.nodeId, value)
        }
    }

    Label {
        Layout.fillWidth: true
        Layout.leftMargin: 4
        visible: entry.model.showObserved
        text: qsTr("Playing on %1").arg(entry.model.observedLabel)
        font.pixelSize: 12
        opacity: 0.65
        elide: Text.ElideRight
    }
}
