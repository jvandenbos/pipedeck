import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material

// A cubic-percent slider (SPEC-UI §4.1): the value is the percentage the
// user sees, and the backend turns it into linear volume. A level above the
// range sits at the end; the label next to it shows the real number, and
// nothing is clamped back on the daemon.
Slider {
    id: slider
    property int nodeId
    property real percent

    // Material's 48 px touch height doubles every row on a desktop window.
    implicitHeight: 28
    topPadding: 0
    bottomPadding: 0
    from: 0
    to: backend.maxPercent
    stepSize: 2
    value: Math.min(percent, to)
    // The window scrolls with the wheel; a slider under the pointer must not
    // eat that and change a level by accident.
    wheelEnabled: false

    onMoved: backend.setVolume(nodeId, value, pressed)
    onPressedChanged: if (!pressed) backend.setVolume(nodeId, value, false)
}
