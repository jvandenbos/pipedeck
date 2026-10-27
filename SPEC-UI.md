# PipeDeck — SPEC-UI v1.0 (2026-10-26): the Qt front end for Omarchy / Hyprland

Companion to `SPEC.md` (the daemon contract). This file specifies `pipedeck-ui`, a standalone
Qt Quick window that gives Omarchy what its stock audio panel lacks: per-stream routing,
notification routing, ports, EQ and the loudness-safety controls — all served by the existing
`pipedeckd` over D-Bus. Target host first: **gargoyle** (ThinkPad T14 Gen 1 AMD, Omarchy
4.5.0rc1, Hyprland 0.56, PipeWire 1.6.9, WirePlumber 0.5.18, Qt 6.12). Then any Omarchy box.

The UI **follows SoundSource's layout** (Jan, 2026-10-27, from screenshots: a System block —
Output / Input / Sound Effects — and an Applications block, each row with Volume · Boost ·
Device · FX, and an icon-and-checkmark device menu), rendered in Omarchy's dark Material
register with the theme accent. An earlier draft with a stacked list of every device, and a
proposed "hero card" layout, were both rejected. Everything else follows Omarchy's own app
conventions.

## 0. Decisions and what is already verified

Design reviewed with Codex on 2026-10-26 (`docs/2026-10-26-ui-design-review.md`). Decisions:

| Question | Decision | Why |
|---|---|---|
| Engine | **`pipedeckd` unchanged** (plus the additive items in §6) | It is the rules engine that must run with no window open (notification routing on new streams, EQ re-apply on hotplug, caps) and it already proved portable |
| Front end | **Standalone Qt Quick app, QtDBus client**, not an omarchy-shell plugin | Quickshell's Pipewire QML module cannot write metadata, Route params or load modules — every feature we add would be CLI-over-Process glue inside an RC-stage plugin API. A Qt app is typed, live (`PropertiesChanged`), testable offscreen, packageable |
| Stock panel | **Kept.** `omarchy.audio` (`SUPER+CTRL+A`) stays the quick volume/output popup | One interaction surface per job; revisit a bar launcher after real use |
| Name | **`pipedeck-ui`** binary, desktop id and Wayland app id; `pipedeck` stays the Rust CLI, `pipedeckd` the daemon | Replacing a working CLI to satisfy a naming convention buys nothing |
| Repo | **This repo**, lane `ui/`, one CHANGELOG; the checked-in D-Bus XML is the binding contract | Interface and clients co-evolve; the UI must still tolerate an older daemon (§5) |
| Window | **xdg toplevel**, single instance, floating by Hyprland rule the user adds | Layer-shell brings panel-like focus/stacking semantics for no gain; the file chooser needs ordinary focus |
| EQ mechanism | **Smart filter, as in SPEC §7** — gated by acceptance 20 | Omarchy's docs report smart filters passing audio unprocessed on PW 1.6.8 / WP 0.5.15; **measured working here on PW 1.6.9 / WP 0.5.18** (below). No fronting-sink fallback in v1 — if the real-sink test fails, EQ is deferred, not redesigned |

Verified on gargoyle, 2026-10-26, with the v1.3.0 tree:
- `cargo build --release` of `pipedeckd` + `pipedeck` succeeds unmodified (cargo 1.98, 69 s;
  needs `pipewire`, `alsa-lib`, `clang`, `pkg-config` — all present on Omarchy).
- `pipedeck status/outputs/ports/automute` list the real graph: sink 58 `ALC257 Analog`
  (`alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink`, UCM profile), sources 59/60
  (`HiFi__Mic2__source` unavailable, `HiFi__Mic1__source` digital mic, default), one route
  per node, auto-mute control present and off. **UCM means the headphone jack will most
  likely appear as a separate sink, not a port** — design for both (§4.1).
- **EQ measured:** a 100 Hz tone into a null sink captured post-EQ (`pw-record
  node.autoconnect=false`, `pw-link` to `<sink>:monitor_*`): RMS 4633.5 off → 18313.2 with a
  +12 dB low shelf at 400 Hz (×3.95 = +11.9 dB) → 4633.5 off again. `pw-link -l` showed
  `pipedeck.eq.<sink>.out` feeding the sink. The daemon left no state behind once the
  preset was removed (config dir deleted after the test).

## 1. Scope

### 1.1 What Omarchy already gives (don't rebuild)
Master volume + OSD on the media keys (`omarchy-audio-output-volume`, resolves through DSP
sinks), output cycling (`SHIFT+XF86AudioMute`), the `omarchy.audio` bar popup (master slider,
output/input pickers, per-app volume), the Microphone bar widget, Bluetooth connect/profile in
`omarchy.bluetooth`, per-laptop speaker tunings (`omarchy-audio-tuning`, none match the T14).

### 1.2 v1.0 — what `pipedeck-ui` adds
1. **Output** — one row for the default sink (volume, mute, Boost, device menu, FX); the
   device menu lists every sink, with a sink's ports flattened into separate entries. §4.1
2. **Input** — the same row for the default source.
3. **Playback streams** — every playback stream: label, volume, mute, and a **destination
   picker** (Follow default · each output). §4.3 defines the semantics.
4. **Notifications** — which output `media.role=event` streams go to. §4.4
5. **Equalizer** — per output: Off or one of the installed presets. §4.5 (import stays in
   the CLI: `pipedeck eq import`.)
6. **Safety** — the port-switch cap (`PortSwitchCap`) in Settings, the ALSA auto-mute switch
   in the Output row's FX panel where the card has one, and a per-row UI-local **Boost**
   toggle that widens that slider to 150 %. §4.6
7. **Status** — daemon version/connection state, the last error, in the window. §5

### 1.3 Non-goals (v1.0)
Meters (v1.1), a device-switch level cap (v1.1, daemon work), AutoEq import in the window
(v1.1 via a daemon method), a bar widget/launcher, replacing the stock panel, per-app effects,
LV2/plugin hosting UI, recording, Super-Volume-Keys (Omarchy's keys already drive any sink),
card-profile / Bluetooth-codec switching (Omarchy's BT panel owns it), a bundled generic
"laptop speakers" preset (speaker correction belongs to a measured hardware model — the
user may still drop any preset file in `~/.config/pipedeck/eq/`), the fronting-sink EQ
fallback, and anything that touches Omarchy's own config or `/usr/share/omarchy-platform/`.

## 2. Architecture

```
 pipedeck-ui (Qt Quick, C++)  ──QtDBus session bus──▶  pipedeckd (Rust)  ──libpipewire──▶ PipeWire
 pipedeck CLI (Rust)            ─────┘                        │
 GNOME extension (chronos)      ─────┘                        └── ~/.config/pipedeck/
```

- `ui/` is a qmake6 project laid out per Omarchy's app conventions (the `omarchy-app` guide
  on gargoyle, `~/.claude/skills/omarchy-app/` with `templates.md` for the `Theme` and
  `Translator` classes): `ui/pipedeck-ui.pro`, `ui/bin/{build,test,install}`,
  `ui/src/{main.cpp,backend.h,backend.cpp,daemon.h,daemon.cpp,theme.*,translator.*,
  Main.qml,*.qml,resources.qrc,i18n/}`, `ui/tests/`, `ui/pkgbuild/`. `src/` stays flat.
- **C++ owns everything that can fail**: the D-Bus proxy, the model (devices, ports,
  streams, EQ, caps as `QAbstractListModel`s or `QVariantList` properties with `NOTIFY`),
  coalescing, reconnection, error text. **QML owns layout and keys only.**
- **D-Bus proxy is hand-written** (`daemon.h/.cpp`, a `QDBusAbstractInterface` subclass):
  the build host has no `qt6-tools`, so no `qdbusxml2cpp`, and the interface is small
  (10 methods, 11 properties, 1 signal). `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml`
  stays the contract: a unit test parses it and asserts every method, property and signal
  is covered with the same signature. Every struct-array property (`a(usssbbdbs)` etc.) gets
  a registered tuple type with `QDBusArgument` stream operators, and the fake-daemon tests
  round-trip each one at wire level — signature coverage alone does not prove decoding.
  Property reads go through `org.freedesktop.DBus.Properties` `GetAll`/`Get`,
  asynchronously. Every call except `StartServiceByName` goes to the owner's **unique**
  name with auto-start off, so a call issued against one daemon generation can never reach
  its replacement. qmake: `QT += quick quickcontrols2 dbus network svg`.
- `QGuiApplication`, `setDesktopFileName("pipedeck-ui")`, Material dark style, accent and
  corner radius from the Omarchy theme with the guide's fallbacks, strings through `qsTr`,
  language from `~/.config/omarchy/language`. Never modify Omarchy's config.
- No shelling out to `wpctl`/`pactl`/`pw-cli` from the app (platform-library rule, SPEC §4).
  The one exception is `hyprctl getoption decoration:rounding` via `QProcess`, as the guide
  prescribes for the corner radius.
- **Single instance**: a `QLocalServer` on `$XDG_RUNTIME_DIR/pipedeck-ui.sock`, guarded by
  a `QLockFile` (`pipedeck-ui.lock`) for primary election: the lock holder takes the lock at
  start but listens only once its window is loaded, so `ok` means a window was shown. A
  second launch sends `show` and waits ≤1 s for `ok`; if nothing answers it tries for the
  lock again (5 rounds) and runs as the primary once the lock is free, else exits 1. A stale socket is removed
  **only** after the lock was acquired (Qt warns never to unlink a running instance's socket
  just because `listen()` failed). `show` = restore from minimised, `requestActivate()`,
  raise. Launcher and keybind both go through this.

## 3. Ownership contract — who decides what on a laptop

WirePlumber owns **device selection and per-device level restore**; PipeDeck owns **explicit
routing, notification routing, EQ, and its documented caps**. The UI never becomes a second
device-preference engine. Consequences, decided now:

| Event | Behaviour |
|---|---|
| Headphones / BT headset appear | WirePlumber's choice stands. The UI shows the new default; no automatic switch of our own. |
| The selected output disappears | WirePlumber falls back; streams routed explicitly to the vanished sink **fall back to default too** (PipeWire behaviour) and the row shows *requested: X (absent) · playing on: Y*. The daemon keeps the request and WirePlumber re-links when X returns (`target.object` persists on the stream). |
| Dock HDMI returns | No restore of a "previous preference" in v1. |
| Suspend / resume | The daemon re-applies EQ and notification routing when sinks reappear (SPEC §7.1, §2.1); the UI reconnects (§5). Nothing overrides a newer user choice. |
| Another controller changes the default or a level | Observed and shown. The port-switch cap applies only to PipeDeck-initiated switches (SPEC §9.2) — v1 **does not claim general switch protection**; the v1.1 restored-level cap (§10) is the follow-up. |
| Omarchy speaker tuning present on a box | PipeDeck's EQ targets the **physical** sink (what `omarchy-audio-output-sink` would resolve to), never `omarchy_speaker_tuning`; the tuning sink is listed but its EQ control is disabled with a reason. Not exercised on the T14 (no tuning). |

## 4. The window

Rogue Amoeba SoundSource's window, mapped onto PipeDeck's data (Jan, 2026-10-27, from two
reference screenshots; this replaces the stacked list of every device of v1.0), drawn in
Omarchy's dark Material register with the theme accent where SoundSource uses green.
Screenshots: `ui/docs/pipedeck-ui{,-devices,-fx}.png`.

- **Geometry**: one `ApplicationWindow`, 560 px wide (SoundSource's proportion), minimum
  520×300. The window **hugs its content**: its height follows the blocks, from 300 px up to
  720 px; past that the Applications list scrolls inside its block and nothing else moves.
  Rows are 36 px. Blocks are rounded panels a shade lighter than the window, radius
  `min(10, theme radius)`; buttons round to `min(height/2, theme radius)`, so Omarchy's
  square default (`decoration:rounding 0`) gives square controls. Accent and radius follow
  the theme live.
- **Header**: "**Pipe**Deck" centred (bold, regular, like "**Sound**Source"); at the right a
  daemon-state dot (accent = connected, grey = connecting, red = absent; tooltip = the version,
  the state, or the last error) and a settings gear. No pin or window buttons — Hyprland owns
  those.
- **System block** (§4.1, §4.4) and **Applications block** (§4.3), each with a collapse chevron,
  a title and dim column captions ("Volume · Boost · Device · FX" and "Volume · Boost ·
  Redirect Audio To") that sit exactly over their columns: every cell after the title has a
  fixed width, so all rows line up (§4.2).
- **Footer**: one thin line with the last daemon error (§5), empty otherwise. SoundSource's
  footer buttons have no PipeDeck equivalent and are not invented.
- **Settings page** (gear or `,`; `Esc` or the gear returns): §4.6.
- **Daemon absent**: the main page shows only "PipeDeck daemon not running", one line of help
  and **Start** (§5). While reconnecting with an earlier snapshot the blocks stay, greyed.
- **Keys**: Tab/Shift+Tab reach every control in reading order (header, System, Applications),
  and the Applications list scrolls to keep the focused control wholly in view. `?` toggles the
  shortcut overlay; `Q` quits unless a text field has focus; `,` toggles Settings (not in a
  text field); `F5` asks the daemon to re-read the graph (`Refresh`). **`Esc` closes the
  topmost thing**: an open menu or picker, then the shortcut overlay, then the effects panel,
  then Settings, then the window. (The overlay comes before the effects panel because it covers
  it: closing something hidden under it would look like a dead key.)
- Right-to-left languages mirror the window. Every string is translated (`src/i18n/`).

### 4.1 System block: Output and Input
- **Output row**: speaker glyph · "Output" · mute · slider · % · Boost · device menu · FX
  chevron. The row drives the **default sink** (`Devices` with `kind=sink` and `is_default`):
  slider and mute write to that node, and when the default changes — here, from Omarchy's
  panel, or because WirePlumber picked a hotplugged device (§3) — the row follows it. No
  default sink → one dim line "No output device".
- **Input row**: the same for the default source, with a microphone glyph, and no FX.
- **Device menu — places.** The menu lists *places*, not raw nodes: a device with ≥2
  `available` ports (`Ports`) is one place per port, named by the port's description
  ("Speaker", "Headphones"); a device with ≤1 port is one place named by its nick (fallback:
  description, then node name). Separate UCM headphone sinks are therefore ordinary places.
  Without the `Ports` property or `SetPort` (older daemon) every device is one place.
- **Choosing a place**: `SetDefault(kind, name)` unless the device is already the default,
  then `SetPort(node, route)` only when the place is a port that is not active. The current
  place sends nothing. The menu's check mark and the button's label follow the next snapshot,
  never the click, so a rejected `SetDefault` leaves the old place shown (§5).
- **Glyphs**, from node and port names (`Backend::glyphFor`): bluetooth for `bluez*`; for a
  sink, monitor for HDMI/DisplayPort, headphones for a headphone/headset port (or a portless
  node named so), USB for a `usb` node, speaker otherwise; for a source, USB or microphone.

### 4.2 Row anatomy and the volume contract
- Columns, left to right: glyph or app icon (20 px) · title (takes the rest) · mute (28) ·
  slider (112) · percentage (42) · Boost (30) · device or redirect menu (134; Applications
  rows add the FX column's 30 to it) · FX (26). A row without a level (Sound Effects) gives
  its mute/slider/%/Boost cells to the title instead of drawing empty controls.
- Slider → `SetVolume` coalesced to ≤20/s while dragging, final value on release; mute →
  `SetMute`. Volume and mute are independent writes (SPEC §9.1 — the UI never sends a cached
  mute with a volume). A press and release without movement sends nothing.
- **Volume contract**: the daemon speaks linear volume (`Devices.volume`, `SetVolume` take
  `d`, 1.0 = 100 %). The slider position is `pos = (vol)^(1/3)` and a write is
  `vol = (percent/100)^3` — 50 % → 0.125, 100 % → 1.0, 150 % → 3.375. The slider spans 0–100 %,
  or 0–150 % with that row's Boost on (§4.6). An observed volume above the slider's range is
  shown at the slider's end with the real percentage in the label; the UI never writes a
  clamp back on its own.
- Controls show only what the daemon reports: radio-like state (current place, switch
  positions, EQ preset) moves on the next snapshot, never optimistically.

### 4.3 Applications block (playback streams)
- One row per `Streams` entry: app icon (desktop entry matching `binary`/`app_name`, generic
  glyph otherwise) · app name (`app_name`, fallback `binary`, then `#<id>`) with the media name
  after it in dim text, elided first · mute · slider · % · Boost · **Redirect** menu. No FX
  column and no favourites. pipewire-pulse and Flatpak clients differ only in metadata
  quality, never in control. Empty: one dim line "No applications are playing audio."
- **Semantics, in the block title's tooltip**: rows are *streams*, not apps. One app may own
  several rows; a restarted stream is a new row. A redirect lasts for that stream's lifetime.
- **Redirect menu**: "No Redirect" (= follow the default, `SetStreamTarget(id, "")`) + one entry
  per sink, by the same display name as a place (sinks only: streams target sinks, ports are
  not destinations) → `SetStreamTarget(id, name)`. The current entry is the **request**
  (`target_name`); a request for a sink that is gone stays the current entry as
  "<name> (absent)" instead of snapping elsewhere.
- **Requested vs observed**: a ⚠ before the menu when the observed sink (`StreamSinks`, §6.1)
  differs from where the stream should be (its request, or the default when following it), or
  the request is absent. Tooltip: "Asked for X, playing on Y" (X = "the default output" when
  following it; "X (absent)" when gone), or "Asked for X, which is not present" with no link.
- **Older daemon** (no `StreamSinks`, ≤1.3.0): `target_name` is the linked sink, not a request;
  the menu shows it as current, `pipedeck.eq.<sink>` as `<sink>`, never absent, no ⚠.
- **Precedence**: an explicit redirect beats notification routing for that stream; the daemon
  only re-targets `event` streams it has not seen a manual target for (§6.2). Said in the Sound
  Effects tooltip too.

### 4.4 Sound Effects (notification routing)
The System block's third row: bell glyph · "Sound Effects" · no level controls (the daemon
routes notification sounds, it does not own their level, so nothing is faked) · device menu:
"Follow Output" + each sink → `SetNotificationSink(""|name)`. When the chosen sink is not
present, a ⚠ before the menu ("…plays on the default output until it returns") and the menu
keeps it as "<name> (absent)". The title's tooltip says which streams count (`media.role`
event/Notification, plus `notification_apps`) and the precedence rule of §4.3.

### 4.5 Effects panel (FX)
The Output row's FX chevron discloses a panel under the row, SoundSource-style; the row gets
the accent-tinted background while it is open. It acts on the current default sink:
- **Equalizer**: "Off" + each `EqPresets` entry → `SetEq(node, id|"")`. With no presets, or for
  a virtual sink, or a sink the daemon offers no EQ for, the picker is replaced by one line
  saying why (and, for no presets, where they live and the import command). The daemon rejects
  EQ on anything but real sinks with `InvalidArgument`, which the footer shows (§5).
- **Auto-mute**: when the sink's node has an `AutoMute` row, a switch "Auto-mute speakers when
  headphones are plugged in" → `SetAutoMute`.
Without `Eq`/`SetEq` and without auto-mute the panel says the daemon has no effects for it.

### 4.6 Safety and Settings
- **Boost** (per row, UI-only): the toggle in each row's Boost column widens *that* slider from
  100 % to 150 % (linear 3.375). It enforces nothing in the daemon and changing it writes
  nothing. Persisted in QSettings `ui/boost` as a list of keys: `node:<node.name>` for devices,
  `app:<binary|app_name>` for streams (stream ids do not survive a restart). Replaces v1.0's
  global "Allow up to 150 %" (`ui/allowOverdrive`, no longer read).
- **Settings page**: the daemon's state and version (with **Start** when absent); the
  **port-switch cap**, a spin box 0–150 % / Off bound to `PortSwitchCap` / `SetPortSwitchCap`
  — `u`, an integer **percentage**, 0 = off (unlike `SetVolume`, which is linear `d`), hidden
  on a daemon without it; one line on where EQ presets live and the import command.

### 4.7 Window placement (documented, not installed)
README snippet for `~/.config/hypr/` (Omarchy 4.5 uses Lua bindings): the keybind
`SUPER + CTRL + SHIFT + A` → `pipedeck-ui` (decided by Jan 2026-10-26; the stock panel keeps
`SUPER+CTRL+A`), and the optional window rules `float`, `move` to the top right, `pin`. No
`size` rule: the window sizes itself to its content (§4). The app never writes these.

## 5. Client lifecycle contract
- All D-Bus calls **asynchronous** (`QDBusPendingCallWatcher`); errors surface in a status
  line at the bottom of the window with the daemon's error name and message, and clear on
  the next successful call. No modal dialogs for errors.
- **One refresh state machine.** The daemon emits both standard
  `org.freedesktop.DBus.Properties.PropertiesChanged` (one per changed property) and its own
  `Changed` after each coalesced graph update (`service.rs`). The client subscribes to both
  **before** the initial `GetAll` snapshot, funnels them into one coalescer with **one refresh
  in flight and a trailing dirty flag**, and re-reads with a full `GetAll` (≤10/s). A failed read is retried after 0.5 s, doubling
  to 8 s, while the owner is unchanged; a successful read clears the error line.
  Every reply is tagged with an **owner generation** (bumped on each `NameOwnerChanged`,
  including direct owner replacement) and dropped when stale.
- Daemon absent: show "PipeDeck daemon not running" + a **Start** button that calls
  `org.freedesktop.DBus.StartServiceByName` (D-Bus activation, §7). On `NameOwnerChanged` →
  lost: grey the window, drop every **unsent** write (a call already dispatched simply has
  its reply ignored via the generation tag); → gained: full snapshot again. **Never replay
  queued slider writes across a reconnect** — node ids are new.
- A pending write for a node that vanished is discarded, with no error shown.
- Older daemon: a missing property or method (`UnknownProperty`/`UnknownMethod`) hides that
  control (after the first call returns `UnknownMethod`) until the daemon owner changes,
  instead of failing the window; `Version` is shown in Status. Without `StreamSinks`
  (≤1.3.0) `target_name` is the sink the stream is linked to, not a request: the picker shows
  it as the current value, `pipedeck.eq.<sink>` as `<sink>`, never flagged absent, no
  "playing on".
- Sliders: while dragging, show the local value; on release send the final value and keep
  showing it until either its echo arrives in a snapshot or 300 ms pass with no local input
  — an *older* observed value arriving in between must not overwrite the final request.
  After that, last observed wins.

## 6. Daemon changes (additive; existing signatures unchanged)
1. **`StreamSinks a(us)`** — `(stream id, node.name of the sink it is currently linked to,
   or "")`, one row per playback stream (same rows as `Streams`), derived from `Link` globals
   and emitted with the same coalesced `PropertiesChanged` + `Changed` as the rest. This is the
   *observed* destination. Resolution (`crates/pipedeckd/src/links.rs`):
   - collect the input nodes of every link whose output node is the stream — the per-channel
     links (FL, FR, …) collapse to one destination;
   - only sink nodes count (`Audio/Sink*`); a capture stream recording straight off the
     playback stream is not a destination;
   - a destination that is one of PipeDeck's EQ filters (`pipedeck.eq.<sink>`) reports
     `<sink>`, the physical sink it fronts — the filter is hidden from `Devices` and its
     playback side is pinned to that sink;
   - several distinct sinks at once (a fan-out client, or the instant of a move) → the
     **lowest node id**, so the value does not flicker;
   - no link → `""`.

   **`Streams.target_name` is the *requested* target**, read in WirePlumber 0.5.18's own
   precedence (`linking/find-defined-target.lua`): the `default` metadata's `target.object`,
   else its `target.node`, else the stream node's own `target.object` property (the client's
   connect-time request: `pw-play --target`, `PULSE_SINK`, pipewire-pulse clients), else the
   deprecated node property `node.target`. A numeric `target.object` is an `object.serial`, a
   numeric `target.node` / `node.target` a node **id**; each resolves to `node.name` (or is
   reported raw). `-1` (WirePlumber's "no target") or no value at all = `""` ("Follow default").
   **"Follow default" = `SetStreamTarget(id, "")`**, and it does follow the default: when
   deleting the `target.object` metadata would leave nothing else naming a sink, the daemon
   deletes it; when the node's own `target.object` / `node.target` (or a `target.node` other
   than `-1`) would then win, it writes `target.object = "-1"` instead, so WirePlumber links
   the stream to the default rather than snapping it back to its connect-time request. Either
   way the stream then reads `""`. The daemon also clears its copy of a node's own target
   property when the client drops it from its properties. Through 1.3.0 `target_name`
   returned the live link first, i.e. it was observed state; that changed with this item
   (CHANGELOG `Changed`).
   CLI: `pipedeck status`/`streams` show `-> requested` and `on: observed` when they differ
   (ASCII arrow, matching the rest of the CLI's output).
2. **Manual target precedence**: the daemon remembers stream ids it received a
   `SetStreamTarget` for in this session — `SetStreamTarget(id, "")` included — and skips them
   when (re)applying notification routing (`SetNotificationSink`, new streams, hotplug,
   `Refresh`): never routed, never released. The set is in memory and drops a stream when its
   node disappears. Unit test (`matching::plan_notification_routing`): an `event` stream with a
   manual target is not re-targeted by a later `SetNotificationSink`; one without is.
3. **Arch packaging paths**: `packaging/arch/pipedeckd.service` (user unit,
   `ExecStart=/usr/bin/pipedeckd`) and `packaging/arch/dev.pipedeck.Daemon.service` (D-Bus
   activation, `Exec=/usr/bin/pipedeckd`, `SystemdService=pipedeckd.service`). The top-level
   `packaging/*.service` stay as they are for `install.sh` on Ubuntu (`%h/.local/bin`).
4. **System presets directory**: besides `~/.config/pipedeck/eq/*.toml`, the daemon reads
   `<dir>/pipedeck/presets/*.toml` for each absolute `<dir>` in `$XDG_DATA_DIRS` (default
   `/usr/local/share:/usr/share`), read-only — so the package's `/usr/share/pipedeck/presets/`
   is live without a first-run copy. For one preset id the user directory wins, then earlier
   `$XDG_DATA_DIRS` entries — the first directory holding a *valid* preset for the id, so a
   broken user file does not hide a bundled one. `EqPresets` lists the merged set; `pipedeck eq
   import` still writes only the user dir; `eq show` goes through the daemon's selection and
   shows exactly what `SetEq` applies. A rescan that cannot list an existing preset directory
   keeps the previous library. No system dir present = the 1.3 behaviour exactly.
Everything else the UI needs exists in 1.3.0 (`Devices`, `Streams`, `Ports`, `Eq`,
`EqPresets`, `AutoMute`, `NotificationSink`, `PortSwitchCap`, `Version`, the setters).

## 7. Packaging (Arch / Omarchy)
Written to pass the `omacom/omarchy-pkgs` PR gates (its README: Arch packaging standards,
pinned release source + `sha256sums`, every supported `arch`, a clean-chroot build, valid
`.omarchy/package.json`); the recipe lives here so it can be copied into that repo verbatim.
- **`packaging/arch/PKGBUILD`** builds **one package `pipedeck`** (the whole project — daemon,
  CLI, UI — so it does not belong under `ui/`): `source=` the GitHub release tarball
  `v$pkgver`, `sha256sums` filled with `updpkgsums`, `arch=('x86_64' 'aarch64')`,
  `options=('!debug')`, `license=('MIT')`, `install=pipedeck.install` (desktop-database +
  icon-cache refresh, as hype/omacut do). `build()` runs `cargo build --release --locked
  --workspace` then `ui/bin/build`; `package()` installs `/usr/bin/{pipedeckd,pipedeck,
  pipedeck-ui}`, `/usr/lib/systemd/user/pipedeckd.service` and
  `/usr/share/dbus-1/services/dev.pipedeck.Daemon.service` from `packaging/arch/`,
  `ui/pkgbuild/pipedeck-ui.desktop` → `/usr/share/applications/`, `ui/pkgbuild/pipedeck-ui.svg`
  → `/usr/share/icons/hicolor/scalable/apps/`, the bundled presets →
  `/usr/share/pipedeck/presets/` (read by the daemon via `$XDG_DATA_DIRS`, §6.4 — no
  first-run copy), LICENSE and README.
  `depends`: `pipewire`, `wireplumber`, `alsa-lib`, `qt6-base`, `qt6-declarative`, `qt6-svg`,
  `qt6-wayland`, `hicolor-icon-theme`, plus the directly linked `libpipewire`, `glibc`,
  `libgcc`, `libstdc++`. `makedepends`: `rust`, `clang`, `pkgconf`,
  `qt6-base`, `qt6-declarative`, `qt6-svg` (nothing from `qt6-tools`); `checkdepends`: `dbus`
  (`check()` runs `cargo test` and `ui/bin/test`, which needs no display). `build()`/`check()`
  add `-ffat-lto-objects` to `CFLAGS`: under makepkg's default `lto`, libspa-sys's C shim is
  GIMPLE-only and rust-lld fails on `libspa_rs_type_param`.
- **`packaging/arch/omarchy-pkgs/package.json`** — the `.omarchy/package.json` for that repo:
  `{"source":"local","upstream":{"watch":{"github":"jvandenbos/pipedeck","pattern":"v(?P<version>[0-9]+(?:\\.[0-9]+)*)"}}}`.
- `ui/bin/install` = `makepkg -fsi` in `packaging/arch/` so the app lands in the launcher.
  **On a box where sudo wants a password (gargoyle) the session builds with `makepkg -fd`
  and Jan runs `pacman -U`**; until then `ui/bin/install-user` installs the UI under
  `~/.local/{bin,share/applications,share/icons/hicolor/scalable/apps}` and the root
  `install.sh` keeps installing the daemon/CLI user-level. The user unit is enabled by a
  post-install note, not by the package.
- `install.sh` / `Makefile` at the root keep working on Ubuntu/GNOME; they do not build `ui/`.
- Publishing to `omacom/omarchy-pkgs` is **out of scope** until Jan says so (outward-facing).

## 8. Build and test
- `ui/bin/build` → `build/pipedeck-ui`; `ui/bin/test` → Qt Test offscreen
  (`QT_QPA_PLATFORM=offscreen`), compiling the `../src/*.cpp` it needs **except `main.cpp`**,
  with a **fake daemon**: a small in-process `QDBusAbstractAdaptor` serving
  `dev.pipedeck.Daemon1` on the test's private bus (`dbus-run-session`), so model,
  decoding, coalescing and reconnect logic (owner-generation drops, unsent-write discard,
  the §5 slider ordering) are tested without PipeWire. Theme/Translator tests take temp
  directories as the guide prescribes. Translations are JSON (`src/i18n/<code>.json`) per
  the guide — no `lupdate`/`lrelease` on the build host.
- Offscreen QML harness in the scratchpad for screenshots and interaction checks
  (`QT_FORCE_STDERR_LOGGING=1`).
- Integration on gargoyle only from the main session (fleet rule). `cargo` work on the
  daemon keeps `make check` green (fmt, clippy `-D warnings`, tests, presets).
- Coding: C++20, Qt 6.12, no CMake, no generated files committed, comments explain *why*.

## 9. Acceptance (continues SPEC.md numbering)
20. **EQ on the real sink**: with `pipedeck eq set 58 <strong preset>` and music playing on
    the speaker sink, `pw-link -l` shows the stream → `pipedeck.eq.<sink>` → sink chain, the
    default sink is still the ALSA sink (`wpctl status`), the media keys still move
    `wpctl get-volume 58`, and the post-EQ monitor capture (the §0 method, on the real sink)
    shows the expected gain; `off` restores the direct link. Repeat after
    `systemctl --user restart wireplumber` and after a suspend/resume.
21. `pipedeck-ui` starts with the daemon down, shows the message, **Start** brings it up via
    activation and the window fills within 1 s.
22. Outputs shows the speaker sink; plugging headphones adds either a port selector or a
    new row, and whichever appears works: selecting it moves audio (`pactl list sinks | grep
    "Active Port"` or `wpctl status` default follows).
23. Playback streams: start two players; route one to a second output (HDMI when docked, or
    a `pw-cli` null sink during the test); the row shows requested = observed; unplug the
    target → row shows requested (absent) / observed = default; replug → audio returns to the
    requested sink without any click.
24. Notifications: set a notification output, fire `notify-send` → the event stream lands
    there (`pipedeck status` shows it) while music stays on the default. Then route that
    stream manually elsewhere → changing the notification output again leaves it alone.
25. Slider drag at 60 Hz produces ≤20 `SetVolume` calls/s (`busctl monitor` count), never a
    `mute` key in a volume write; mute from the media key while dragging is not undone.
26. Kill the daemon mid-session: window greys out, no crash, no queued writes replay on
    restart; node ids after restart are used correctly.
27. `omarchy theme set <other>` → accent changes live; `omarchy language da` → no clipped or
    untranslated strings in the five sections; back to `en`.
28. `ui/bin/test` passes offscreen; `makepkg -fsi` installs and the launcher shows PipeDeck.

## 10. v1.1 backlog (not in v1.0, in order)
- **Restored-level cap on observed default-sink changes** (daemon): when the default output
  changes for any reason and the new sink's restored level exceeds the cap, clamp it; "a
  reactive cap can still allow a loud transient" — spec it as a *restored-level* cap and say
  so in the UI. Covers the BT-headset-at-100 % case that matters on a laptop.
- **Meters**: in-app libpipewire capture streams created only while the view is visible;
  monitor-only. Known interaction: Omarchy's Microphone widget lights "in use" for any
  unmuted capture stream it does not exclude by name — fix upstream with a generic marker
  property in `AudioNodesModel.js` rather than impersonating `quickshell-peak-monitor`.
- **`ImportEqPreset(path s) → id s`** on the daemon, then an Import button using the
  xdg-desktop-portal FileChooser (Omacut's `portalfilepicker` pattern).
- Bar widget: a third-party omarchy-shell `bar-widget` plugin whose click runs `pipedeck-ui`
  and whose tooltip shows EQ/routing state. Only after real use says it is wanted.
- Persistent per-app rules (needs a stable identity policy — `application.name` +
  `binary`? decide with data from real clients first).

## 11. Open for Jan
- ~~Keybind~~ **Decided 2026-10-26: `SUPER+CTRL+SHIFT+A`**; the stock panel keeps `SUPER+CTRL+A` (§4.7).
- One package `pipedeck` (daemon + CLI + UI) or split `pipedeck-ui` — §7 assumes one.
- Later, as agreed on 2026-10-26: a review session of the Arch/Linux audio chain itself
  (PipeWire → WirePlumber → UCM/ALSA on the ALC257, Omarchy's tuning host, where EQ and
  volume should live). §0's measurements are its first data points.
