import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

ColumnLayout {
    Layout.fillWidth: true
    visible: backend.features.streams === true
    spacing: 12

    SectionTitle {
        text: qsTr("Playback streams")
        help: qsTr("Rows are streams, not apps: one app can own several, and a restarted stream is a new row. A destination you pick lasts as long as that stream.")
    }

    Label {
        visible: backend.streams.count === 0
        text: qsTr("Nothing is playing")
        opacity: 0.6
    }

    Repeater {
        model: backend.streams
        delegate: StreamEntry {}
    }
}
