import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

// The pill that names a row's device and opens its list (SPEC-UI §4.3), as
// SoundSource's device menu: one line per entry with a type glyph, a check
// and the accent on the current one. `entries` is [{label, glyph, current,
// ...}]; picking one emits chosen(entry) — the button itself changes
// nothing, the next snapshot does.
AbstractButton {
    id: button
    property var entries: []
    property string glyph
    property string tip
    signal chosen(var entry)

    readonly property alias popup: menu
    readonly property int currentIndex: {
        for (let i = 0; i < entries.length; ++i)
            if (entries[i].current)
                return i
        return -1
    }

    implicitHeight: 28
    focusPolicy: Qt.StrongFocus
    hoverEnabled: true
    font.pixelSize: 13
    Accessible.role: Accessible.ComboBox
    Accessible.name: tip
    ToolTip.visible: hovered && !menu.visible && tip.length > 0
    ToolTip.text: tip
    ToolTip.delay: 600
    onClicked: menu.opened ? menu.close() : menu.open()
    Keys.onDownPressed: menu.open()

    background: Rectangle {
        radius: Math.min(button.height / 2, theme.radius)
        color: button.down || menu.visible ? "#34343c" : button.hovered ? "#2c2c33" : "#26262c"
        border.width: button.visualFocus ? 2 : 0
        border.color: button.Material.accent
    }
    contentItem: RowLayout {
        spacing: 6
        Glyph {
            Layout.leftMargin: 4
            name: button.glyph
            size: 16
            visible: name.length > 0
        }
        Label {
            Layout.fillWidth: true
            text: button.text
            font: button.font
            elide: Text.ElideRight
            verticalAlignment: Text.AlignVCenter
        }
        Glyph {
            name: "unfold"
            size: 14
            opacity: 0.6
        }
    }

    Popup {
        id: menu
        y: button.height + 4
        x: Math.min(0, button.width - width)
        width: Math.max(button.width, 240)
        padding: 6
        // Kept inside the window: near the bottom it opens upwards.
        margins: 6
        focus: true
        // Esc goes through the window's chain (SPEC-UI §4.1), which closes
        // this before anything else.
        closePolicy: Popup.CloseOnPressOutside | Popup.CloseOnPressOutsideParent
        Material.roundedScale: theme.radius
        background: Rectangle {
            radius: Math.min(10, theme.radius)
            color: "#1f1f24"
            border.color: "#2e2e36"
        }
        onAboutToShow: list.currentIndex = Math.max(0, button.currentIndex)
        onOpened: {
            list.forceActiveFocus()
            button.ApplicationWindow.window.activePopup = menu
        }
        onAboutToHide: {
            const w = button.ApplicationWindow.window
            if (w && w.activePopup === menu)
                w.activePopup = null
        }

        contentItem: ListView {
            id: list
            implicitHeight: Math.min(contentHeight, 340)
            clip: true
            model: button.entries
            keyNavigationEnabled: true
            boundsBehavior: Flickable.StopAtBounds
            highlightMoveDuration: 0
            function pick(index) {
                const entry = button.entries[index]
                menu.close()
                if (entry && !entry.current)
                    button.chosen(entry)
            }
            Keys.onReturnPressed: pick(currentIndex)
            Keys.onEnterPressed: pick(currentIndex)
            Keys.onSpacePressed: pick(currentIndex)

            delegate: ItemDelegate {
                id: row
                required property var modelData
                required property int index
                objectName: "entry:" + modelData.label
                width: ListView.view.width
                height: 32
                padding: 0
                focusPolicy: Qt.NoFocus
                highlighted: ListView.isCurrentItem
                onClicked: list.pick(index)
                background: Rectangle {
                    radius: Math.min(6, theme.radius)
                    color: row.modelData.current ? Qt.rgba(row.Material.accent.r, row.Material.accent.g, row.Material.accent.b, 0.85)
                         : row.highlighted || row.hovered ? "#2c2c33" : "transparent"
                }
                contentItem: RowLayout {
                    spacing: 8
                    Glyph {
                        Layout.leftMargin: 6
                        name: "check"
                        size: 14
                        opacity: row.modelData.current ? 1 : 0
                        color: theme.accentForeground
                    }
                    Glyph {
                        name: row.modelData.glyph || ""
                        size: 16
                        color: row.modelData.current ? theme.accentForeground : row.Material.foreground
                    }
                    Label {
                        Layout.fillWidth: true
                        Layout.rightMargin: 8
                        text: row.modelData.label
                        elide: Text.ElideRight
                        font.pixelSize: 13
                        color: row.modelData.current ? theme.accentForeground : row.Material.foreground
                    }
                }
            }
        }
    }
}
