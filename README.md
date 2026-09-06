# PipeDeck

SoundSource-style audio control for PipeWire desktops: a Quick Settings panel for GNOME Shell with output / input / **notification** device pickers, headphone-vs-speaker port switching, per-application volume, and a parametric equalizer with AutoEq import — built on nothing but libpipewire, WirePlumber and the GNOME Shell APIs.

> **Status:** v1.2.0. Developed and tested on one machine (Ubuntu 26.04.1,
> GNOME Shell 50.1, PipeWire 1.6.2, WirePlumber 0.5.13, Realtek ALC892 + NVIDIA HDMI). Reports from
> other cards, Bluetooth devices and multi-channel outputs are welcome. See [CHANGELOG.md](CHANGELOG.md).

## Features

- **Output device picker** — Select the default audio output sink
- **Input device picker** — Select the default audio input source  
- **Notification device picker** — Route notification/event sounds independently (unique to PipeDeck)
- **Ports** — Headphones, Line Out, HDMI… listed as separate rows even when they are ports of one card; the codec's *Auto-Mute Mode* (which silently kills line-out while headphones are plugged in) is detected and switched off for you
- **Per-application volume sliders** — Control volume and mute for every playback stream
- **Equalizer presets** — Parametric EQ per output via PipeWire's filter-chain, inserted transparently by WirePlumber as a smart filter; import headphone corrections straight from [AutoEq](https://github.com/jaakkopasanen/AutoEq)
- **Menu-bar integration** — Built into GNOME Quick Settings for quick access

Future (v1.2):
- Automatic null sink for notification routing

## Architecture

```
GNOME Shell extension (GJS) ──D-Bus──▶ pipedeckd daemon (Rust) ──libpipewire──▶ PipeWire
pipedeck CLI (Rust, same crate) ─────┘
```

- **pipedeckd** — A lightweight daemon that manages all PipeWire interaction via D-Bus
- **extension** — A GNOME Shell extension that provides the Quick Settings UI
- **pipedeck CLI** — Command-line tool for scripting and testing

## Requirements

- **PipeWire** 1.0 or later (1.6 tested) and **WirePlumber** 0.5 or later (smart filters are needed for the EQ)
- **GNOME Shell 50** for the panel (the daemon and CLI work on any desktop)
- **Rust** 1.85+ with `libpipewire-0.3-dev`, `libclang-dev`, `pkg-config` to build (Ubuntu 26.04: `sudo apt install cargo rustc libpipewire-0.3-dev libclang-dev clang pkg-config build-essential libasound2-dev`)

No dependency on EasyEffects, pavucontrol, or other external audio tools — PipeDeck uses only platform libraries (libpipewire, WirePlumber metadata, GNOME Shell APIs).

## Installation

### From source

```bash
git clone https://github.com/jvandenbos/pipedeck.git
cd pipedeck
./install.sh
```

The install script will:
1. Build PipeDeck with `cargo build --release`
2. Install binaries to `~/.local/bin/`
3. Install the systemd user service and D-Bus activation file
4. Copy the GNOME Shell extension to `~/.local/share/gnome-shell/extensions/`
5. Enable the extension and start the daemon

### Installation options

```bash
./install.sh --no-extension   # Skip extension installation
./install.sh --no-service     # Skip systemd service installation
```

### Uninstallation

```bash
./uninstall.sh           # Remove binaries and service, keep config
./uninstall.sh --purge   # Also remove ~/.config/pipedeck
```

## Usage

### CLI

The `pipedeck` command provides a CLI interface for scripting and testing:

```bash
# Show daemon status and current devices/streams
pipedeck status

# List available output devices
pipedeck outputs

# List available input devices  
pipedeck inputs

# Set the default output device
pipedeck set-output <device-name>

# Set the default input device
pipedeck set-input <device-name>

# Set the notification output device
pipedeck set-notify <device-name>
pipedeck set-notify none   # Follow default output

# Set volume (0-150, where 100 is unity gain)
pipedeck vol <stream-id> <0-150>

# Mute/unmute a stream
pipedeck mute <stream-id> on
pipedeck mute <stream-id> off
pipedeck mute <stream-id>      # Toggle

# Watch for changes (live updates)
pipedeck watch

# Equalizer: list available presets
pipedeck eq list

# Show details of a specific preset
pipedeck eq show bass-boost

# Apply a preset to an output device (node id)
pipedeck eq set <node-id> vocal-clarity

# Disable EQ on an output device
pipedeck eq set <node-id> off

# Import a preset from AutoEq (see https://github.com/crinacle/AutoEq/tree/master/results)
pipedeck eq import "Sennheiser HD 650 ParametricEQ.txt" --name hd650
```

### GUI

Open GNOME Settings → Quick Settings (the panel in the top-right) and look for the "Audio" section. The PipeDeck panel will show:

1. **Output devices** — radio button list of available sinks
2. **Input devices** — radio button list of available sources
3. **Notification device** — independent routing for event sounds
4. **Application sliders** — one slider per active audio stream, with mute button
5. **Equalizer** — A menu of EQ presets for the current default output device

## Equalizer

PipeDeck's equalizer applies parametric EQ through PipeWire's smart filter-chain module, which inserts a virtual filter transparently between applications and the real output device. The real device remains the default, and all port/volume/notification routing continues to work normally — nothing visible changes except the audio signal path.

**Presets** are stored as TOML files in `~/.config/pipedeck/eq/` and describe a set of parametric bands (low-shelf, peaking, high-shelf) plus a preamp gain to prevent clipping. The panel shows an "Equalizer" section when presets are available; selecting one applies it to the current default output device. Selecting "Off" disables EQ. Presets are stored in PipeDeck's config and re-applied when the daemon starts.

### Preset file format

Each preset is a TOML file at `~/.config/pipedeck/eq/<slug>.toml`:

```toml
name = "My Preset"          # Display name
preamp_db = -2.0            # Preamp gain (negative = protection against clipping)

[[band]]
type = "lowshelf"           # lowshelf | peaking | highshelf
freq = 100.0                # Frequency in Hz (20–20000)
q = 0.707                   # Q factor (0.5–2.0 typical)
gain_db = 5.0               # Gain in dB (−8 to +8 typical)

[[band]]
type = "peaking"
freq = 2000.0
q = 1.2
gain_db = 3.0
```

Max 1 lowshelf + 12 peaking + 1 highshelf per preset. Unused bands are automatically filtered out when applying.

### AutoEq import

PipeDeck can import presets from AutoEq's parametric EQ files (https://github.com/crinacle/AutoEq/tree/master/results), where each headphone model has a `ParametricEQ.txt` file:

```bash
pipedeck eq import "Sennheiser HD 650 ParametricEQ.txt" --name hd650
```

This parses the AutoEq format (`Preamp: -2.5 dB`, `Filter 1: ON PK Fc 200 Hz Gain 2.1 dB Q 0.70`) and writes a TOML preset.

### Bundled presets

| Preset | Description |
|--------|-------------|
| **Flat** | No EQ applied (unity response) |
| **Bass Boost** | +5 dB lowshelf at 100 Hz for punchier bass |
| **Vocal Clarity** | Vocal presence boost: −2 dB at 250 Hz, +3 dB at 2500 Hz, +1.5 dB shelf at 8 kHz |
| **Loudness Curve** | Loudness compensation: +4 dB bass, −1 dB midrange, +3 dB treble (subjective loudness) |
| **Late Night** | Tame bass rumble (−6 dB lowshelf at 120 Hz) for late-night listening |
| **Treble Tame** | Reduce harsh sibilance (−4 dB highshelf at 6 kHz) |

## Configuration

Config file: `~/.config/pipedeck/config.toml`

```toml
# Which sink receives notification/event sounds.
# Empty string "" means follow the default output device.
notification_sink = ""

# Additional application names to treat as notifications.
# (media.role "event" and "Notification" are always included)
notification_apps = []

# EQ preset mapping: sink node.name → preset id (file stem)
[eq]
# Example: "alsa_output.pci-0000_28_00.4.analog-stereo" = "vocal-clarity"
```

## Development

### Build targets

```bash
make build          # Build Rust workspace
make test           # Run all tests
make clippy         # Run clippy linter
make fmt            # Check code formatting
make check          # Run all checks

make ext-check      # Check extension JavaScript syntax

make install        # Build and install (Linux only)
make uninstall      # Uninstall from system
```

All build and test commands run in the Docker dev container (`pipedeck-dev`) to ensure consistency.

### Building the Docker image

```bash
make docker-build   # Build the dev container
```

Or manually:
```bash
docker build -t pipedeck-dev dev/
```

### Running tests

```bash
make test
```

Unit tests run in the Docker container (no PipeWire runtime needed). Integration tests on the target system (chronos) are run by the main CI session.

### Code structure

```
crates/
  pipedeckd/       # Main daemon binary + library
    src/
      main.rs      # Entry point
      db/          # D-Bus service
      pw/          # PipeWire interaction
      config.rs    # Configuration
      state.rs     # Graph state
  pipedeck-cli/    # CLI binary
    src/
      main.rs      # CLI commands

extension/         # GNOME Shell extension (GJS)
  extension.js     # Main extension class
  stylesheet.css   # UI styling
  metadata.json    # Extension metadata

packaging/         # Installation files
  pipedeckd.service              # systemd user unit
  dev.pipedeck.Daemon.service    # D-Bus activation

install.sh         # Installation script
uninstall.sh       # Uninstallation script
Makefile           # Build targets
```

## D-Bus Interface

The daemon exposes itself on the session bus with well-known name `dev.pipedeck.Daemon` and provides the following D-Bus interface:

**Interface:** `dev.pipedeck.Daemon1`

**Properties:**
- `Devices` — Array of audio devices (sinks/sources)
- `Streams` — Array of active audio streams
- `Ports` — Array of audio device ports (output/input routes)
- `NotificationSink` — Current notification sink name
- `EqPresets` — Array of available EQ presets (id, name pairs)
- `Eq` — Array of current EQ settings per output device (node_id, preset_id pairs; empty preset_id = off)
- `Version` — Daemon version

**Methods:**
- `SetDefault(kind: String, name: String)` — Set default sink/source (`kind` = "sink" or "source")
- `SetNotificationSink(name: String)` — Route notifications to a specific sink (empty = follow default)
- `SetPort(id: UInt32, index: UInt32)` — Switch output/input port on a device (e.g. headphones ↔ speakers)
- `SetVolume(id: UInt32, volume: Double)` — Set volume 0.0–3.375 linear (= 0–150 % on the cubic scale wpctl/GNOME show)
- `SetMute(id: UInt32, mute: Boolean)` — Mute/unmute a device or stream
- `SetStreamTarget(id: UInt32, name: String)` — Route a stream to a specific sink (advanced)
- `SetEq(id: UInt32, preset: String)` — Apply an EQ preset to a sink (empty preset = off)
- `Refresh()` — Force a state refresh (rescans presets, rescans devices/ports/streams)

**Signals:**
- `Changed()` — Emitted when graph state changes (coalesced to ≤10/s)

## Troubleshooting

### Daemon won't start

Check the systemd user service:
```bash
systemctl --user status pipedeckd
journalctl --user -u pipedeckd -f   # Follow logs
```

Common issues:
- PipeWire not running: `systemctl --user status pipewire`
- Missing libraries: install build dependencies (`libpipewire-dev`, `libdbus-1-dev`)

### Extension not showing in Quick Settings

1. Verify the daemon is running: `systemctl --user is-active pipedeckd`
2. Force a Shell restart: Press Alt+F2, type `r`, press Enter (or log out and back in)
3. Check enabled extensions: `gsettings get org.gnome.shell enabled-extensions`

### Audio device not appearing

- Ensure PipeWire is the active sound server: `pw-cli info 0 | grep server.version`
- Check WirePlumber is managing devices: `wpctl status`

## License

MIT License. Copyright 2026 Jan Vandenbos.

See LICENSE file for details.

## See also

- [SPEC.md](SPEC.md) — Full specification
- [CLAUDE.md](CLAUDE.md) — Developer notes

## Documentation

- [SPEC.md](SPEC.md) — the design contract: graph model, D-Bus interface, ports, EQ mechanism, acceptance tests
- [PLAN.md](PLAN.md) — work log, live-test notes and the gotchas found on real hardware
- [CHANGELOG.md](CHANGELOG.md) — release history

## How it was built

PipeDeck was specified and integration-tested by a person driving Claude Code, with the daemon,
extension and packaging written by parallel agent sessions against `SPEC.md`. Every mechanism the
agents could not verify without a live PipeWire graph was tested on real hardware afterwards; the
bugs that only showed up there are recorded in `PLAN.md` and `CLAUDE.md`.
