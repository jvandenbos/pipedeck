# Changelog

All notable changes to PipeDeck. Versions follow [Semantic Versioning](https://semver.org);
milestones map to the numbered sections of `SPEC.md`.

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

[1.1.0]: https://github.com/jvandenbos/pipedeck/releases/tag/v1.1.0
[1.0.1]: https://github.com/jvandenbos/pipedeck/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/jvandenbos/pipedeck/commits/v1.0.0
