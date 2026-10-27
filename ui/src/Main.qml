import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import "Columns.js" as Cols

// SoundSource's window on PipeDeck's data (SPEC-UI §4): a header, the System
// block (Output, Input, Sound Effects), the Applications block, a status
// footer; the gear swaps the blocks for the Settings page.
ApplicationWindow {
    id: win
    width: 560
    // Hugs its content, from 300 px up to 720; past that the Applications
    // block scrolls.
    height: Math.max(minimumHeight, Math.min(720, Math.ceil(root.implicitHeight)))
    minimumWidth: 520
    minimumHeight: 300
    visible: true
    title: "PipeDeck"
    color: "#0e0e10"

    Material.theme: Material.Dark
    Material.accent: theme.accent
    Material.roundedScale: theme.radius

    property bool helpVisible: false
    property bool settingsOpen: false
    property bool fxOpen: false      // the Output row's effects panel
    // The open popup (a device menu or a picker), so Esc closes it first.
    property var activePopup: null
    readonly property bool textFocus: activeFocusItem !== null
                                      && (activeFocusItem instanceof TextInput || activeFocusItem instanceof TextEdit)
    readonly property bool showBlocks: backend.hasData && backend.state !== "absent"

    function closeTopmost() {
        if (activePopup) {
            activePopup.close()
            activePopup = null
        } else if (helpVisible) {
            helpVisible = false
        } else if (fxOpen && !settingsOpen) {
            fxOpen = false
        } else if (settingsOpen) {
            settingsOpen = false
        } else {
            win.close()
        }
    }

    // "?" is Shift+/ on most layouts; listing both keeps it working whether or
    // not the platform's keymapper drops the Shift before matching.
    Shortcut { sequences: ["?", "Shift+?"]; context: Qt.ApplicationShortcut; onActivated: win.helpVisible = !win.helpVisible }
    Shortcut { sequence: "Q"; context: Qt.ApplicationShortcut; enabled: !win.textFocus; onActivated: Qt.quit() }
    Shortcut { sequence: ","; context: Qt.ApplicationShortcut; enabled: !win.textFocus; onActivated: win.settingsOpen = !win.settingsOpen }
    Shortcut { sequence: "Esc"; context: Qt.ApplicationShortcut; onActivated: win.closeTopmost() }
    Shortcut { sequence: "F5"; context: Qt.ApplicationShortcut; onActivated: backend.refresh() }

    // Keyboard navigation scrolls the focused control into view when it is
    // in the Applications list, the one part of the window that scrolls.
    onActiveFocusItemChanged: {
        const item = activeFocusItem
        let inside = false
        for (let p = item; p; p = p.parent)
            if (p === appsFlick.contentItem) {
                inside = true
                break
            }
        if (!inside)
            return
        const top = item.mapToItem(appsFlick.contentItem, 0, 0).y
        const bottom = top + item.height
        if (top < appsFlick.contentY)
            appsFlick.contentY = Math.max(0, top - 4)
        else if (bottom > appsFlick.contentY + appsFlick.height)
            appsFlick.contentY = Math.min(appsFlick.contentHeight - appsFlick.height, bottom - appsFlick.height + 4)
    }

    function placeOf(places) {
        for (const p of places)
            if (p.current)
                return p
        return null
    }

    // Right-to-left languages mirror the window; the Translator sets the
    // application's layout direction.
    Item {
        anchors.fill: parent
        LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
        LayoutMirroring.childrenInherit: true

        ColumnLayout {
            id: root
            anchors.fill: parent
            spacing: 0

            // ---- header ---------------------------------------------------
            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: 48
                Layout.maximumHeight: 48
                implicitHeight: 48

                Label {
                    anchors.centerIn: parent
                    textFormat: Text.StyledText
                    text: "<b>Pipe</b>Deck"
                    font.pixelSize: 18
                }
                RowLayout {
                    anchors { right: parent.right; rightMargin: 12; verticalCenter: parent.verticalCenter }
                    spacing: 8
                    Rectangle {
                        objectName: "daemonDot"
                        implicitWidth: 8
                        implicitHeight: 8
                        radius: 4
                        color: backend.state === "connected" ? win.Material.accent
                             : backend.state === "absent" ? "#ff6b6b" : "#7a7a84"
                        HoverHandler { id: dotHover; margin: 6 }
                        ToolTip.visible: dotHover.hovered
                        ToolTip.text: backend.lastError.length > 0 ? backend.lastError
                                    : backend.state === "connected"
                                      ? (backend.version.length > 0 ? qsTr("Connected to pipedeckd %1").arg(backend.version) : qsTr("Connected to pipedeckd"))
                                    : backend.state === "absent" ? qsTr("PipeDeck daemon not running") : qsTr("Connecting…")
                    }
                    ToolButton {
                        objectName: "settings"
                        implicitWidth: 34
                        implicitHeight: 34
                        padding: 6
                        icon.source: "qrc:/icons/settings.svg"
                        icon.width: 22
                        icon.height: 22
                        icon.color: win.settingsOpen ? win.Material.accent : win.Material.foreground
                        Accessible.name: qsTr("Settings")
                        ToolTip.visible: hovered
                        ToolTip.text: qsTr("Settings")
                        ToolTip.delay: 600
                        onClicked: win.settingsOpen = !win.settingsOpen
                    }
                }
            }

            // ---- main page ------------------------------------------------
            DaemonBanner {
                Layout.fillWidth: true
                Layout.margins: 16
                visible: !win.settingsOpen && !win.showBlocks
            }

            ColumnLayout {
                id: blocks
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.leftMargin: 12
                Layout.rightMargin: 12
                spacing: 10
                visible: !win.settingsOpen && win.showBlocks
                // Greyed while reconnecting; the last snapshot stays so the
                // window does not jump around (SPEC-UI §5).
                enabled: backend.connected
                opacity: enabled ? 1 : 0.4

                BlockPanel {
                    id: system
                    key: "system"
                    Layout.fillWidth: true
                    title: qsTr("System")
                    visible: backend.features.devices === true
                    captions: [{ text: qsTr("Volume"), width: Cols.volume }, { text: qsTr("Boost"), width: Cols.boost },
                               { text: qsTr("Device"), width: Cols.device }, { text: qsTr("FX"), width: Cols.fx }]

                    Repeater {
                        model: backend.defaultOutput
                        delegate: ColumnLayout {
                            id: outputRow
                            required property var model
                            Layout.fillWidth: true
                            spacing: 0
                            readonly property var place: win.placeOf(backend.outputPlaces)
                            MixerRow {
                                Layout.fillWidth: true
                                key: "output"
                                glyph: "speaker-box"
                                title: qsTr("Output")
                                titleTip: outputRow.model.description
                                accessibleName: qsTr("Output")
                                nodeId: outputRow.model.nodeId
                                percent: outputRow.model.percent
                                muted: outputRow.model.mute
                                boost: outputRow.model.boost
                                maxPercent: outputRow.model.maxPercent
                                deviceButton.text: outputRow.place ? outputRow.place.label : outputRow.model.nick
                                deviceButton.glyph: outputRow.place ? outputRow.place.glyph : "speaker"
                                deviceButton.tip: qsTr("Choose the system's output device")
                                deviceButton.entries: backend.outputPlaces
                                deviceButton.enabled: backend.features.defaults === true
                                deviceButton.onChosen: entry => backend.choosePlace("sink", entry.nodeId, entry.route)
                                hasFx: true
                                fxOpen: win.fxOpen
                                selected: win.fxOpen
                                onFxToggled: win.fxOpen = !win.fxOpen
                            }
                            FxPanel {
                                Layout.fillWidth: true
                                visible: win.fxOpen
                                model: outputRow.model
                            }
                        }
                    }
                    Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Cols.pad + Cols.lead + 8
                        Layout.preferredHeight: Cols.rowHeight
                        verticalAlignment: Text.AlignVCenter
                        visible: backend.defaultOutput.count === 0
                        text: qsTr("No output device")
                        opacity: 0.55
                    }

                    Repeater {
                        model: backend.defaultInput
                        delegate: MixerRow {
                            id: inputRow
                            required property var model
                            readonly property var place: win.placeOf(backend.inputPlaces)
                            Layout.fillWidth: true
                            key: "input"
                            glyph: "mic"
                            input: true
                            title: qsTr("Input")
                            titleTip: model.description
                            accessibleName: qsTr("Input")
                            nodeId: model.nodeId
                            percent: model.percent
                            muted: model.mute
                            boost: model.boost
                            maxPercent: model.maxPercent
                            deviceButton.text: place ? place.label : model.nick
                            deviceButton.glyph: place ? place.glyph : "mic"
                            deviceButton.tip: qsTr("Choose the system's input device")
                            deviceButton.entries: backend.inputPlaces
                            deviceButton.enabled: backend.features.defaults === true
                            deviceButton.onChosen: entry => backend.choosePlace("source", entry.nodeId, entry.route)
                        }
                    }
                    Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Cols.pad + Cols.lead + 8
                        Layout.preferredHeight: Cols.rowHeight
                        verticalAlignment: Text.AlignVCenter
                        visible: backend.defaultInput.count === 0
                        text: qsTr("No input device")
                        opacity: 0.55
                    }

                    // The daemon routes notification sounds; it does not own
                    // their level, so the level cells stay empty.
                    MixerRow {
                        id: effects
                        Layout.fillWidth: true
                        visible: backend.features.notifications === true
                        key: "effects"
                        glyph: "bell"
                        title: qsTr("Sound Effects")
                        titleTip: qsTr("Where notification sounds play: streams with the event or Notification role, and the apps listed in notification_apps. A redirect picked for an application wins over this.")
                        hasLevel: false
                        readonly property string sink: backend.notificationSink
                        readonly property var known: {
                            for (const p of backend.sinkPlaces)
                                if (p.name === sink)
                                    return p
                            return null
                        }
                        warning: sink.length > 0 && known === null
                        warningTip: qsTr("Sound effects are set to play on %1, which is not present; they play on the default output until it returns.").arg(sink)
                        deviceButton.text: sink.length === 0 ? qsTr("Follow Output") : known ? known.label : sink
                        deviceButton.glyph: sink.length === 0 ? "arrow-up" : known ? known.glyph : "speaker"
                        deviceButton.tip: qsTr("Choose where notification sounds play")
                        deviceButton.entries: {
                            const list = [{ value: "", label: qsTr("Follow Output"), glyph: "arrow-up", current: sink === "" }]
                            for (const p of backend.sinkPlaces)
                                list.push({ value: p.name, label: p.label, glyph: p.glyph, current: p.name === sink })
                            if (warning)
                                list.push({ value: sink, label: qsTr("%1 (absent)").arg(sink), glyph: "warning", current: true })
                            return list
                        }
                        deviceButton.onChosen: entry => backend.setNotificationSink(entry.value)
                    }
                }

                BlockPanel {
                    id: apps
                    key: "applications"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.maximumHeight: implicitHeight
                    Layout.minimumHeight: Math.min(implicitHeight, 40 + 2 * Cols.rowHeight)
                    visible: backend.features.streams === true
                    title: qsTr("Applications")
                    titleTip: qsTr("Rows are streams, not apps: one app can own several, and a restarted stream is a new row. A redirect you pick lasts as long as that stream.")
                    captions: [{ text: qsTr("Volume"), width: Cols.volume }, { text: qsTr("Boost"), width: Cols.boost },
                               { text: qsTr("Redirect Audio To"), width: Cols.redirect }]

                    Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Cols.pad + Cols.lead + 8
                        Layout.preferredHeight: Cols.rowHeight
                        verticalAlignment: Text.AlignVCenter
                        visible: backend.streams.count === 0
                        text: qsTr("No applications are playing audio.")
                        opacity: 0.55
                    }
                    Flickable {
                        id: appsFlick
                        objectName: "applications"
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        Layout.preferredHeight: contentHeight
                        Layout.maximumHeight: contentHeight
                        visible: backend.streams.count > 0
                        contentWidth: width
                        contentHeight: streamColumn.implicitHeight
                        implicitHeight: contentHeight
                        clip: true
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: ScrollBar {}

                        Column {
                            id: streamColumn
                            width: appsFlick.width
                            Repeater {
                                model: backend.streams
                                delegate: MixerRow {
                                    id: streamRow
                                    required property var model
                                    width: streamColumn.width
                                    key: String(model.nodeId)
                                    appIcon: model.icon
                                    title: model.appLabel
                                    detail: model.mediaLabel
                                    titleTip: model.label
                                    accessibleName: model.label
                                    nodeId: model.nodeId
                                    percent: model.percent
                                    muted: model.mute
                                    boost: model.boost
                                    maxPercent: model.maxPercent
                                    hasFxColumn: false
                                    hasDevice: backend.features.streamTarget === true
                                    readonly property var known: {
                                        for (const p of backend.sinkPlaces)
                                            if (p.name === model.target)
                                                return p
                                        return null
                                    }
                                    // Requested vs observed (SPEC-UI §4.3); an older daemon
                                    // sets neither flag.
                                    warning: model.targetAbsent || model.showObserved
                                    warningTip: model.observedLabel.length === 0
                                                ? qsTr("Asked for %1, which is not present").arg(model.targetLabel)
                                                : qsTr("Asked for %1, playing on %2")
                                                  .arg(model.target.length === 0 ? qsTr("the default output")
                                                       : model.targetAbsent ? qsTr("%1 (absent)").arg(model.targetLabel) : model.targetLabel)
                                                  .arg(model.observedLabel)
                                    deviceButton.objectName: "redirect:" + model.nodeId
                                    deviceButton.text: model.target.length === 0 ? qsTr("No Redirect") : model.targetLabel
                                    deviceButton.glyph: model.target.length === 0 ? "arrow-up" : known ? known.glyph : "speaker"
                                    deviceButton.tip: qsTr("Redirect %1 to another output").arg(model.label)
                                    deviceButton.entries: {
                                        const target = model.target
                                        const list = [{ value: "", label: qsTr("No Redirect"), glyph: "arrow-up", current: target === "" }]
                                        for (const p of backend.sinkPlaces)
                                            list.push({ value: p.name, label: p.label, glyph: p.glyph, current: p.name === target })
                                        // A request (or, from an older daemon, a link)
                                        // to a sink not listed stays itself, so the menu
                                        // never silently shows another choice.
                                        if (target !== "" && known === null)
                                            list.push({ value: target, glyph: "speaker", current: true,
                                                        label: model.targetAbsent ? qsTr("%1 (absent)").arg(model.targetLabel) : model.targetLabel })
                                        return list
                                    }
                                    deviceButton.onChosen: entry => {
                                        if (entry.value === "")
                                            backend.followDefault(model.nodeId)
                                        else
                                            backend.setStreamTarget(model.nodeId, entry.value)
                                    }
                                }
                            }
                        }
                    }
                }

                Item { Layout.fillHeight: true }
            }

            // Takes the slack under the banner (the window's minimum height).
            Item {
                Layout.fillHeight: true
                visible: !win.settingsOpen && !win.showBlocks
            }

            SettingsPage {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.margins: 16
                visible: win.settingsOpen
            }

            // ---- footer: the last daemon error, nothing otherwise ----------
            Rectangle {
                Layout.fillWidth: true
                Layout.topMargin: 6
                Layout.maximumHeight: 26
                implicitHeight: 26
                color: "transparent"
                Label {
                    objectName: "lastError"
                    anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 14 }
                    text: backend.lastError
                    color: "#ff8a80"
                    font.pixelSize: 12
                    elide: Text.ElideRight
                    HoverHandler { id: errorHover }
                    ToolTip.visible: errorHover.hovered && truncated
                    ToolTip.text: backend.lastError
                }
            }
        }

        ShortcutOverlay {
            anchors.fill: parent
            visible: win.helpVisible
            onDismissed: win.helpVisible = false
        }
    }
}
