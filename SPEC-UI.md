# PipeDeck — SPEC-UI v1.0 (2026-10-26): the Qt front end for Omarchy / Hyprland

Companion to `SPEC.md` (the daemon contract). This file specifies `pipedeck-ui`, a standalone
Qt Quick window that gives Omarchy what its stock audio panel lacks: per-stream routing,
notification routing, ports, EQ and the loudness-safety controls — all served by the existing
`pipedeckd` over D-Bus. Target host first: **gargoyle** (ThinkPad T14 Gen 1 AMD, Omarchy
4.5.0rc1, Hyprland 0.56, PipeWire 1.6.9, WirePlumber 0.5.18, Qt 6.12). Then any Omarchy box.

The UI does **not** copy SoundSource's layout (Jan, 2026-10-26); it borrows the idea of one
place for the whole audio graph and otherwise follows Omarchy's own app conventions.

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
1. **Outputs** — one row per sink with volume, mute, default marker; **ports nested** under
   the sink that owns them (a selector, not peer rows). §4.1
2. **Inputs** — same shape for sources.
3. **Playback streams** — every playback stream: label, volume, mute, and a **destination
   picker** (Follow default · each output). §4.3 defines the semantics.
4. **Notifications** — which output `media.role=event` streams go to. §4.4
5. **Equalizer** — per output: Off or one of the installed presets. §4.5 (import stays in
   the CLI: `pipedeck eq import`.)
6. **Safety** — the port-switch cap (`PortSwitchCap`), the ALSA auto-mute switch where the
   card has one, and a UI-local "allow up to 150 %" slider range toggle. §4.6
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
- **C++ owns everything that can fail**: the D-Bus proxy (generated from
  `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml` with `qdbusxml2cpp` at build time — the
  XML is the contract), the model (devices, ports, streams, EQ, caps as `QAbstractListModel`s
  or `QVariantList` properties with `NOTIFY`), coalescing, reconnection, error text.
  **QML owns layout and keys only.**
- `QGuiApplication`, `setDesktopFileName("pipedeck-ui")`, Material dark style, accent and
  corner radius from the Omarchy theme with the guide's fallbacks, strings through `qsTr`,
  language from `~/.config/omarchy/language`. Never modify Omarchy's config.
- No shelling out to `wpctl`/`pactl`/`pw-cli` from the app (platform-library rule, SPEC §4).
  The one exception is `hyprctl getoption decoration:rounding` via `QProcess`, as the guide
  prescribes for the corner radius.
- **Single instance**: a `QLocalServer` on `$XDG_RUNTIME_DIR/pipedeck-ui.sock`; a second
  launch asks the first to raise/show and exits. Launcher and keybind both go through this.

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

One `ApplicationWindow`, default 420×720, resizable, sections stacked in a `ListView`-free
`ColumnLayout` inside a `Flickable` (few rows, no need for delegates). Section order as §1.2.
Every row is reachable by Tab/arrow keys; `?` shows the shortcut overlay; `Q` quits unless a
text field has focus; `Esc` closes an open picker first, then the window.

### 4.1 Outputs
- One row per `Devices` entry with `kind=sink`, hiding nothing the daemon shows (the daemon
  already hides its own EQ nodes). Row: nick (description in the tooltip), default radio,
  slider (cubic, `pos = vol^(1/3)`, 0–100 %, or 0–150 % with the §4.6 toggle), mute button,
  EQ badge when a preset is active.
- If `Ports` has ≥2 `available` rows for that node, the row grows a **port selector** (a
  compact segmented control: "Speaker · Headphones"); selecting calls `SetPort`. ≤1 port →
  no selector. Separate UCM headphone sinks are ordinary rows — nothing special to do.
- Under a sink whose node has an `AutoMute` row: a switch "Auto-mute speakers when
  headphones are plugged in" → `SetAutoMute`.
- Default radio → `SetDefault("sink", name)`. Slider → `SetVolume` coalesced to ≤20/s while
  dragging, final value on release; mute → `SetMute`. Volume and mute are independent writes
  (SPEC §9.1 — the UI never sends a cached mute with a volume).

### 4.2 Inputs
Same row shape for `kind=source`, no EQ badge, no auto-mute switch.

### 4.3 Playback streams
- **Semantics, stated in the section header's help text**: rows are *streams*, not apps. One
  app may own several rows; a restarted stream is a new row. A destination you pick lasts
  for that stream's lifetime. "Follow default" = `SetStreamTarget(id, "")`.
- Label: `app_name` — `media_name` (either may be empty; fall back to `binary`, then
  `#<id>`). Icon from the desktop entry matching `binary`/`app_name` when found; otherwise a
  generic glyph. pipewire-pulse and Flatpak clients differ only in metadata quality, never in
  control.
- Destination picker: "Follow default" + one entry per output (ports are not destinations).
  Shows **requested** (`target_name` from `Streams`) and, when it differs, **observed**
  (`StreamSinks`, §6.1): "→ Headphones · playing on Speaker".
- **Precedence**: an explicit destination set here beats notification routing for that
  stream; the daemon only re-targets `event` streams it has not seen a manual target for
  (§6.2). Documented in the Notifications section too.
- Slider/mute as in §4.1 via node `Props` (the daemon picks the path).

### 4.4 Notifications
Radio list: "Follow default output" + each output → `SetNotificationSink(name|"")`. One line
of help: which streams count (`media.role` event/Notification, plus `notification_apps`).

### 4.5 Equalizer
Per output row (or a dedicated section listing outputs — implementer's call, one of the two):
"Off" + each `EqPresets` entry → `SetEq(node, id|"")`. Hidden when `EqPresets` is empty, with
one line telling the user where presets live and the import command. Applies to real sinks
only; the daemon rejects the rest with `InvalidArgument`, which the UI shows (§5).

### 4.6 Safety
- **Port-switch cap**: a spin box 0–150 % / Off bound to `PortSwitchCap` / `SetPortSwitchCap`
  (D-Bus surface added in 1.3.0 — confirm the exact signature from the XML).
- **Allow up to 150 %**: UI-only toggle (QSettings `ui/allowOverdrive`) that widens sliders
  from 100 % to 150 % (linear 3.375). It enforces nothing in the daemon.

### 4.7 Window placement (documented, not installed)
README snippet for `~/.config/hypr/` (Omarchy 4.5 uses Lua bindings; the README gives both
the Lua `o.bind` form for a keybind such as `SUPER + CTRL + SHIFT + A` → `pipedeck-ui`, and
the window rules `float`, `size 420 720`, `move 100%-w-12 40`, `pin` as optional lines). The
app never writes these.

## 5. Client lifecycle contract
- All D-Bus calls **asynchronous** (`QDBusPendingCallWatcher`); errors surface in a status
  line at the bottom of the window with the daemon's error name and message, and clear on
  the next successful call. No modal dialogs for errors.
- On start: `GetAll` snapshot, then `PropertiesChanged` and `Changed` keep the model current;
  on `Changed` the client re-reads only the properties it uses (coalesced to ≤10/s).
- Daemon absent: show "PipeDeck daemon not running" + a **Start** button that calls
  `org.freedesktop.DBus.StartServiceByName` (D-Bus activation, §7). On `NameOwnerChanged` →
  lost: grey the window, drop every pending write; → gained: full snapshot again. **Never
  replay queued slider writes across a reconnect** — node ids are new.
- A pending write for a node that vanished is discarded, with no error shown.
- Older daemon: a missing property or method (`UnknownProperty`/`UnknownMethod`) hides that
  control instead of failing the window; `Version` is shown in Status.
- Sliders: while dragging, show the local value; on release and on every observed change,
  reconcile to the daemon's value (last observed wins after 300 ms of no local input).

## 6. Daemon changes (additive; existing signatures unchanged)
1. **`StreamSinks a(us)`** — `(stream id, node.name of the sink it is currently linked to,
   or "")`, derived from `Link` globals (output port's node → sink node), coalesced like the
   rest. This is the *observed* destination; `Streams.target_name` stays the *requested* one.
   CLI: `pipedeck status` shows `→ requested` and `on: observed` when they differ.
2. **Manual target precedence**: the daemon remembers stream ids it received a
   `SetStreamTarget` for in this session and skips them when (re)applying notification
   routing. Unit test: an `event` stream with a manual target is not re-targeted by
   `SetNotificationSink`.
3. **Arch packaging paths**: `packaging/pipedeckd.service` and the D-Bus activation file must
   not assume `%h/.local/bin` — add `packaging/arch/` variants with `/usr/bin/pipedeckd`, or
   template the path in the PKGBUILD.
Everything else the UI needs exists in 1.3.0 (`Devices`, `Streams`, `Ports`, `Eq`,
`EqPresets`, `AutoMute`, `NotificationSink`, `PortSwitchCap`, `Version`, the setters).

## 7. Packaging (Arch / Omarchy)
- `ui/pkgbuild/PKGBUILD` builds **one package `pipedeck`** from the repo (release tarball
  once tagged): cargo-builds `pipedeckd` + `pipedeck`, qmake-builds `pipedeck-ui`, installs
  the systemd user unit and `dev.pipedeck.Daemon.service` D-Bus activation file with
  `/usr/bin` paths, `pipedeck-ui.desktop`, `pipedeck-ui.svg`, the bundled presets to
  `/usr/share/pipedeck/presets/` (the daemon still reads `~/.config/pipedeck/eq/`; the
  installer copies them there on first run as `install.sh` does today). `depends`: `pipewire`,
  `wireplumber`, `alsa-lib`, `qt6-base`, `qt6-declarative`, `qt6-svg`, `qt6-wayland`;
  `makedepends`: `cargo`, `clang`, `pkgconf`, `qt6-tools` (for `qdbusxml2cpp`? — it ships in
  `qt6-base` on Arch; verify at build time).
- `ui/bin/install` = `makepkg -fsi` so the app lands in the launcher. The user unit is
  enabled by a post-install note, not by the package (user services are per user).
- `install.sh` / `Makefile` at the root keep working on Ubuntu/GNOME; they do not build `ui/`.
- Publishing to `omacom/omarchy-pkgs` is **out of scope** until Jan says so (outward-facing).

## 8. Build and test
- `ui/bin/build` → `build/pipedeck-ui`; `ui/bin/test` → Qt Test offscreen
  (`QT_QPA_PLATFORM=offscreen`), compiling `../src/*.cpp` directly, with a **fake daemon**:
  a small in-process `QDBusAbstractAdaptor` serving `dev.pipedeck.Daemon1` on the test's
  private bus (`dbus-run-session`), so model/coalescing/reconnect logic is tested without
  PipeWire. Theme/Translator tests take temp directories as the guide prescribes.
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
- Keybind: a new chord (`SUPER+CTRL+SHIFT+A`?) or take over `SUPER+CTRL+A` from the stock panel.
- One package `pipedeck` (daemon + CLI + UI) or split `pipedeck-ui` — §7 assumes one.
- Later, as agreed on 2026-10-26: a review session of the Arch/Linux audio chain itself
  (PipeWire → WirePlumber → UCM/ALSA on the ALC257, Omarchy's tuning host, where EQ and
  volume should live). §0's measurements are its first data points.
