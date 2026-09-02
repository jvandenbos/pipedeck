# PipeDeck — PLAN

Status legend: ☐ todo · ◐ in progress · ☑ done · ✗ blocked

## Phase 1 — daemon + CLI (Rust)  — agent: Opus — **code complete 2026-09-01, untested on a live graph**
- ☑ Workspace skeleton: `Cargo.toml` workspace, `crates/pipedeckd` (lib + bin), `crates/pipedeck-cli`.
- ☑ PipeWire thread: `MainLoop` + `Context` + `Core` + `Registry`; track nodes (media.class
  sink/source/stream), their props, `Props` params (volume/mute), and the `default` metadata
  object. Publish snapshots into `Arc<RwLock<State>>` + notify channel.
- ☑ Commands PW-thread-side: set default (metadata write), set volume/mute (Props param via
  `pw_node_set_param`), set stream target (metadata `target.object`).
- ☑ Notification routing per SPEC §2.1 (matching rule unit-tested; re-apply on new stream, on
  sink appear, on config change).
- ☑ Config load/save (`~/.config/pipedeck/config.toml`, atomic write, round-trip test).
- ☑ zbus service `dev.pipedeck.Daemon1` per SPEC §2.2 (properties + PropertiesChanged + Changed
  signal coalesced ≤10/s).
- ☑ CLI per SPEC §2.3.
- ☑ `cargo test` (52 tests), `clippy --all-targets -D warnings`, `fmt --check` clean in the dev image.

## Phase 2 — GNOME Shell extension  — agent: Sonnet
- ☑ `extension/metadata.json` (uuid `pipedeck@jvandenbos.github.io`, shell-version ["50"]),
  `extension.js`, `stylesheet.css`, optional `prefs.js` (none needed for v1).
- ☑ D-Bus proxy to `dev.pipedeck.Daemon1` (introspection XML embedded in the extension so it
  works before the daemon starts), properties cached, `Changed` → refresh.
- ☑ Quick Settings item: Output / Input / Notifications radio sections + per-app sliders (cubic).
- ☑ Robust to daemon absence/restart; `gjs -m` parses.

## Phase 3 — packaging  — agent: Haiku
- ☑ `packaging/pipedeckd.service` (systemd user unit), `install.sh` (builds with cargo, installs
  binaries to `~/.local/bin`, unit to `~/.config/systemd/user`, extension to
  `~/.local/share/gnome-shell/extensions/<uuid>`, enables extension via gsettings if absent),
  `uninstall.sh`, `Makefile` wrapping the docker commands from CLAUDE.md, `README.md`, `.gitignore`.

## Phase 4 — integration on chronos  — main session only — **done 2026-09-01 (virtual sinks)**
- ☑ rsync repo → chronos, `./install.sh` (release build 41 s), daemon live under the systemd user unit.
- ☑ Acceptance SPEC §5 1/2/3/5 passed against two `support.null-audio-sink` nodes (hardware is
  ACL'd to gdm-greeter until Jan logs in at the desk): set-output flips the WirePlumber default;
  set-notify routes a `media.role=event` stream to the notification sink while a Music stream
  stays on the default; volume/mute agree with `wpctl get-volume` (percent = cubic scale, fixed
  post-agent: MAX_VOLUME is now 1.5³ = 3.375 linear); daemon restart re-applies routing from
  config; unplug → stream falls back, replug → re-routed. Zero warnings in the journal.
- ☐ §5.4 (extension UI) — needs Jan logged into GNOME on chronos. Extension installed + enabled.
- Gotcha: janv has `Linger=no`, so every ssh session that ends tears down the user manager
  (PipeWire, WirePlumber, pipedeckd all restart with the next login). Test inside ONE ssh session.
- Gotcha: `pactl load-module module-null-sink` sinks die with the pactl session; use
  `pw-cli create-node adapter { factory.name=support.null-audio-sink … object.linger=true }`.

## Phase 5 — ports (routes) UI, SPEC §6  — agent: Sonnet (extension) + daemon agent (crates/)
- ☑ Extension side complete against SPEC §6.1/§6.2 (see Handoffs below).
- ☑ Daemon side complete: `Audio/Device` tracking, `EnumRoute`/`Route` parsing, device-`Route`
  volume/mute, `Ports` property, `SetPort` method, `pipedeck ports` / `set-port`, `nick` on
  `Devices`. 76 unit tests; build/test/clippy/fmt clean in `pipedeck-dev`.
- ☐ Live verification on chronos once both sides are deployed together (see Handoffs).

## Phase 5 — ports/routes + hardware volume (SPEC §6)  — **LIVE on chronos 2026-09-01 late evening**
- ☑ Daemon: Audio/Device tracking, Route-param volume/mute for ALSA nodes, `SetPort`, `Ports`
  property, `nick` on Devices; read-back via device `info`-driven re-enumeration (device
  `subscribe_params` never delivers change events on PipeWire 1.6.2 — pipewire-pulse's pattern).
- ☑ Extension v2: port rows "Headphones · ALC892 Analog" / "Line Out · ALC892 Analog", nick labels,
  22em menu. Installed; **needs Jan's logout/login to load** (Wayland can't hot-reload extensions).
- ☑ Live acceptance §6.3 6+7 on the real ALC892: set-port both ways (pactl Active Port follows),
  vol 45 → wpctl 0.45, external wpctl change → pipedeck follows, mute round-trip, no warnings.
- Note: HDMI sink (Dell AW3423DW) vanished from PipeWire mid-session — its route reports
  `available: no`, WirePlumber removed the node. Card/monitor side, not PipeDeck.

## Phase 6 — EQ presets, SPEC §7  — agent: Sonnet (extension) + daemon agent (crates/), concurrent 2026-09-02
- ☑ Extension side complete against SPEC §7.3/§7.4 (see Handoffs below).
- ☑ Daemon side complete against SPEC §7.1/§7.2/§7.3 (see Handoffs below): `crates/pipedeckd/src/eq.rs`
  (filter-chain args, preset files, control mapping, AutoEq importer), filter-chain module
  load/unload + `Props` control writes + `filters`-metadata bypass in `pw.rs`, `EqPresets`/`Eq`/
  `SetEq` in `service.rs`, `pipedeck eq {list,show,set,import}` in the CLI. 111 tests, all four
  container checks clean.
- ☑ **Diff owed to the extension agent is done and CLEAN** — the regenerated
  `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml` matches `extension/dbus.js`'s hand-written block
  exactly (`EqPresets a(ss)`, `Eq a(us)`, `SetEq(node_id u, preset s)`), argument names included.
  No changes needed on the extension side.
- ☐ Live verification on chronos once both sides are deployed together (see Handoffs).

## v1.1 (later)
- ☑ EQ via filter-chain (SPEC §7) — both sides code-complete, untested on a live graph (Phase 6).
- ☐ Optional null sink `pipedeck.notifications`.

## Handoffs / notes between agents

**Phase 5 (extension side) completed (2026-09-01):** SPEC §6.2 implemented in `extension/`, plus
two live-testing fixes folded in mid-task from the main session's chronos run against v0.1
(v0.1 itself was verified working — Audio item + subtitle + Output/Input/Notifications sections
rendered correctly).

- `extension/dbus.js`: added property `Ports` (`a(uussbb)` = node_id, route_index, name,
  description, available, active) and method `SetPort(node_id u, route_index u)` to the
  introspection XML. Also widened `Devices` from `a(usssbbdb)` to `a(usssbbdbs)` — a 9th trailing
  `nick` field, added per the main session's live-testing note (see below), came in *after* the
  v0.1 property signature was already fixed, not from SPEC §6.1 itself.
- `extension/extension.js`:
  - `unpackDevice` destructures the new `nick` field positionally; an 8-field tuple (older daemon)
    or an empty string both fall back to `.description` (`nick: nick || description`). Every
    device *label* (`_buildDeviceItem`, `_buildPortItem`, `_describeOutput` for the toggle
    subtitle) now reads `.nick`, never `.description`, directly. Notifications section is the one
    deliberate exception (SPEC §6.2: "stays sink-level"; kept `sink.description` there,
    unchanged) — worth a second look if Jan wants nick consistency there too.
  - `unpackPort` + `groupPortsByNode(ports)` (Map<nodeId, port[]>; `ports === null` → empty Map).
  - `_rebuildDeviceSection` groups by node id, filters `available === true`; ≥2 available ports
    renders one row per port (`"<port description> · <device nick>"`, checked when
    `device.isDefault && port.active`); ≤1 renders the v0.1 single row. Both paths coexist inside
    the same loop, so Output/Input sections can mix multi-port and single-port devices.
  - `_activatePort(device, port)`: `await setDefault` (only if not already default), then
    `await setPort` (only if not already active), one try/catch around both, `console.error` on
    failure — matches SPEC §6.2's activation rule exactly.
  - Degrade path: `_queueRebuild` reads `this._proxy.Ports` and maps it to `null` (not `[]`) when
    the property is absent from the proxy's cached properties (v0.1 daemon) — `groupPortsByNode`
    treats `null` the same as "no ports for any device", so every device renders exactly as v0.1
    did, no special-casing needed elsewhere.
  - `_callRemote` now returns a Promise (still logs every failure via `console.error` itself, same
    as before) so `_activatePort` can sequence two calls. Every other, still-fire-and-forget call
    site (`setDefault`/`setNotificationSink`/`setVolume`/`setMute`) got `.catch(() => {})` appended
    so GJS doesn't log an "Unhandled promise rejection" on top of the error `_callRemote` already
    printed.
  - `Ports` refresh: confirmed `_queueRebuild` (triggered by both `g-properties-changed` and
    `Changed`, unconditionally, same as v0.1) reads `Ports` fresh every time — no extra wiring
    needed.
- `extension/stylesheet.css`: `.pipedeck-menu { min-width: 22em; }` (applied via
  `this.menu.box.add_style_class_name('pipedeck-menu')` in `PipeDeckToggle._init`) plus
  `.pipedeck-device-label { text-overflow: ellipsis; }` (applied to `item.label` in
  `_buildDeviceItem`/`_buildPortItem`) so labels like "Headphones · ALC892 Analog" ellipsize at
  the end instead of clipping mid-word in the default-width popup menu. Per-app rows already had
  `min-width`/`max-width`/`text-overflow: ellipsis` on `.pipedeck-app-label` and `min-width` on
  `.pipedeck-app-slider` from Phase 2, so the wider menu just gives them more breathing room —
  nothing else needed changing there.
- `extension/metadata.json`: `"version": 2` added (no prior `version` key existed, only
  `version-name`).
- Verified: `python3 -m json.tool extension/metadata.json` clean; in `pipedeck-dev`, both
  `dbus.js` and `extension.js` `gjs -c "import(...)"` produce no syntax errors, and a
  `gjs -m` dynamic-import probe (written to a temp file, since `gjs -m -c "..."` itself fails to
  resolve the `<command line>` pseudo-path as a module in this image — that's a harness quirk, not
  a signal about the code) confirms `extension.js`'s only failure is
  `Unable to load file from: resource:///org/gnome/shell/extensions/extension.js` — i.e. missing
  Shell runtime, not a `SyntaxError`; `dbus.js` imports clean standalone (prints `dbus ok`).
- **`crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml` exists but is still the pre-§6.1 v0.1 shape**
  (`Devices` = `a(usssbbdb)`, no `Ports`, no `SetPort`, mtime predates this session's edits) — the
  daemon agent's §6.1/nick work had not landed as of this handoff. **Next agent/session: diff this
  file against `extension/dbus.js`'s `DaemonInterfaceXml` before shipping** — specifically confirm
  the `Ports` tuple field order (`node_id, route_index, name, description, available, active`),
  the `SetPort` arg names/order (`node_id`, `route_index`), and that `Devices` really does end in
  a 9th `s` field, at the position this extension assumes (immediately after `mute`).
- **What can only be checked live on chronos (needs the daemon's §6.1 side deployed too):**
  1. A real multi-port node (chronos's ALC892 "Line Out"/"Headphones") actually renders as two
     rows and single-port nodes (Dell AW3423DW HDMI) still render as one — the ≥2/≤1
     `available`-count branch was only exercised by reading code, never against real `Ports` data.
  2. Port-row activation end-to-end: clicking "Headphones · ALC892 Analog" while "Line Out" is
     active calls `SetDefault` (if needed) then `SetPort`, and `pactl list sinks | grep "Active
     Port"` follows, per SPEC §6.3 item 6.
  3. Toggle subtitle showing `"<active port> · <nick>"` for a live default output with ≥2 ports.
  4. The `nick` field's actual live values/formatting ("ALC892 Analog", "Dell AW3423DW") in
     context — confirm they read well combined with port descriptions and don't duplicate info
     already in the port description.
  5. Menu width/ellipsis: whether `min-width: 22em` and `text-overflow: ellipsis` on
     `.pipedeck-device-label` actually fix the clipping the main session photographed, and whether
     22em is the right number once real (possibly longer) `nick`/port-description strings are on
     screen together.
  6. Whether `this.menu.box.add_style_class_name(...)` is even the right actor to style for
     `QuickMenuToggle`'s menu in Shell 50 — confirmed `this.menu.box` exists and is the item
     container by the same reasoning as Phase 2's other menu-API assumptions (read from GNOME
     Shell 50 source, never rendered), not by opening the menu.
  7. Whether a v0.1-vs-v0.1.1(ports) daemon *version skew* during a live upgrade (extension
     reloaded before/after the daemon restarts) ever shows a half-migrated state — e.g. `Ports`
     present but `Devices` still 8-field, or vice versa. Both fields are read defensively
     (`nick || description`, `Ports ?? null`) so this should degrade rather than throw, but it was
     never forced.

**Phase 3 completed (2026-09-01):**
- `packaging/pipedeckd.service` — systemd USER unit, installs to `~/.config/systemd/user/pipedeckd.service`
- `packaging/dev.pipedeck.Daemon.service` — D-Bus activation file, installs to `~/.local/share/dbus-1/services/dev.pipedeck.Daemon.service`
- `install.sh` — builds workspace, installs binaries to `~/.local/bin/`, systemd unit, D-Bus service, extension (if present)
  - Flags: `--no-extension`, `--no-service` (idempotent, safe to re-run)
  - Falls back to `gsettings` for extension enable if `gnome-extensions` unavailable
  - Prints summary with status commands and paths
- `uninstall.sh` — stops/disables service, removes files, optionally purges `~/.config/pipedeck` with `--purge`
- `Makefile` — `build`, `test`, `clippy`, `fmt`, `check` (all four), `ext-check`, `install`/`uninstall` (Linux guard with uname)
  - All cargo targets run in `pipedeck-dev` Docker container (as specified in CLAUDE.md)
- `README.md` — features, architecture, requirements (Ubuntu 26.04 / GNOME 50 / PipeWire 1.0+ / WirePlumber 0.5+), install/uninstall, CLI/GUI usage, config format, dev (make targets), D-Bus interface, troubleshooting, license
- `LICENSE` — MIT, copyright 2026 Jan Vandenbos

Both shell scripts:
- `bash -n` syntax check: ✓
- `shellcheck` check: ✓ (SC2155 fixed)
- Made executable (755)

**From Opus (Phase 1):** expects binaries at `target/release/pipedeckd` and `target/release/pipedeck`
**From Sonnet (Phase 2):** expects `extension/` dir with `metadata.json`, `extension.js`, `stylesheet.css` (uuid `pipedeck@jvandenbos.github.io`)
**For Phase 4 (main session):** installation paths now documented in README §Installation

**Phase 2 completed (2026-09-01):**
- `extension/metadata.json` — uuid `pipedeck@jvandenbos.github.io`, name "PipeDeck",
  `shell-version: ["50"]`, url `https://github.com/jvandenbos/pipedeck`. No `settings-schema`
  (no prefs in v1, per SPEC §2.4).
- `extension/dbus.js` — `dev.pipedeck.Daemon1` introspection XML derived from SPEC §2.2, wrapped
  with `Gio.DBusProxy.makeProxyWrapper`. Exports `BUS_NAME` (`dev.pipedeck.Daemon`), `OBJECT_PATH`
  (`/dev/pipedeck/Daemon`), `DaemonProxy`. **No `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml`
  existed at the time this was built (Phase 1's workspace is still just the `hw` placeholder
  crate) — nothing to diff against yet.** When the daemon agent lands the canonical XML, diff it
  against `extension/dbus.js`'s `DaemonInterfaceXml` constant and reconcile; the property tuple
  field *order* is where this is most likely to drift silently (GVariant tuples are positional,
  not named), see field-order note below.
- `extension/extension.js` — ESM extension (GNOME 45+ style), `PipeDeckExtension extends
  Extension`. `PipeDeckIndicator` (a `SystemIndicator`) owns the D-Bus proxy lifecycle
  (`Gio.bus_watch_name` + one `StartServiceByName` activation attempt per name-vanish, proxy
  recreated on name-appear); `PipeDeckToggle` (a `QuickMenuToggle`, added via
  `addExternalIndicator(indicator, 2)` so it spans both Quick Settings columns) renders Output /
  Input / Notifications radio sections (checkmark ornament on the active device) plus per-app
  volume rows (`AppVolumeRow`: icon via best-effort `Gio.DesktopAppInfo` lookup, label, cubic
  `Slider`, mute `St.Button`). Every proxy call and D-Bus callback is try/caught with
  `console.error`; `disable()` destroys the toggle's `quickSettingsItems` then the indicator,
  whose own `'destroy'` handler unwatches the bus name and disconnects the proxy signals.
- `extension/stylesheet.css` — `.pipedeck-app-row`/`-icon`/`-label`/`-slider`, `.pipedeck-mute-button`,
  `.pipedeck-unavailable-item`.

**Assumptions the daemon (Phase 1) must honour, since the extension was built against SPEC §2.2
alone with no daemon to test against:**
- Property tuple field order exactly as documented in SPEC §2.2's parenthetical:
  `Devices a(usssbbdb)` = `(id, name, description, kind, is_default, virtual, volume, mute)`;
  `Streams a(ussssdb)` = `(id, app_name, binary, media_name, target_name, volume, mute)`. The
  extension's `unpackDevice`/`unpackStream` in `extension.js` destructure positionally — a
  reordering on the daemon side (even one matching the same type signature) will silently
  scramble the UI rather than error.
- `kind` string values on `Devices` are exactly `"sink"` / `"source"` (lowercase) — the extension
  filters on those literals and also passes `device.kind` straight back into `SetDefault(kind,
  name)`, so it never hardcodes "sink"/"source" itself but does assume the daemon's own vocabulary
  is self-consistent between what it reports and what it accepts.
- `NotificationSink == ""` means "follow default output" both ways: the extension's "Follow
  output" menu item is checked when `NotificationSink === ''` and calls
  `SetNotificationSink('')` to select it.
- Volume is linear `0.0–1.5` on the wire; the extension only ever cubic-maps
  (`pos = vol^(1/3)` for display, `vol = pos^3` on send) and relies on the daemon to clamp —
  it does not clamp outgoing values itself, per SPEC §2.2's "0.0–1.5 linear" contract note on
  `SetVolume`.
- `Changed` is a no-argument signal and property values are read fresh from the proxy's cached
  properties (`g-properties-changed` also triggers the same rebuild) — the extension never reads
  signal args, so if the daemon ever wants to pass data on `Changed` it's ignored, and the
  extension instead re-reads `Devices`/`Streams`/`NotificationSink` synchronously off the proxy
  right after either signal fires. This means the daemon must actually update its D-Bus
  properties (emit `PropertiesChanged` or otherwise ensure cached values are current) *before* or
  *atomically with* emitting `Changed` — if `Changed` fires while the properties on the bus are
  still stale, the extension will rebuild from stale data and not retry.
- Methods are called via the async `*Remote(...args, callback)` form generated by
  `makeProxyWrapper`, never `*Sync` — a daemon that's slow or wedged can delay a menu action but
  will never block the Shell's main loop.

**What could only be verified in a live GNOME session (not testable in the headless dev
container, which has no PipeWire/D-Bus/Shell runtime):**
- Whether `addExternalIndicator(indicator, 2)` actually renders the toggle spanning both grid
  columns as intended — confirmed the two-arg `colSpan` signature exists in GNOME Shell 50's
  `js/ui/panel.js` source (fetched from `gitlab.gnome.org/GNOME/gnome-shell` `gnome-50` branch),
  but never rendered.
- Whether the `QuickMenuToggle`/`QuickToggleMenu` API calls (`menu.setHeader(...)`,
  `PopupSeparatorMenuItem(text)` as a section header, `PopupMenuSection.removeAll()`,
  `PopupBaseMenuItem({activate: false})` for the slider rows) compose visually the way intended —
  each individual method was confirmed to exist with the assumed signature by reading GNOME Shell
  50's actual source, but the assembled menu was never opened.
- Actor lifecycle on `disable()`/re-`enable()`: whether destroying `AppVolumeRow`'s
  `PopupBaseMenuItem` (which cascades into its `Slider`/`St.Button` children) before or after my
  own `row.destroy()` cleanup runs ever double-disconnects a signal handler or logs a Shell
  warning — GJS/Clutter are generally tolerant of this, but it was never exercised by an actual
  enable/disable cycle. Worth a `journalctl --user -f | grep -i pipedeck` (or `-i gjs`) watch
  during the first live enable/disable.
- App icon resolution (`Gio.DesktopAppInfo.new('<binary>.desktop')` / `'<app_name>.desktop')` /
  `Gio.AppInfo.get_all()` executable-basename fallback) — reasonable heuristics, untested against
  real `application.process.binary`/`application.name` values from actual PipeWire streams.
- Slider feel: whether cubic mapping with `maximum_value = Math.cbrt(1.5) ≈ 1.145` and
  `overdrive_start = 1.0` visually matches "GNOME's own slider feel" as SPEC §2.4 asks for — GNOME's
  *own* volume mixer slider (`js/ui/status/volume.js`, confirmed by reading its source) is actually
  linear, not cubic; PipeDeck's cubic mapping is SPEC's own deliberate design choice, not a literal
  copy of GNOME's implementation, so "feel" here can only be judged by ear/eye once live.

**Install/enable for testing (once Phase 4 puts this on chronos), per `packaging/install.sh` and
the standard extension layout:**
```bash
mkdir -p ~/.local/share/gnome-shell/extensions/pipedeck@jvandenbos.github.io
cp extension/*.{js,json,css} ~/.local/share/gnome-shell/extensions/pipedeck@jvandenbos.github.io/
# Wayland: log out/in (or a nested session) to pick up a brand-new extension; X11 only: Alt+F2, r, Enter
gnome-extensions enable pipedeck@jvandenbos.github.io
journalctl --user -f -o cat /usr/bin/gnome-shell   # or: journalctl --user -f | grep -i pipedeck
```
`packaging/install.sh` already does the copy + `gnome-extensions enable` (falling back to
`gsettings` if `gnome-extensions` is unavailable) — no changes needed there for this handoff.

### Phase 1 → everyone (daemon agent, 2026-09-01)

**Binaries.** `pipedeckd` (from `crates/pipedeckd`) and `pipedeck` (from `crates/pipedeck-cli`,
package name `pipedeck-cli`). `cargo build --release` puts them at `target/release/pipedeckd` and
`target/release/pipedeck`. Both are plain binaries with no runtime data files. `pipedeckd`
understands only `--help` / `--version`; logging is `RUST_LOG` (default `info`).

**D-Bus identity** — bus name `dev.pipedeck.Daemon`, object path `/dev/pipedeck/Daemon`,
interface `dev.pipedeck.Daemon1`, session bus. The name is taken with `DoNotQueue`, so a second
instance exits with an error instead of silently waiting.

**→ extension agent: introspection XML** is at `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml`,
already wrapped in `<node>` so it can be embedded verbatim:
`Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]`. It is generated from the daemon's own
`zbus::interface` impl and a unit test (`introspection_xml_matches_the_checked_in_copy`) fails if
the two drift, so it is safe to copy.

Property signatures are exactly as SPEC §2.2 says:
`Devices a(usssbbdb)` = (id, name, description, kind, is_default, virtual, volume, mute);
`Streams a(ussssdb)` = (id, app_name, binary, media_name, target_name, volume, mute);
`NotificationSink s`; `Version s`. Methods `SetDefault(ss)`, `SetNotificationSink(s)`,
`SetVolume(ud)`, `SetMute(ub)`, `SetStreamTarget(us)`, `Refresh()`; signal `Changed()`.

Notes for the panel:
- **`volume` on the wire is LINEAR** (`channelVolumes`, 0.0–1.5). The cubic slider mapping is the
  client's job, exactly as SPEC §2.4 says: `pos = vol^(1/3)`, `vol = pos^3`.
- `Changed` is coalesced to at most one per 100 ms, and `PropertiesChanged` is emitted for
  `Devices` and `Streams` in the same tick. `NotificationSink` emits its own `PropertiesChanged`
  when `SetNotificationSink` succeeds. `Version` never changes.
- `SetNotificationSink("")` means "follow the default output".
- Errors come back as `dev.pipedeck.Error.NotFound`, `.InvalidArgument`, `.PipeWire` — wrap every
  proxy call, since `.PipeWire` is what you get if the graph link dropped.
- `kind` in `Devices` is the string `"sink"` or `"source"`.

**→ packaging agent:**
- systemd user unit should be `ExecStart=%h/.local/bin/pipedeckd`, `Type=simple`,
  `After=pipewire.service wireplumber.service`, `WantedBy=default.target`. The daemon exits
  non-zero if it cannot reach PipeWire, so `Restart=on-failure` with a `RestartSec=2` is right —
  it is the intended recovery path when it starts before WirePlumber.
- It also exits on SIGTERM and SIGINT, and shuts the PipeWire thread down cleanly on the way out.
- Config lives at `$XDG_CONFIG_HOME/pipedeck/config.toml` (fallback `~/.config/...`); the daemon
  creates it on the first `SetNotificationSink` and never needs one to start.
- **`dev/Dockerfile` is missing `rust-clippy` and `rustfmt`** — two of the four commands in
  CLAUDE.md cannot run in the image as built. I verified them in a derived image
  (`FROM pipedeck-dev` + `apt-get install -y --no-install-recommends rust-clippy rustfmt`); please
  fold those two packages into `dev/Dockerfile` so `make lint` works out of the box.

**→ main session, for integration on chronos — the things that need a live graph:**
1. **Volume/mute via node `Props`.** The daemon writes `SPA_PROP_channelVolumes` (one f32 per
   channel) and `SPA_PROP_mute` with `pw_node_set_param(SPA_PARAM_Props)`. This is what
   pulse-server does, but on ALSA sinks WirePlumber may route volume through the device Route
   param instead — verify `pipedeck vol <sink id> 50` is audible and that `wpctl get-volume`
   agrees, on both the analog and the HDMI sink.
2. **`target.object` type string.** Written as a plain `node.name` with SPA type `"Spa:String"`
   (SPEC §2.1 prefers node.name because it survives re-plugs). WirePlumber 0.5 also accepts a
   decimal `object.serial`. If notification routing does not take, try
   `pw-metadata -n default <stream id> target.object` to see what WirePlumber actually stored and
   compare with what `wpctl`/pavucontrol write — `meta::TYPE_SPA_STRING` in
   `crates/pipedeckd/src/meta.rs` is the one constant to change.
3. **Notification routing end-to-end** (SPEC §5.3): `pipedeck set-notify <analog name>`, play
   music on HDMI, then `canberra-gtk-play -i bell`.
4. **Channel count.** Node channel count is learned from the first `Props` event and defaults to
   2 until then; a mono or 5.1 sink touched before its first param event would get the wrong
   array length. Check a 5.1/HDMI sink.
5. **Stream `target_name`** is resolved from live `Link` globals first, then from the metadata
   `target.object` value (numeric serial or node name). Worth eyeballing in `pipedeck status`.

**Deliberate SPEC deviations (both small, both flagged rather than silently taken):**
- SPEC §2.1 says capture streams are "listed but not rendered". The `Streams` tuple has no field
  that distinguishes playback from capture, so a client could not tell them apart and the panel
  would render them. The `Streams` property therefore lists **playback streams only**. Capture
  streams are still tracked internally, so `SetVolume`/`SetMute` on a capture stream id works; if
  the panel ever wants them, add a separate property rather than widening this one.
- `SetNotificationSink` rejects a name that is not currently a sink, with
  `dev.pipedeck.Error.NotFound`. The absent-sink path from SPEC §2.1 is still exercised on
  hot-unplug: the config keeps the name, matching streams are left alone, and routing re-applies
  when the sink comes back.


### Phase 5 → everyone (daemon agent, 2026-09-01) — ports, hardware volume, `nick`

**What changed in `crates/` (only lane touched: `crates/**`, plus one line of SPEC §2.2 and this
file). Nothing was committed.**

- **New module `crates/pipedeckd/src/route.rs`** — the pure half of SPEC §6.1: `RouteDirection`
  (`SPA_DIRECTION_*`), `Availability` (`SPA_PARAM_AVAILABILITY_*`), `Route` (one `EnumRoute`
  entry), `RouteProps`, `ActiveRoute`, `DeviceRoutes` (route table for one card), `Port` +
  `PortTuple`, and `validate_set_port`. No PipeWire types, so all of it is unit-tested without a
  graph. `pw.rs` is still the only module linking libpipewire.
- **`pw.rs`** now binds `PipeWire:Interface:Device` globals whose `media.class` is `Audio/Device`,
  `subscribe_params([EnumRoute, Route])` + an initial `enum_params` for both, and parses both from
  the `param` listener with a single generic pod parser (`parse_route`) keyed on
  `libspa_sys::SPA_PARAM_ROUTE_*` — **libspa 0.10.1 ships no typed route helpers** (`param/` has
  only `audio`, `format`, `format_utils`, `video`), so generic `Object` iteration is the only way.
  The nested `props` sub-object is parsed by the *same* `Props` parser the node path uses
  (`parse_props` was split into `parse_props`/`parse_props_object`).
- **Node→device link** via node props `device.id` (u32) and `card.profile.device` (i32), both read
  at bind time and refreshed from `info` events.
- **Routed nodes take the device `Route` param for volume/mute** — `Object(Route){ index, device,
  props: Object(Props){ channelVolumes:[v; channels], mute }, save: true }`. `channels` is the
  length of the active route's `channelVolumes`, falling back to the node's learned channel count.
  Non-routed nodes (streams, null/virtual sinks, EQ sinks) keep the v0.1 node-`Props` path
  untouched. Read-back for routed nodes comes from the Route props, so `Devices.volume` now
  matches what `wpctl` shows.
- **`SetPort(node_id u, route_index u)`** → `Object(Route){ index, device, save: true }`, no props.
- **`Ports a(uussbb)`** = `(node_id, route_index, name, description, available, active)`, one row
  per applicable route per node, unavailable rows included, sorted by `(node_id, route_index)`.
  Emitted with `PropertiesChanged` in the same tick as `Devices`/`Streams`, then `Changed` —
  identical coalescing to the others (≤10/s).
- **`Devices` is now `a(usssbbdbs)`** — trailing `nick` (`node.nick`, falling back to
  `description`), per the main session's live-testing note. SPEC §2.2's property block was edited
  (one line) to match; every other signature is unchanged.
- **CLI**: `pipedeck ports`, `pipedeck set-port <node id> <route name|description|index>`, and
  `outputs`/`inputs`/`status` print the nick as the primary label with the active port in brackets
  when a node has more than one port, then the description and `node.name` on continuation lines.
- **Checks**: `cargo build --workspace`, `cargo test --workspace` (**76 tests**, up from 52),
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` — all clean in the
  `pipedeck-dev` image.

**→ extension agent: the D-Bus XML is regenerated** at
`crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml` (the
`introspection_xml_matches_the_checked_in_copy` test pins it to the live interface). The two new
pieces, and the widened `Devices`, are exactly what `extension/dbus.js` already assumed — diff
confirmed, no changes needed on your side:

```xml
<method name="SetPort">
  <arg name="node_id" type="u" direction="in"/>
  <arg name="route_index" type="u" direction="in"/>
</method>
<property name="Devices" type="a(usssbbdbs)" access="read"/>
<property name="Ports" type="a(uussbb)" access="read"/>
```

Full interface, in the order zbus emits it (methods in declaration order, properties alphabetical):
`SetDefault(ss)`, `SetNotificationSink(s)`, `SetVolume(ud)`, `SetMute(ub)`,
`SetStreamTarget(us)`, `SetPort(uu)`, `Refresh()`; signal `Changed()`; properties
`Devices a(usssbbdbs)`, `NotificationSink s`, `Ports a(uussbb)`, `Streams a(ussssdb)`,
`Version s` — all read-only.

Two behaviours worth knowing in the panel:
- `available` is `false` **only** when the card says `SPA_PARAM_AVAILABILITY_no`. `unknown` (what a
  codec without jack-detection reports, and what most HDMI outputs report) maps to `true`. Filtering
  on `available === true`, as `_rebuildDeviceSection` does, is therefore correct and will not hide
  HDMI.
- A node's `volume`/`mute` in `Devices` now come from its active route when it has one, so the
  slider and GNOME's own slider should finally agree on ALSA sinks.

**→ main session, live verification on chronos (exact commands).** Node ids change every session —
take them from `pipedeck outputs` first.

```bash
# 0. deploy + restart, then confirm the new surface is on the bus
./install.sh && systemctl --user restart pipedeckd && sleep 2
gdbus introspect --session --dest dev.pipedeck.Daemon --object-path /dev/pipedeck/Daemon \
  | grep -E "Ports|SetPort|Devices"

# 1. ports show up at all — expect Line Out (3) and Headphones (4) on the ALC892 sink,
#    the mic routes on the source, and HDMI's single port. `*` marks the active one.
pipedeck ports
pipedeck outputs          # nick first, "[Headphones]" bracket on the multi-port node

# 2. SPEC §6.3 acceptance 6 — port switching, both directions
pipedeck set-port <analog sink id> analog-output-lineout
pactl list sinks | grep -A1 "Active Port"        # follows to analog-output-lineout
pipedeck set-port <analog sink id> analog-output-headphones
pactl list sinks | grep -A1 "Active Port"

# 3. SPEC §6.3 acceptance 7 — hardware volume, which v0.1 silently dropped
pipedeck vol <analog sink id> 45
wpctl get-volume <analog sink id>                # expect 0.45
pipedeck vol <analog sink id> 100 && wpctl get-volume <analog sink id>
pipedeck mute <analog sink id> on  && wpctl get-volume <analog sink id>   # [MUTED]
pipedeck mute <analog sink id> off

# 4. the other direction: change it in GNOME / wpctl and watch pipedeck follow
wpctl set-volume <analog sink id> 0.30 && pipedeck outputs
wpctl set-default <hdmi sink id>       && pipedeck status

# 5. non-routed nodes must be unaffected (this is the regression risk)
pipedeck vol <a stream id> 60 && pipedeck streams
#   ... and on a null sink, if one is still around from Phase 4:
pipedeck vol <null sink id> 60 && wpctl get-volume <null sink id>

# 6. input side
pipedeck inputs
pipedeck set-port <analog source id> analog-input-rear-mic   # then check `pipedeck ports`

# 7. error paths (all should print a dev.pipedeck.Error.InvalidArgument message, not hang)
pipedeck set-port <analog sink id> 99                    # unknown route
pipedeck set-port <analog sink id> analog-input-front-mic # wrong direction
pipedeck set-port <hdmi sink id> analog-output-lineout    # different card
pipedeck set-port <a stream id> 3                         # a stream has no ports

# 8. profile change must re-enumerate, not accumulate stale rows
wpctl status | head -30       # note the card profile
#   switch the card profile in gnome-control-center Sound, then:
pipedeck ports                # row set should change, not grow

# 9. nothing noisy in the journal through all of the above
journalctl --user -u pipedeckd -f
```

**Things that could only be checked in the container, and are worth an eye on chronos:**
1. **The nested `props` object id.** `pulse-server` builds it as
   `SPA_TYPE_OBJECT_Props` with object id **`SPA_PARAM_Route`** (not `SPA_PARAM_Props`), and that
   is what `route_pod` emits. The ALSA device parses `props` with a NULL id filter, so it should
   not matter — but if hardware volume writes are accepted-and-ignored, this is the first line to
   change (`crates/pipedeckd/src/pw.rs`, `route_pod`).
2. **Re-enumeration on `index == 0`.** The route table is cleared whenever a `param` event arrives
   with `index == 0`, on the assumption that PipeWire re-emits a whole enumeration from 0. If a
   card-profile change leaves stale ports in `pipedeck ports` (step 8), that assumption is wrong
   and the fix is to key the clear off a `seq`/`info`-driven re-enum instead.
3. **`devices` array pod shape.** Parsed as `SPA_TYPE_Array` of `Int`, with `Id` arrays and a bare
   scalar tolerated. If `pipedeck ports` is empty while `pw-dump <device id>` clearly shows
   `EnumRoute` entries, dump the raw pod in `parse_route` — the array spelling is the suspect.
4. **Multi-profile-device cards.** SPEC §6's chronos card has route 3 on devices `[4,5]`; the
   active-route map is keyed by `card.profile.device`, so a card exposing two sinks off one device
   global (e.g. HDMI with several outputs) is the untested case. `pipedeck ports` listing the same
   route index under two node ids is *correct*, not a bug.
5. **Channel count on a routed node.** Taken from the active route's `channelVolumes` length, so a
   5.1 HDMI output should get 6 floats — worth confirming `pipedeck vol` on HDMI does not collapse
   it to stereo.

**Deliberate choices / deviations from §6.1, all small:**
- **`available` maps `unknown` → `true`.** SPEC §6.1 gives the field as a bool without saying what
  to do with `SPA_PARAM_AVAILABILITY_unknown`. Only an explicit `no` is reported unavailable;
  treating `unknown` as unavailable would hide most HDMI outputs and any codec without jack
  detection. `Availability::is_selectable` in `route.rs` is the single place to flip this.
- **Error kinds.** SPEC §6.1 says unknown/unavailable/mismatched-direction routes are
  `InvalidArgument`, and they are. An unknown *node id* is `NotFound` (consistent with
  `SetVolume`/`SetMute`); a node that exists but has no card route — a stream, a null sink — is
  `InvalidArgument("node N has no ports")`, since the id is real and it is the request that is
  meaningless.
- **`SetVolume` on a routed node always writes both `channelVolumes` and `mute`** (the unchanged
  one carried over from the active route's current props), matching the shape SPEC §6.1 spells
  out, rather than sending a partial props object.
- **`pipedeck set-port` resolves its argument index-first, then route name, then description**
  (both case-insensitive), so a numeric argument that is a valid index for that node always wins.
  Resolution happens client-side against the `Ports` property; the daemon validates again.
- **`status`, not just `outputs`/`inputs`, shows the active-port bracket.** They share one
  `device_line` renderer; SPEC §6.1 only asked for `outputs`, and splitting them to honour that
  literally would have been worse.


### Phase 5 fix (daemon agent, 2026-09-01) — stale read-back after v0.2 live test

**Symptom from chronos:** `set-port` and `vol` really took effect (pactl `Active Port` followed,
`wpctl get-volume` = 0.45), but `Devices`/`Ports` kept their startup values until a daemon
restart. Main session's debug lines showed the full `EnumRoute` + `Route` enumeration arriving
*twice* at startup and then no device param event ever again, while registry events kept flowing.

**Cause:** on PipeWire 1.6.2, `Device::subscribe_params` does not deliver change notifications.
`wpctl` and pipewire-pulse do not rely on it either — `module-protocol-pulse/manager.c`'s
`device_event_info` reacts to the **`info` event**, walking `info.params()` and re-enumerating the
changed READ-able ids itself.

**Fix, in `crates/pipedeckd/src/pw.rs` only:**
- `on_device_global` now registers an `.info(...)` listener alongside `.param(...)`. When
  `change_mask` contains `DeviceChangeMask::PARAMS` it walks `info.params()` and re-issues
  `enum_params(0, Some(EnumRoute|Route), 0, u32::MAX)` for the ids reported readable.
  `subscribe_params` is deliberately **kept** — a duplicate enumeration is idempotent, so a future
  server that does deliver them costs nothing.
- pipewire-rs 0.10's `ParamInfo` exposes only `id()` and `flags()`, **not** the `user`/serial
  counter pulse diffs against, so every tracked READ-able param is re-enumerated on a PARAMS-masked
  info event (the coordinator's documented fallback). An info event carrying no param list at all
  falls back to re-reading everything we track. The rule is the pure, unit-tested
  `params_to_reenumerate`.
- **Node side, checked by reading rather than assumed:** the node listener already had an `.info()`
  hook, but it only read the props *dict* — `Props` read-back rested entirely on
  `subscribe_params`, which demonstrably works for nodes (v0.1 read stream and null-sink volumes
  back live). The same PARAMS-gated re-enum was added anyway as belt-and-braces; it is free
  because a `media.name` change sets the PROPS mask, not PARAMS, so a music player's constant
  churn does not trigger it.
- `index == 0` clearing semantics are unchanged, and now do real work: each info-driven
  re-enumeration restarts at index 0 and rebuilds the table rather than appending to it.
- The main session's `debug!` lines in `on_device_param` are kept as-is; two more were added
  (`"re-enumerating device param after info"`) so the next live run shows the loop closing.

**Checks:** build / test (**77 tests**, +1 for `info_param_list_selects_what_to_reenumerate`) /
`clippy --all-targets -D warnings` / `fmt --check` all clean in `pipedeck-dev`. No new
dependencies; `Cargo.lock` untouched.

**What to re-test live:** everything in the Phase 5 list above still applies, but the three that
actually prove this fix are —
```bash
pipedeck ports                                     # note the active port
pipedeck set-port <analog sink id> analog-output-lineout
pipedeck ports                                     # `*` must MOVE, with no restart
pipedeck vol <analog sink id> 45 && pipedeck outputs   # must read back 45%, not 40%
wpctl set-volume <analog sink id> 0.30 && pipedeck outputs  # external change must land too
```
If read-back is still stale, `RUST_LOG=debug` and look for `re-enumerating device param after
info`: no such line means the device `info` event itself is not firing with the PARAMS mask, and
the next fallback is a timer-free re-enum on every info event regardless of `change_mask`.

### Phase 6 (extension side) completed (2026-09-02) — EQ presets, SPEC §7.3/§7.4

Built concurrently with the daemon agent's filter-chain work; `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml`
had no EQ members yet at the time of this pass, so `dbus.js` was written directly against SPEC
§7.3's documented wire types rather than diffed against the checked-in XML:

- `extension/dbus.js`: added property `EqPresets` (`a(ss)` = id, name), property `Eq` (`a(us)` =
  node_id, preset id or "") and method `SetEq(node_id u, preset s)` to the introspection XML,
  alongside the existing `Ports`/`SetPort` from Phase 5. Doc comment at the top of the file updated
  to list all five wire types together.
- `extension/extension.js`:
  - `unpackEqPreset([id, name])` and `unpackEq([nodeId, preset])`, matching the existing
    `unpackPort`/`unpackDevice` pattern.
  - New **Equalizer** section (`_eqHeader`/`_eqSection`) inserted after Notifications and before
    the per-app rows, added to `_sectionActors` so `setUnavailable()`/`setAvailable()` cover it
    like every other section.
  - `_rebuildEqSection(defaultOut, eqPresets, eq)`: "Off" row + one row per preset (label =
    preset `name`), ornament CHECK on whichever is active for the current default output (found
    by matching `eq[].nodeId` against `defaultOut.id`; no matching row, or `preset === ''`, means
    Off). **Hides the header+section entirely** — not a disabled/empty placeholder like the device
    sections use — when there's no default sink (`defaultOut` falsy) or `eqPresets` is
    null/empty, per SPEC §7.4's "hidden entirely" wording. This runs on every `rebuild()`, so it
    re-hides/re-shows correctly on every `Changed`/property-change tick, not just once at startup.
  - Activation calls `SetEq(defaultOut.id, presetId)` (`''` for Off) via a new `setEq` proxyOp →
    `SetEqRemote`, try/catch + `console.error` around the call site plus the existing
    `_callRemote` logging (same double-layer as `_activatePort`).
  - `_queueRebuild()` unpacks `EqPresets`/`Eq` in the same pass as `Devices`/`Streams`/`Ports`,
    normalizing an absent property (older daemon, property genuinely not in the introspected
    interface) to `null` — same convention as `Ports`, and the reason `_rebuildEqSection` treats
    `null` and `[]` identically (both hide the section) while still distinguishing "no daemon
    support" from "daemon supports EQ, zero presets configured" is irrelevant to the UI: both look
    like "nothing to show" from the panel's point of view.
- `extension/metadata.json`: `version` 2 → 3.
- No `stylesheet.css` changes — the Equalizer rows reuse plain `PopupMenu.PopupMenuItem`, no new
  style classes needed.

**Verified in this pass (dev container, no live daemon):** `python3 -m json.tool metadata.json`
clean; `gjs -c 'import("./dbus.js")'` loads with no output (success); `gjs -c 'import("./extension.js")'`
fails only on the same `resource:///org/gnome/shell/...` promise-rejection as every prior version
(no `SyntaxError`, `Gjs-WARNING **: Unhandled promise rejection` from `import()` itself, which is
the expected failure mode outside a real Shell — matches CLAUDE.md's stated pass/fail bar).

**Owed before this can ship:**
1. **XML diff against the daemon's checked-in copy.** `crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml`
   did not yet have `EqPresets`/`Eq`/`SetEq` when this pass ran — once the daemon agent lands them,
   diff `extension/dbus.js`'s `DaemonInterfaceXml` against it attribute-for-attribute (arg names,
   ordering, the exact `a(ss)`/`a(us)` signatures) and reconcile any drift, per this file's own
   header comment.
2. **Live verification on chronos** (needs both sides deployed + Jan logged into GNOME, same
   Wayland hot-reload constraint as every prior extension change):
   - Import a real AutoEq preset (`pipedeck eq import`), confirm it shows up as a labeled row
     (not just an id) and picking it moves the CHECK ornament off "Off".
   - Switch the **default output** (Output section) and confirm the Equalizer section's CHECK
     follows to that device's own `Eq` entry, not the previous device's.
   - Daemon with zero presets configured (fresh/no presets dir yet) → Equalizer header+section
     genuinely absent from the menu, not present-but-empty.
   - Restart daemon after picking a preset → on reconnect, the row that was CHECKed before the
     restart is CHECKed again (persisted `[eq]` config re-read into `Eq`).
   - An older daemon still running v0.2-era code (Phase 5, no EQ support at all) must render the
     rest of the panel exactly as before with **no console errors and no Equalizer section** —
     this is the fallback path `eqPresets: null` exists for and wasn't exercisable without a real
     mixed-version daemon.


### Phase 6 → everyone (daemon agent, 2026-09-02) — EQ presets, SPEC §7

**Lane touched: `crates/**` only, plus this file. `Cargo.lock` is unchanged — no new dependencies;
`pipewire-sys` is reached through the `pipewire` crate's own `pub use pw_sys as sys`.**

- **New module `crates/pipedeckd/src/eq.rs`** — the whole pure half of SPEC §7: `BandKind`/`Band`/
  `PresetFile`/`Preset`, `parse_preset`/`validate`/`load_presets`/`write_preset`/`slugify`,
  `preset_to_params`, `build_filter_chain_args`, the node-naming + hiding helpers, and
  `parse_autoeq`/`autoeq_to_preset`. No PipeWire types, so all of it is unit-tested without a graph;
  `pw.rs` is still the only module linking libpipewire.
- **`pw.rs`** gained: `ModuleHandle` (a `*mut pw_impl_module` that `pw_impl_module_destroy`s on
  drop), `EqInstance` (one per target sink), `load_filter_chain` (the one new `unsafe` call —
  `pw::sys::pw_context_load_module(context.as_raw_ptr(), …)` with a null `pw_properties`), the
  `eq_params_pod` control write, and an `Inner::apply_eq` reconciler run on every graph change,
  `SetConfig` and `Refresh`. It also binds the WirePlumber **`filters`** metadata alongside
  `default`, and every node global now carries a `hidden` flag from
  `eq::is_eq_node(pipedeck.eq, node.link-group)`.
- **Hiding**: hidden nodes are tracked (we need the main node's id to write controls and to key the
  metadata) but skipped in `publish()`, so they never appear in `Devices`/`Streams`; `sink_exists`
  ignores them too, which is what stops an EQ node ever becoming the notification sink or a
  `SetStreamTarget` destination.
- **Config**: `[eq]` stays a `toml::Table` (so a hand-edited non-string value round-trips instead of
  refusing to load) with typed accessors `eq_preset` / `set_eq_preset` / `eq_entries`. Every
  pre-existing config test is untouched and still green.
- **Checks**: `cargo build --workspace`, `cargo test --workspace` (**111 tests** = 94 daemon lib +
  17 CLI, up from 77), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`
  — all clean in the `pipedeck-dev` image.

**→ extension agent: the XML is regenerated and your `dbus.js` needs no change.** The
`introspection_xml_matches_the_checked_in_copy` test pins
`crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml` to the live interface, and it now carries exactly
the three members you wrote by hand, argument names included:

```xml
<method name="SetEq">
  <arg name="node_id" type="u" direction="in"/>
  <arg name="preset" type="s" direction="in"/>
</method>
<property name="Eq" type="a(us)" access="read"/>
<property name="EqPresets" type="a(ss)" access="read"/>
```

Full interface, in the order zbus emits it (methods in declaration order, properties alphabetical):
`SetDefault(ss)`, `SetNotificationSink(s)`, `SetVolume(ud)`, `SetMute(ub)`, `SetStreamTarget(us)`,
`SetPort(uu)`, `SetEq(us)`, `Refresh()`; signal `Changed()`; properties `Devices a(usssbbdbs)`,
`Eq a(us)`, `EqPresets a(ss)`, `NotificationSink s`, `Ports a(uussbb)`, `Streams a(ussssdb)`,
`Version s` — all read-only.

Behaviours the panel should count on:
- **`Eq` has one row per output device, always** — including sinks with no EQ, whose preset string
  is `""`. It reports the *configured* selection resolved against the library, not the chain's
  internal state, so the ornament lands on the right row the instant `SetEq` returns rather than
  after the filter chain's node shows up. A configured id that no longer names a preset reads as
  `""` (SPEC §7.1's "unknown preset name → treat as off").
- **`PropertiesChanged` for `Eq`** rides the same coalesced tick as `Devices`/`Streams`/`Ports`.
  `EqPresets` emits its own `PropertiesChanged` from `SetEq` (it is rescanned there), so a preset
  file dropped in while the panel is open appears after any `SetEq` or `Refresh`.
- **`SetEq` errors**: `NotFound` for an unknown node id or an unknown preset, `InvalidArgument` for
  a source (EQ is output-only). `SetEq(id, "")` on a sink that already has no EQ is a silent no-op.
- **The EQ sinks never appear in `Devices`** — the real sink stays the default and stays the row the
  user picks, exactly as SPEC §7.1 intends. There is nothing new for the Output section to filter.

**→ packaging agent:** `presets/*.toml` all parse against the loader as shipped, and
`crates/pipedeckd/src/eq.rs` has a test (`the_shipped_presets_parse`) that `include_str!`s
`presets/flat.toml` and `presets/loudness.toml` so a format drift between the lanes fails the build
instead of failing on chronos. If you rename or delete either file, that test needs the same edit.
`install.sh`'s `~/.config/pipedeck/eq` destination is exactly where the daemon scans.

**→ main session, live verification on chronos (exact commands, SPEC §7.5).** Node ids change every
session — take them from `pipedeck outputs` first.

```bash
# 0. deploy + restart, then confirm the new surface is on the bus
./install.sh && systemctl --user restart pipedeckd && sleep 2
gdbus introspect --session --dest dev.pipedeck.Daemon --object-path /dev/pipedeck/Daemon \
  | grep -E "SetEq|EqPresets|\"?Eq\"?"
pipedeck eq list          # the six shipped presets, by id and name

# 1. SPEC §7.5 acceptance 9 — the smart filter really gets inserted.
#    Start music on the analog sink FIRST, then:
pipedeck eq set <analog sink id> loudness
pw-link -l | grep -i pipedeck            # stream -> pipedeck.eq.<sink>, pipedeck.eq.<sink>.out -> <sink>
pipedeck outputs                          # "{eq: Loudness Curve}" after the port bracket
wpctl status | head -30                   # the DEFAULT SINK MUST STILL BE THE REAL SINK
pipedeck status                           # no pipedeck.eq.* node listed anywhere

#    Audible check with a deliberately extreme preset:
cat > ~/.config/pipedeck/eq/shout.toml <<'EOF'
name = "Shout (test)"
preamp_db = -6.0
[[band]]
type = "lowshelf"
freq = 120.0
q = 0.707
gain_db = 12.0
EOF
pipedeck eq set <analog sink id> shout    # rescans on SetEq, no Refresh needed; bass jumps

# 2. SPEC §7.5 acceptance 10 — off goes back to a direct link with no reload
pipedeck eq set <analog sink id> off
pw-link -l | grep -i pipedeck             # links go direct; the eq nodes are STILL THERE (bypassed)
pw-metadata -n filters                    # filter.smart.disabled = true on the main node's id
pipedeck eq set <analog sink id> loudness # re-enables instantly, no module reload

# 3. SPEC §7.5 acceptance 11 — persistence and hot-unplug
grep -A3 '\[eq\]' ~/.config/pipedeck/config.toml
systemctl --user restart pipedeckd && sleep 2 && pipedeck outputs   # EQ re-applied from config
#    then remove the sink (unplug HDMI, or `systemctl --user restart wireplumber`) and watch:
journalctl --user -u pipedeckd -f         # "sink gone; unloading its EQ filter chain", no crash
#    ... and when it returns, the chain must come back on its own.

# 4. the AutoEq importer (SPEC §7.2). Any AutoEq ParametricEQ.txt will do:
pipedeck eq import ~/Downloads/"Sennheiser HD 650 ParametricEQ.txt"
pipedeck eq show sennheiser-hd-650
pipedeck eq list

# 5. error paths — all should print a dev.pipedeck.Error message, not hang
pipedeck eq set 99999 loudness            # NotFound (no such node)
pipedeck eq set <analog source id> loudness   # InvalidArgument (input)
pipedeck eq set <analog sink id> nosuchpreset # NotFound (no such preset)

# 6. nothing regressed from v0.2 while a chain is loaded
pipedeck vol <analog sink id> 45 && wpctl get-volume <analog sink id>
pipedeck set-port <analog sink id> analog-output-lineout && pactl list sinks | grep "Active Port"
pipedeck set-notify <analog sink name> && canberra-gtk-play -i bell
pipedeck ports && pipedeck streams

# 7. panel (SPEC §7.5 acceptance 12) — Equalizer section switches presets on the default output
```

**Things that could only be reasoned about in the container — watch these first on chronos:**
1. **Re-entrancy on the module load.** `apply_eq` runs while `inner.borrow_mut()` is held (it is
   called from the registry/metadata listeners), and it calls `pw_context_load_module` from inside
   that borrow. This is safe *if* module loading never synchronously re-enters one of our own
   listeners — it should not, because the new nodes only reach us as registry globals after a
   server round-trip, and our core listener's `error` closure never touches `Inner`. If it does
   re-enter, the symptom is unmistakable: an immediate `already mutably borrowed: BorrowMutError`
   panic in the `pipedeck-pw` thread on the first `pipedeck eq set`. The fix would be to defer the
   load onto a loop idle callback rather than doing it inline.
2. **Whether `pw_context_load_module` finds a usable core.** The module is loaded into the daemon's
   own already-connected context, EasyEffects-style. If it returns NULL you get
   `could not load libpipewire-module-filter-chain` in the journal — check `RUST_LOG=debug` for the
   PipeWire-side reason (missing module, or a graph the parser rejected).
3. **The module argument string.** It is built by `eq::build_filter_chain_args` and unit-tested to
   be valid strict JSON with every SPEC §7.1 key in place, but only a live load proves filter-chain
   *accepts* it. `RUST_LOG=debug` logs "loading the EQ filter chain" with the channel count just
   before the call; the args themselves are not logged (they are ~4 KB) — add a `debug!(%args)` in
   `Inner::eq_enable` if the load fails.
4. **`SPA_PROP_params` = `0x80001`** (`SPA_PROP_START_Other + 1`, confirmed against
   `/usr/include/spa-0.2/spa/param/props.h` in the container) and the value is a `Struct` of
   alternating `String`/`Float`. The pod builds and re-parses in a unit test, but if a preset
   selects with no audible change, this write is the first suspect —
   `pw-cli s <main node id> Props '{ params = [ "ls:Gain" 12.0 ] }'` by hand tells you whether the
   chain accepts controls at all.
5. **`audio.position`.** Taken from the target sink's `audio.position` prop and only passed through
   when its length matches the channel count; otherwise the key is omitted and filter-chain infers a
   layout. Worth an eye on a 5.1 HDMI sink — `pw-dump <sink id> | grep -A2 audio.position`.
6. **The `filters` metadata may not exist until something has used a smart filter.** If it is
   absent, `SetEq(id, "")` falls back to unloading the module (logged: "the WirePlumber `filters`
   metadata is missing; unloading the EQ filter chain instead") and the next `SetEq` reloads it.
   That is a correct-but-slower path, not a fault — but if it happens *always*, the instant-bypass
   half of SPEC §7.1 is not being exercised at all.

**Deliberate choices / deviations from §7, all small and all flagged:**
- **`Eq` reports the configured selection, not the chain's applied state** (see above). SPEC §7.3
  just says "(node_id, preset id or \"\")"; reporting configuration is what makes the panel's
  ornament stable across the load → node-appears → controls-written sequence.
- **The graph is built once and never rebuilt.** SPEC §7.1 already says so ("controls updated
  live"), and `preset_to_params` always writes *every* control on the graph — unused bands get
  `Gain 0` at their slot defaults — so switching from a 9-band preset to a 2-band one cannot leave
  bands from the previous preset behind. That is the one non-obvious invariant in the module.
- **A chain is loaded lazily**, only when a sink actually has a preset, and is unloaded (not
  bypassed) when its sink disappears or when "off" is asked for before its node ever came up.
  Steady-state "off" on a sink that *had* EQ is the metadata bypass SPEC §7.1 asks for.
- **`SetEq` rescans the preset directory before validating** (SPEC §7.3 says the property is
  rescanned there), so a file dropped in a second ago is selectable without a `Refresh` first.
  `Refresh` rescans too. A directory that cannot be read leaves the previous library in place — a
  transient I/O error must not silently empty the panel's preset list.
- **Preset ids are slugs.** The file stem is `slugify`d (lowercase, `[a-z0-9-]`), so
  `HD 650.toml` and `hd-650.toml` are the same id. `eq import` derives the id from `--name`, or
  from the file stem with AutoEq's ` ParametricEQ` suffix stripped.
- **Validation ranges** (freq 10–24000 Hz, Q 0.05–20, gain ±30 dB, preamp ±30 dB) are the ones the
  task set; SPEC §7.2 fixes only the band *counts*. The importer **clamps** out-of-range AutoEq
  values with a warning rather than rejecting the file, because a single wild band should not cost
  the user the import; a hand-written preset file with the same value is rejected outright.
- **`pipedeck eq show` and `eq import` read and write the presets directory directly** rather than
  going through D-Bus — the bus only carries `(id, name)`, and the directory is the daemon's own
  source of truth. `import` ends with a best-effort `Refresh` so `eq list` is immediately current.
- **`{eq: <name>}` shows on `status`, `outputs` and `inputs`** (they share one `device_line`
  renderer); SPEC §7.3 only asked for `outputs`, and sources never have an EQ row so it is invisible
  there anyway. Same reasoning as the Phase 5 port bracket.
