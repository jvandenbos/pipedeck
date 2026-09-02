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

## v1.1 (later)
- ☐ EQ via filter-chain (SPEC §2.5), preset picker in panel, AutoEq importer.
- ☐ Optional null sink `pipedeck.notifications`.

## Handoffs / notes between agents

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
