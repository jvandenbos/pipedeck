import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material

// A cubic-percent slider (SPEC-UI §4.2): the value is the percentage the
// user sees, and the backend turns it into linear volume. `maxPercent` is
// 100, or 150 with the row's Boost on. A level above the range sits at the
// end; the label next to it shows the real number, and nothing is clamped
// back on the daemon.
Slider {
    id: slider
    property int nodeId
    property real percent
    property int maxPercent: 100
    objectName: "volume:" + nodeId

    // Material's 48 px touch height doubles every row on a desktop window.
    implicitHeight: 28
    topPadding: 0
    bottomPadding: 0
    leftPadding: 6
    rightPadding: 6
    from: 0
    to: maxPercent
    stepSize: 2
    value: Math.min(percent, to)
    // The Applications list scrolls with the wheel; a slider under the
    // pointer must not eat that and change a level by accident.
    wheelEnabled: false

    // A press and release without movement sends nothing: the release only
    // finishes a gesture that moved. (The backend applies the same rule, so a
    // clamped level is never written back from either path.)
    property bool movedSincePress: false
    onMoved: {
        movedSincePress = pressed
        backend.setVolume(nodeId, value, pressed)
    }
    onPressedChanged: {
        if (pressed)
            movedSincePress = false
        else if (movedSincePress)
            backend.setVolume(nodeId, value, false)
    }
}
