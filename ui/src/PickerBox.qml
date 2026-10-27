import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material

// A ComboBox that tells the window when its popup is open, so Esc closes the
// picker before it closes the window (SPEC-UI §4). Choices are
// [{value, text}]; `current` is the value to show as selected.
ComboBox {
    id: box
    property var choices: []
    property string current

    model: choices
    textRole: "text"
    valueRole: "value"
    currentIndex: {
        for (let i = 0; i < choices.length; ++i)
            if (choices[i].value === current)
                return i
        return -1
    }
    Material.roundedScale: theme.radius
    font.pixelSize: 13
    implicitHeight: 40
    topInset: 0
    bottomInset: 0

    Connections {
        target: box.popup
        function onOpened() { box.ApplicationWindow.window.activePicker = box }
        function onClosed() {
            if (box.ApplicationWindow.window && box.ApplicationWindow.window.activePicker === box)
                box.ApplicationWindow.window.activePicker = null
        }
    }
}
