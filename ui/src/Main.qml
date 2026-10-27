import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ApplicationWindow {
    id: win
    width: 420
    height: 720
    minimumWidth: 360
    minimumHeight: 420
    visible: true
    title: "PipeDeck"
    color: "#0e0e10"

    Material.theme: Material.Dark
    Material.accent: theme.accent
    Material.roundedScale: theme.radius

    property bool helpVisible: false
    // The picker whose popup is open, so Esc can close it before the window.
    property Item activePicker: null
    readonly property bool textFocus: activeFocusItem !== null
                                      && (activeFocusItem instanceof TextInput || activeFocusItem instanceof TextEdit)

    function closeTopmost() {
        if (activePicker) {
            activePicker.popup.close()
            activePicker = null
        } else if (helpVisible) {
            helpVisible = false
        } else {
            win.close()
        }
    }

    // "?" is Shift+/ on most layouts; listing both keeps it working whether or
    // not the platform's keymapper drops the Shift before matching.
    Shortcut { sequences: ["?", "Shift+?"]; context: Qt.ApplicationShortcut; onActivated: win.helpVisible = !win.helpVisible }
    Shortcut { sequence: "Q"; context: Qt.ApplicationShortcut; enabled: !win.textFocus; onActivated: Qt.quit() }
    Shortcut { sequence: "Esc"; context: Qt.ApplicationShortcut; onActivated: win.closeTopmost() }
    Shortcut { sequence: "F5"; context: Qt.ApplicationShortcut; onActivated: backend.refresh() }

    // Keyboard navigation scrolls the focused control into view.
    onActiveFocusItemChanged: {
        const item = activeFocusItem
        if (!item || !content.contains(content.mapFromItem(item, 0, 0)))
            return
        const top = item.mapToItem(content, 0, 0).y
        const bottom = top + item.height
        if (top < flick.contentY + 8)
            flick.contentY = Math.max(0, top - 8)
        else if (bottom > flick.contentY + flick.height - 8)
            flick.contentY = Math.min(flick.contentHeight - flick.height, bottom - flick.height + 8)
    }

    // Right-to-left languages mirror the window; the Translator sets the
    // application's layout direction.
    Item {
        anchors.fill: parent
        LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
        LayoutMirroring.childrenInherit: true

        Flickable {
            id: flick
            anchors { top: parent.top; left: parent.left; right: parent.right; bottom: status.top }
            contentWidth: width
            contentHeight: content.implicitHeight + 32
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar {}

            ColumnLayout {
                id: content
                x: 16
                y: 16
                width: flick.width - 32
                spacing: 24

                DaemonBanner { Layout.fillWidth: true }

                // Greyed while the daemon is gone; the last snapshot stays
                // visible so the window does not jump around (SPEC-UI §5).
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 24
                    visible: backend.hasData
                    enabled: backend.connected
                    opacity: enabled ? 1 : 0.4

                    DeviceSection {
                        title: qsTr("Outputs")
                        empty: qsTr("No outputs")
                        kind: "sink"
                        rows: backend.outputs
                    }
                    DeviceSection {
                        title: qsTr("Inputs")
                        empty: qsTr("No inputs")
                        kind: "source"
                        rows: backend.inputs
                    }
                    StreamSection {}
                    NotifySection {}
                    EqSection {}
                    SafetySection {}
                }
            }
        }

        StatusLine {
            id: status
            anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
        }

        ShortcutOverlay {
            anchors.fill: parent
            visible: win.helpVisible
            onDismissed: win.helpVisible = false
        }
    }
}
