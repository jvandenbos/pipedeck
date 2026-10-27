import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import "Columns.js" as Cols

// One SoundSource-style row: glyph · name · mute · slider · % · Boost ·
// device popup · FX (SPEC-UI §4.2). The cells after the title have fixed
// widths, so the rows line up under the block's captions.
Rectangle {
    id: row
    property string glyph          // a bundled icon, or
    property string appIcon        // an app icon name for image://appicon
    property string title
    property string detail         // dim text after the title (a stream's media name)
    property string titleTip
    property bool hasLevel: true   // false: no mute, slider, % or Boost; the title takes their room
    property bool input: false
    property int nodeId
    property real percent
    property bool muted
    property bool boost
    property int maxPercent: 100
    property string accessibleName: title

    property bool hasDevice: true
    property alias deviceButton: device
    property bool warning: false
    property string warningTip

    property bool hasFxColumn: true // Applications rows give theirs to the device button
    property bool hasFx: false
    property bool fxOpen: false
    property bool selected: false
    property string key            // objectName suffix: output, input, effects or a stream id

    signal fxToggled()

    implicitHeight: Cols.rowHeight
    color: selected ? Qt.rgba(Material.accent.r, Material.accent.g, Material.accent.b, 0.22) : "transparent"

    RowLayout {
        anchors { fill: parent; leftMargin: Cols.pad; rightMargin: Cols.pad }
        spacing: Cols.spacing

        Item {
            Layout.preferredWidth: Cols.lead
            Layout.minimumWidth: Cols.lead
            Layout.maximumWidth: Cols.lead
            Layout.preferredHeight: Cols.lead
            Glyph {
                anchors.centerIn: parent
                name: row.glyph
                size: 20
                visible: row.glyph.length > 0
            }
            Image {
                id: appImage
                anchors.fill: parent
                sourceSize: Qt.size(40, 40)
                source: row.appIcon.length > 0 ? "image://appicon/" + row.appIcon : ""
                visible: row.glyph.length === 0 && status === Image.Ready && implicitWidth > 0
            }
            // The generic glyph when no desktop entry matched, or its icon
            // is not in the theme.
            Glyph {
                anchors.centerIn: parent
                name: "app"
                size: 20
                opacity: 0.6
                visible: row.glyph.length === 0 && !appImage.visible
            }
        }

        // Takes what the fixed columns leave; the app name before the media.
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredWidth: 0
            Layout.minimumWidth: 0
            Layout.leftMargin: 4
            spacing: 6
            // Not fillWidth: the name keeps its natural width and gives way
            // only when the row is too narrow; the media name gets the rest.
            Label {
                id: titleLabel
                Layout.minimumWidth: Math.min(implicitWidth, 48)
                text: row.title
                font.pixelSize: 14
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                Layout.preferredWidth: 0
                visible: row.detail.length > 0
                text: "— " + row.detail
                font.pixelSize: 13
                opacity: 0.5
                elide: Text.ElideRight
            }
            // Keeps the title region elastic when the labels are short, so
            // the fixed columns after it never spread apart.
            Item { Layout.fillWidth: true; visible: row.detail.length === 0 }
            HoverHandler { id: titleHover }
            ToolTip.visible: titleHover.hovered && row.titleTip.length > 0
            ToolTip.text: row.titleTip
            ToolTip.delay: 600
        }

        Item {
            visible: row.hasLevel
            Layout.preferredWidth: Cols.mute
            Layout.minimumWidth: Cols.mute
            Layout.maximumWidth: Cols.mute
            Layout.preferredHeight: Cols.mute
            MuteButton {
                anchors.centerIn: parent
                visible: row.hasLevel
                objectName: "mute:" + row.nodeId
                muted: row.muted
                input: row.input
                onClicked: backend.setMute(row.nodeId, !row.muted)
            }
        }
        Item {
            visible: row.hasLevel
            Layout.preferredWidth: Cols.slider
            Layout.minimumWidth: Cols.slider
            Layout.maximumWidth: Cols.slider
            Layout.preferredHeight: 28
            LevelSlider {
                anchors.fill: parent
                visible: row.hasLevel
                nodeId: row.nodeId
                percent: row.percent
                maxPercent: row.maxPercent
                Accessible.name: qsTr("Volume of %1").arg(row.accessibleName)
            }
        }
        PercentLabel {
            Layout.preferredWidth: Cols.percent
            Layout.minimumWidth: Cols.percent
            Layout.maximumWidth: Cols.percent
            percent: row.percent
            visible: row.hasLevel
        }
        Item {
            visible: row.hasLevel
            Layout.preferredWidth: Cols.boost
            Layout.minimumWidth: Cols.boost
            Layout.maximumWidth: Cols.boost
            Layout.preferredHeight: Cols.boost
            ToolButton {
                anchors.centerIn: parent
                visible: row.hasLevel
                objectName: "boost:" + row.nodeId
                implicitWidth: 28
                implicitHeight: 28
                padding: 4
                icon.source: "qrc:/icons/boost.svg"
                icon.width: 16
                icon.height: 16
                icon.color: row.boost ? theme.accentForeground : Material.foreground
                Accessible.name: qsTr("Boost %1 up to 150 %").arg(row.accessibleName)
                Accessible.checkable: true
                Accessible.checked: row.boost
                ToolTip.visible: hovered
                ToolTip.text: row.boost ? qsTr("Boost on: this slider goes to 150 %") : qsTr("Boost: let this slider go to 150 %")
                ToolTip.delay: 600
                background: Rectangle {
                    radius: Math.min(height / 2, theme.radius)
                    color: row.boost ? row.Material.accent : parent.hovered ? "#2c2c33" : "#222228"
                    border.width: parent.visualFocus ? 2 : 0
                    border.color: row.Material.accent
                }
                onClicked: backend.setBoost(row.nodeId, !row.boost)
            }
        }
        RowLayout {
            Layout.preferredWidth: row.hasFxColumn ? Cols.device : Cols.redirect
            Layout.minimumWidth: Layout.preferredWidth
            Layout.maximumWidth: Layout.preferredWidth
            spacing: 4
            Glyph {
                visible: row.warning
                name: "warning"
                size: 16
                color: "#ffcc00"
                HoverHandler { id: warnHover }
                ToolTip.visible: warnHover.hovered
                ToolTip.text: row.warningTip
            }
            DeviceButton {
                id: device
                Layout.fillWidth: true
                visible: row.hasDevice
                objectName: "device:" + row.key
            }
            Item { Layout.fillWidth: true; visible: !row.hasDevice }
        }
        Item {
            visible: row.hasFxColumn
            Layout.preferredWidth: Cols.fx
            Layout.minimumWidth: Cols.fx
            Layout.maximumWidth: Cols.fx
            Layout.preferredHeight: Cols.fx
            ToolButton {
                anchors.centerIn: parent
                visible: row.hasFx
                objectName: "fx:" + row.key
                implicitWidth: 26
                implicitHeight: 26
                padding: 4
                icon.source: "qrc:/icons/chevron-right.svg"
                icon.width: 16
                icon.height: 16
                rotation: row.fxOpen ? 90 : 0
                Accessible.name: qsTr("Effects for %1").arg(row.accessibleName)
                ToolTip.visible: hovered
                ToolTip.text: qsTr("Equalizer and auto-mute")
                ToolTip.delay: 600
                background: Rectangle {
                    radius: Math.min(height / 2, theme.radius)
                    color: parent.hovered ? "#2c2c33" : "#222228"
                    border.width: parent.visualFocus ? 2 : 0
                    border.color: row.Material.accent
                }
                onClicked: row.fxToggled()
            }
        }
    }
}
