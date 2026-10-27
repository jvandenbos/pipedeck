import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Label {
    property real percent
    // Reads translator.language so the number re-renders with the new
    // locale's digits and separators on a language switch.
    text: (translator.language, qsTr("%1 %").arg(Math.round(percent).toLocaleString(Qt.locale(), 'f', 0)))
    horizontalAlignment: Text.AlignRight
    Layout.minimumWidth: 44
    font.features: { "tnum": 1 }
    opacity: 0.8
}
