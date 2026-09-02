# PipeDeck — SPEC v1.0 (2026-09-01)

SoundSource-style audio control for PipeWire desktops, built in-house on platform libraries only.
Target host: **chronos** (Ubuntu 26.04.1, GNOME Shell 50.1 on Wayland, PipeWire 1.6.2,
WirePlumber 0.5.13). No dependency on EasyEffects, pavucontrol, pwvucontrol or any other
end-user project. Libraries (libpipewire, GTK, GJS/Shell APIs, Rust crates) are fine.

## 1. What it does

A menu-bar (GNOME Quick Settings) panel that gives, in one place:

1. **Output device** picker — which sink is the default.
2. **Input device** picker — which source is the default.
3. **Notification device** picker — which sink *event/notification sounds* go to, independent
   of the default output. (GNOME has no such concept; this is the feature gap we close.)
4. **Per-application volume** sliders with mute, for every playback stream.
5. (v1.1) **EQ preset** picker per output device, powered by PipeWire's built-in filter-chain.

Everything the panel shows is served by a small daemon that owns all PipeWire interaction.
The panel is a thin D-Bus client, so a KDE/Sway/CLI front end later is just another client.

## 2. Architecture

```
 GNOME Shell extension (GJS)  ──D-Bus session bus──▶  pipedeckd (Rust)  ──libpipewire──▶ PipeWire
 pipedeck CLI (Rust, same crate)  ─────┘                     │
                                                             └── ~/.config/pipedeck/config.toml
```

### 2.1 `pipedeckd` — the daemon (Rust)

- Crate layout: one Cargo workspace, `crates/pipedeckd` (binary + lib), `crates/pipedeck-cli`.
- Libraries: `pipewire` (pipewire-rs, libpipewire bindings) for the graph, `zbus` for D-Bus,
  `tokio` for the async side, `serde`+`toml` for config, `tracing` for logs.
- Threading: libpipewire's `MainLoop` runs on its own thread. The D-Bus/tokio side talks to it
  via channels; the PW thread pushes a snapshot of graph state (devices, streams) into a shared
  `Arc<RwLock<State>>` and notifies D-Bus to emit `Changed`. Never call libpipewire from the
  tokio thread.
- Runs as a **systemd user service** `pipedeckd.service` (`WantedBy=default.target`,
  `After=pipewire.service wireplumber.service`). Single instance via D-Bus name ownership.

#### Graph model
- **Device** = a PipeWire node with `media.class` `Audio/Sink` or `Audio/Source`
  (ignore `Audio/Sink/Virtual`? — no: include virtual/null sinks, flag `virtual: true`; they are
  how EQ chains and the notification sink appear). Fields: `id` (u32 node id), `name`
  (`node.name`, stable across sessions), `description` (`node.description`, human label),
  `kind` (`sink`|`source`), `volume` (linear `channelVolumes`, 0.0–3.375; the UI, CLI and wpctl show the cube root, so 3.375 = 150 %),
  `mute`, `is_default`, `virtual`.
- **Stream** = node with `media.class` `Stream/Output/Audio` (playback) — v1 shows playback only,
  capture streams are listed but not rendered. Fields: `id`, `app_name` (`application.name`),
  `binary` (`application.process.binary`), `media_name` (`media.name`), `role` (`media.role`),
  `target` (node name it is routed to, from metadata `target.object` or its current link),
  `volume`, `mute`.
- Defaults: read/write WirePlumber **metadata object `default`**:
  `default.configured.audio.sink` / `default.configured.audio.source` with value
  `{"name":"<node.name>"}` — exactly what `wpctl set-default` writes. Read the *effective* default
  from `default.audio.sink` / `default.audio.source`.
- Volume: set node `Props` param (`channelVolumes`, all channels equal; keep `mute` separate).
  Read from the node's `Props` param on `param` events. Streams same.

#### Notification routing (the new thing)
- Config key `notification_sink = "<node.name>"` (empty = follow default output).
- A stream counts as a **notification stream** when `media.role` ∈ {`event`, `Notification`}
  (libcanberra/GNOME set `event`; some apps set `Notification`) — *or* `application.name` is in
  `notification_apps` (config list, default `[]`).
- On every new stream that matches, and on every change of `notification_sink`, set metadata
  `target.object` (subject = stream id, value = target node **serial** as string — WirePlumber
  0.5 resolves either `object.serial` or `node.name`; use `node.name`, it survives reconnects) so
  WirePlumber links it there. If the target sink is absent, leave the stream alone (falls back to
  default) and re-apply when the sink appears.
- Optional convenience: `create_notification_sink = true` creates a null sink named
  `pipedeck.notifications` via a PipeWire `adapter` node (`support.null-audio-sink`) — v1.1.

#### Config `~/.config/pipedeck/config.toml`
```toml
notification_sink = ""          # node.name; "" = follow default output
notification_apps = []          # extra application.name values treated as notifications
[eq]                            # v1.1
```
Written atomically (tmp + rename). Never store node ids (they change every session).

### 2.2 D-Bus interface

Well-known name `dev.pipedeck.Daemon`, object `/dev/pipedeck/Daemon`, interface
`dev.pipedeck.Daemon1`, session bus.

```
Properties (read-only, emit PropertiesChanged):
  Devices        a(usssbbdb)   (id, name, description, kind, is_default, virtual, volume, mute)
  Streams        a(ussssdb)    (id, app_name, binary, media_name, target_name, volume, mute)
  NotificationSink  s          node.name or ""
  Version        s

Methods:
  SetDefault(kind s, name s)            kind = "sink"|"source"
  SetNotificationSink(name s)           "" = follow default
  SetVolume(id u, volume d)             device or stream, 0.0–3.375 linear (150 % cubic)
  SetMute(id u, mute b)
  SetStreamTarget(id u, name s)         "" = default (kept in the API even though the panel
                                        doesn't expose it in v1 — it's free)
  Refresh()
Signals:
  Changed()      cheap "re-read the properties" nudge, coalesced to ≤10/s
```
Errors as `dev.pipedeck.Error.NotFound` / `.InvalidArgument` / `.PipeWire`.

### 2.3 `pipedeck` CLI
Thin client for scripting and testing without the Shell: `pipedeck status`, `pipedeck outputs`,
`pipedeck set-output <name>`, `set-input`, `set-notify <name|none>`, `vol <id> <0-150>`,
`mute <id> [on|off]`, `watch` (prints on every Changed). Same crate, talks D-Bus only.

### 2.4 GNOME Shell extension (`extension/`)
- UUID `pipedeck@jvandenbos.github.io`, `shell-version: ["50"]`, ESM extension (GNOME 45+ style,
  `extension.js` exporting a class extending `Extension`).
- Adds one **QuickSettings item** (`QuickMenuToggle`-style, spans 2 columns) titled "Audio" with a
  menu containing three sections — **Output**, **Input**, **Notifications** — each a radio list
  of devices (description text, checkmark on the active one), then a separator and **per-app
  sliders** (app icon if resolvable via `Gio.DesktopAppInfo`, name, slider, mute button).
- Talks only to `dev.pipedeck.Daemon1` via `Gio.DBusProxy`; if the daemon isn't running the
  item shows "PipeDeck daemon not running" and tries `Gio.DBus.session` activation once.
- Volume slider ↔ daemon: slider position is **cubic** (`pos = vol^(1/3)`, `vol = pos^3`) to match
  GNOME's own slider feel. Debounce slider→SetVolume to ≤20/s.
- Must pass `gjs -m` syntax check in the dev container, and must **not** crash the Shell if the
  daemon vanishes (wrap every proxy call, log with `console.error`).
- Optional: hide GNOME's built-in output/input pickers? **No** — leave them; duplication is fine
  in v1, Jan decides after using it.

### 2.5 v1.1 — EQ (spec'd, not built in v1)
- Per output device, a preset = parametric bands. Realised as a PipeWire **filter-chain** module
  (`libpipewire-module-filter-chain`, builtin `bq_peaking`/`bq_lowshelf`/`bq_highshelf` nodes)
  loaded by the daemon (`pw_context_load_module`) as a virtual sink `pipedeck.eq.<device>` that
  targets the real sink; applying a preset = reload module with a new graph; selecting it in the
  panel = set default to the EQ sink. Presets in `~/.config/pipedeck/eq/*.toml`; importer for
  AutoEq "ParametricEQ.txt" (plain text, not a dependency).

## 3. Non-goals (v1)
- Per-app *output routing* UI (API has it, panel doesn't).
- Capture-stream volumes, Bluetooth codec/profile switching, card profile switching.
- Anything KDE/other desktops (daemon is portable; only the panel is GNOME).
- Sound effects beyond EQ (compressor, loudness, crossfeed).

## 4. Constraints & rules
- **Platform-library-only** policy: libpipewire, WirePlumber metadata API, GNOME Shell/GJS/St,
  GTK4 (if a prefs window ever appears), Rust crates from crates.io. No calling out to `wpctl`,
  `pactl`, `pw-cli` from the daemon (the CLI test harness may use them to cross-check).
- **Nothing in the daemon may require root.** User service, user config, user D-Bus.
- Build/test targets: (a) the `pipedeck-dev` Docker image (Ubuntu 26.04, compile + unit tests,
  no PipeWire runtime), (b) chronos for integration — **only the main Claude session touches
  chronos**; subagents never SSH into fleet hosts.
- Coding standards: `cargo fmt`, `cargo clippy -- -D warnings` clean, unit tests for all pure
  logic (metadata JSON parse/format, notification-stream matching, config round-trip, volume
  cubic/linear conversion, D-Bus tuple mapping). Extension: `gjs -m` parses, no `imports.*`
  legacy API.

## 5. Acceptance (v1)
1. `systemctl --user start pipedeckd` → `pipedeck status` lists chronos's real sinks/sources
   (Starship/Matisse analog, TU104 HDMI) and running streams with correct defaults.
2. `pipedeck set-output <hdmi name>` flips the default and GNOME's own slider follows.
3. `pipedeck set-notify <analog name>` then trigger a GNOME event sound → it plays on the analog
   output while music keeps playing on HDMI. (Test: `canberra-gtk-play -i bell` or
   `gdbus call --session --dest org.freedesktop.Notifications …`.)
4. Extension shows the three pickers + per-app sliders in Quick Settings; switching in the panel
   changes the daemon state and vice-versa within 200 ms.
5. Killing the daemon leaves the Shell healthy; restarting it re-applies notification routing.

## 6. v1.0.1 addendum — ports (routes) and hardware volume (2026-09-01, from live chronos)

**Finding:** chronos's motherboard codec (ALC892) exposes ONE sink
`alsa_output.pci-0000_28_00.4.analog-stereo` with two output **ports**: `analog-output-lineout`
"Line Out" (speakers) and `analog-output-headphones` "Headphones". Switching between them is a
**device `Route` param** change on the ALSA card (`Audio/Device` object, id 53 today), not a
different sink. Also: **on ALSA-backed sinks WirePlumber owns volume via the Route param** —
writing the node's `Props` (v0.1) is ignored (`wpctl get-volume` stays put). Streams and virtual
sinks still take node `Props`. Live data (`pw-dump 53`):
```
EnumRoute: index 3 Output analog-output-lineout "Line Out" priority 9000 available yes devices [4,5,…]
           index 4 Output analog-output-headphones "Headphones" priority 9900 available yes devices [4]
           index 0 Input  analog-input-front-mic "Front Microphone" … devices [0]   (+ rear-mic, linein: available no)
Route (active): index 4 device 4 props {mute:false, channelVolumes:[0.064,0.064]}   ← 40 % cubic
                index 0 device 0 …
```
The sink node carries `device.id = "53"` and `card.profile.device = "4"`; a route applies to a node
when `route.devices` contains the node's `card.profile.device` and `route.direction` matches
(Output ↔ sink, Input ↔ source).

### 6.1 Daemon changes
- Track `Audio/Device` globals: bind `Device`, `subscribe_params([EnumRoute, Route])`, keep per
  device the route list + the active route per profile-device index. Link nodes → device by
  `device.id`/`card.profile.device`.
- **Volume/mute on a node that has a device route** → set the device `Route` param:
  `Object(Route){ index: <active route index>, device: <card.profile.device>,
  props: Object(Props){ channelVolumes: [v; channels], mute }, save: true }`. Nodes without a
  route (streams, null/virtual sinks, EQ sinks) keep the v0.1 node-`Props` path. Read volume back
  from the Route's `props` for routed nodes (that is what wpctl shows).
- **SetPort(node_id u, route_index u)** → `Object(Route){ index, device: <card.profile.device>,
  save: true }` on the device (no props). Reject unknown/unavailable/mismatched-direction routes
  with `InvalidArgument`.
- New property **`Ports a(uussbb)`** = `(node_id, route_index, name, description, available, active)`,
  one row per applicable route per node (only nodes that have a device; include `available: false`
  rows — the panel hides them, the CLI shows them dimmed). Emits `PropertiesChanged` + `Changed`
  like the others.
- Existing `Devices`/`Streams` signatures and all v1 methods are **unchanged**.
- CLI: `pipedeck ports`, `pipedeck set-port <node id> <route name|index>`; `outputs` shows the
  active port in brackets after the description when the node has >1 port.

### 6.2 Extension changes
- Output and Input sections render **one row per selectable port**: for a node with ≥2
  `available` ports, rows are `"<port description> · <node description>"` (e.g.
  "Headphones · Starship/Matisse HD Audio Controller"), each checked only when the node is default
  AND that port is active; nodes with ≤1 available port render as today (node description).
- Selecting a port row: `SetDefault(kind, name)` if not default, then `SetPort(id, index)` if not
  active. Selecting a plain node row: `SetDefault` only.
- Notifications section stays sink-level (ports are a property of the sink, and this card can
  drive only one port at a time).
- `dbus.js` gains `Ports` + `SetPort` exactly as in §6.1.

### 6.3 Acceptance additions
6. `pipedeck set-port 39 analog-output-lineout` moves sound from headphones to speakers and
   `pactl list sinks | grep "Active Port"` follows; back again with `analog-output-headphones`.
7. `pipedeck vol 39 45` changes `wpctl get-volume 39` to 0.45 on the real card (was ignored in v0.1).
8. Panel shows "Headphones · …", "Line Out · …", "Dell AW3423DW (HDMI)" as three output rows.
