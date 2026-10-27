import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts

// Outputs and Inputs share one shape (SPEC-UI §4.1, §4.2).
ColumnLayout {
    id: section
    property string title
    property string empty
    property string kind // "sink" or "source"
    property var rows

    Layout.fillWidth: true
    visible: backend.features.devices === true
    spacing: 12

    SectionTitle { text: section.title }

    Label {
        visible: section.rows.count === 0
        text: section.empty
        opacity: 0.6
    }

    ButtonGroup { id: defaults }

    Repeater {
        model: section.rows
        delegate: DeviceEntry {
            kind: section.kind
            group: defaults
        }
    }
}
