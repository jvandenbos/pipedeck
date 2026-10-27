import QtQuick
import QtQuick.Controls.impl

// One of the bundled monochrome icons (qrc:/icons/<name>.svg), tinted.
Item {
    id: glyph
    property string name
    property color color: "white"
    property int size: 18
    implicitWidth: size
    implicitHeight: size

    ColorImage {
        anchors.centerIn: parent
        width: glyph.size
        height: glyph.size
        source: glyph.name.length > 0 ? "qrc:/icons/" + glyph.name + ".svg" : ""
        sourceSize: Qt.size(glyph.size * 2, glyph.size * 2)
        color: glyph.color
        fillMode: Image.PreserveAspectFit
    }
}
