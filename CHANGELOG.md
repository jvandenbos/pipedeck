# Changelog

All notable changes to PipeDeck. Versions follow [Semantic Versioning](https://semver.org);
milestones map to the numbered sections of `SPEC.md`.

## [Unreleased]

## [1.4.0] — 2026-10-26

### Added
- **`pipedeck-ui`**, the Qt Quick window for Omarchy / Hyprland (`ui/`), and the Arch package
  `pipedeck` (`packaging/arch/PKGBUILD`: daemon, CLI, UI, user unit, D-Bus activation, launcher
  entry and system presets in one package; SPEC-UI §7). `ui/bin/install-user` installs the UI
  under `~/.local` without root.
- `SPEC-UI.md`: the Qt Quick front end for Omarchy / Hyprland (`pipedeck-ui`), reviewed with Codex
  (`docs/2026-10-26-ui-design-review.md`). Daemon and CLI verified to build and run unmodified on Arch
  (PipeWire 1.6.9, WirePlumber 0.5.18); the smart-filter EQ measured working there.
- **`StreamSinks a(us)`** D-Bus property (SPEC-UI §6.1): where each playback stream is
  *actually* playing, `(stream id, sink node.name or "")`, derived from PipeWire `Link` globals.
  Per-channel links collapse to one destination, PipeDeck's own EQ filter resolves to the physical
  sink it fronts, and a stream linked to two sinks at once reports the lower node id. Emitted with
  `PropertiesChanged` + `Changed` like the other properties. `pipedeck status`/`streams` print
  `on: <sink>` after the requested target when the two differ.
- **Manual stream targets beat notification routing** (SPEC-UI §6.2): a stream that received
  `SetStreamTarget` this session — `""` ("follow default") included — is never re-targeted or
  released by notification routing, on `SetNotificationSink`, new streams or hotplug. Forgotten
  when the stream goes away.
- **System EQ presets** (SPEC-UI §6.4, SPEC §7.2): `<dir>/pipedeck/presets/*.toml` for each
  `$XDG_DATA_DIRS` entry (default `/usr/local/share:/usr/share`) is read alongside
  `~/.config/pipedeck/eq/`, read-only; a user preset with the same id wins. `pipedeck eq import`
  still writes the user directory only; `eq show` finds system presets too.
- **Arch packaging units** (SPEC-UI §6.3): `packaging/arch/pipedeckd.service` and
  `packaging/arch/dev.pipedeck.Daemon.service` run `/usr/bin/pipedeckd`; the `%h/.local/bin`
  units stay for `install.sh`.

### Changed
- **`Streams.target_name` is now the *requested* target** — the stream's `target.object`
  metadata, else the stream node's own `target.object` property (`pw-play --target`,
  `PULSE_SINK`), metadata winning; serials resolve to `node.name`, `""` when there is neither. Through 1.3.0 it reported the
  live link's destination first, so it was observed state (and read `pipedeck.eq.<sink>` while EQ
  was on). The CLI's `-> <sink>` and anything else reading the field now show intent; the observed
  sink is `StreamSinks`, shown separately by `pipedeck status`. The GNOME extension does not render
  the field.

### Fixed
- **"Follow default" now follows the default for streams that picked a sink when they
  connected** (`pw-play --target`, `PULSE_SINK`). `SetStreamTarget(id, "")` used to delete the
  `target.object` metadata, and WirePlumber then fell back to the stream's own request, so it
  snapped straight back. The daemon now writes WirePlumber's `-1` "no target" value when the
  stream has a request of its own (and still just deletes the key when it doesn't);
  `target_name` reads `-1` as `""`, and also honours the older `target.node` metadata and
  `node.target` property (node ids), in WirePlumber's order.
- **A stream that drops its connect-time target no longer keeps reporting it** in
  `target_name`.
- **`pipedeck eq show <id>` shows the preset the daemon actually uses**: a broken user file no
  longer hides a valid bundled preset of the same id, and a file named `Bass Boost.toml` is
  found as `bass-boost`. A preset rescan that cannot read a preset directory (permissions)
  now keeps the previous preset list instead of emptying it.

## [1.3.0] — 2026-09-07

Loudness safety (SPEC §9).

### Changed
- **Volume and mute writes are independent.** A volume write carries only `channelVolumes`, a
  mute write only `mute` — never the daemon's cached value of the other field, so a mute made
  elsewhere a moment earlier can no longer be undone by a slider move (and vice-versa).

### Added
- **Port-switch level cap.** WirePlumber restores volume per port, so switching from quiet
  headphones to speakers stored at 82 % used to jump straight there. After a PipeDeck-initiated
  port switch the new port is clamped to `[safety] port_switch_max_percent` (default 60 % on the
  cubic scale) for a 2 s window, re-applied if WirePlumber's own restore lands later; a volume
  change by the user inside the window cancels it. `pipedeck cap [<0-150>|off]`, D-Bus
  `PortSwitchCap` / `SetPortSwitchCap`. Outputs only — capture levels are never touched.

## [1.2.0] — 2026-09-06

### Added
- **ALSA auto-mute detection and switch** (SPEC §8). Realtek-style codecs have a mixer enum
  `Auto-Mute Mode` that silences line-out whenever a headphone plug is present, so selecting the
  speaker port produced silence. The daemon now reads it through alsa-lib, flips it off
  automatically when a speaker port is selected while headphones are plugged in (policy `auto`),
  persists the choice per card in `[alsa.auto_mute]` and re-applies it after reboots.
  `AutoMute` property, `SetAutoMute`, `pipedeck automute [<id> on|off]`, `[auto-mute]` tag.
- Panel: "Auto-mute speakers when headphones are plugged in" switch under the device's port rows.

### Fixed
- Extension: use `GioUnix.DesktopAppInfo` on GNOME 50 (silences a per-rebuild deprecation trace).

### Build
- `libasound2-dev` is now required to build (runtime needs only `libasound.so.2`).

## [1.1.0] — 2026-09-05

First public release.

### Added
- **Equalizer** (SPEC §7): per-output parametric EQ realised as a PipeWire `filter-chain` that
  WirePlumber inserts as a *smart filter* — the real device stays the default output, ports,
  volume and notification routing are untouched. Presets are TOML files in
  `~/.config/pipedeck/eq/`; six bundled presets; `pipedeck eq list|show|set|import`.
- **AutoEq importer**: `pipedeck eq import <ParametricEQ.txt>` converts AutoEq's parametric
  results into a preset.
- Preset switching is a single live control write (no module reload); "off" is an instant
  WirePlumber bypass via the `filters` metadata.
- Panel: **Equalizer** section applying to the current default output.
- D-Bus: `EqPresets`, `Eq`, `SetEq`.

### Fixed
- Filter-chain nodes are recognised from the node's own info event; the registry global only
  carries a whitelist of properties, so custom tags never appear there.
- Installer restarts an already-running daemon and no longer aborts on the first copied preset.

## [1.0.1] — 2026-09-01

### Added
- **Ports** (SPEC §6): headphones / line-out / HDMI style ports are ALSA *routes* of one sink.
  `Ports` property, `SetPort`, `pipedeck ports`, `pipedeck set-port`; the panel lists one row
  per selectable port ("Headphones · ALC892 Analog").
- Device `nick` in the `Devices` tuple; the panel and CLI show short names.
- Panel: wider menu, end-ellipsised labels.

### Fixed
- Volume and mute on ALSA-backed devices go through the device `Route` param (WirePlumber owns
  hardware volume there; node `Props` writes were silently ignored).
- Live read-back: device parameter changes are re-enumerated from the device `info` event —
  `subscribe_params` never delivers route changes on PipeWire 1.6.
- Volume percentages use the cubic scale that `wpctl` and GNOME display (100 % = 1.0 linear,
  150 % = 3.375).

## [1.0.0] — 2026-09-01

### Added
- `pipedeckd`: Rust daemon over libpipewire — devices, streams, WirePlumber defaults, per-stream
  volume/mute/target, **notification-sink routing** for `media.role=event` streams, TOML config,
  zbus interface `dev.pipedeck.Daemon1`, systemd user unit + D-Bus activation.
- `pipedeck` CLI.
- GNOME Shell 50 Quick Settings extension: Output / Input / Notifications pickers and per-app
  volume sliders.
- Docker dev image mirroring the target (Ubuntu 26.04, PipeWire 1.6, Rust 1.93).

[Unreleased]: https://github.com/jvandenbos/pipedeck/compare/v1.4.0...HEAD
[1.4.0]: https://github.com/jvandenbos/pipedeck/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/jvandenbos/pipedeck/releases/tag/v1.3.0
[1.2.0]: https://github.com/jvandenbos/pipedeck/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/jvandenbos/pipedeck/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/jvandenbos/pipedeck/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/jvandenbos/pipedeck/commits/v1.0.0
