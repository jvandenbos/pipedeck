import QtQuick
import Quickshell
import Quickshell.Services.Pipewire
import qs.Ui
import qs.Commons as Commons

// The PipeDeck bar entry: SoundSource's menu-bar icon, for Omarchy. A click
// opens (or raises) pipedeck-ui; the tooltip says where sound is going now.
// Routing itself stays in the window and the daemon, so this widget reads
// only what Quickshell's own Pipewire service already exposes.
BarWidget {
  id: root
  moduleName: "pipedeck.bar"

  Commons.Translations { id: strings; directory: Qt.resolvedUrl("i18n") }

  readonly property var sink: Pipewire.defaultAudioSink
  readonly property bool muted: sink && sink.audio ? sink.audio.muted : false
  readonly property int percent: sink && sink.audio ? Math.round(sink.audio.volume * 100) : 0
  readonly property string sinkName: {
    if (!sink) return ""
    var s = String(sink.nickname || sink.description || sink.name || "")
    // Node names arrive from PipeWire; strip anything markup-shaped before
    // it reaches the shared tooltip, which the plugin cannot set to PlainText.
    return s.replace(/[<>&]/g, "").slice(0, 80)
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  PwObjectTracker { objects: root.sink ? [root.sink] : [] }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "󰙪"
    tooltipText: !root.sink
      ? strings.t("tooltip.noOutput")
      : (root.muted
          ? strings.t("tooltip.muted").arg(root.sinkName)
          : strings.t("tooltip.output").arg(root.sinkName).arg(Commons.I18n.formatNumber(root.percent)))
    onPressed: function(b) {
      if (b === Qt.RightButton) root.bar.run("omarchy-shell shell toggle omarchy.audio")
      else root.bar.run("pipedeck-ui")
    }
  }
}
