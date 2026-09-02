# PipeDeck

SoundSource-style audio control for PipeWire desktops. A compact menu-bar panel for GNOME Shell that provides output/input/notification device selection and per-application volume control.

## Features

- **Output device picker** — Select the default audio output sink
- **Input device picker** — Select the default audio input source  
- **Notification device picker** — Route notification/event sounds independently (unique to PipeDeck)
- **Per-application volume sliders** — Control volume and mute for every playback stream
- **Menu-bar integration** — Built into GNOME Quick Settings for quick access

Future (v1.1):
- EQ presets via PipeWire's filter-chain module
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

- **OS**: Ubuntu 26.04 LTS or compatible
- **Desktop**: GNOME 50+ (Wayland or X11)
- **PipeWire**: 1.0 or later
- **WirePlumber**: 0.5 or later
- **Rust**: 1.70+ (for building from source)

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
```

### GUI

Open GNOME Settings → Quick Settings (the panel in the top-right) and look for the "Audio" section. The PipeDeck panel will show:

1. **Output devices** — radio button list of available sinks
2. **Input devices** — radio button list of available sources
3. **Notification device** — independent routing for event sounds
4. **Application sliders** — one slider per active audio stream, with mute button

## Configuration

Config file: `~/.config/pipedeck/config.toml`

```toml
# Which sink receives notification/event sounds.
# Empty string "" means follow the default output device.
notification_sink = ""

# Additional application names to treat as notifications.
# (media.role "event" and "Notification" are always included)
notification_apps = []

# v1.1: EQ presets (not yet implemented)
[eq]
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
- `NotificationSink` — Current notification sink name
- `Version` — Daemon version

**Methods:**
- `SetDefault(kind: String, name: String)` — Set default sink/source (`kind` = "sink" or "source")
- `SetNotificationSink(name: String)` — Route notifications to a specific sink (empty = follow default)
- `SetVolume(id: UInt32, volume: Double)` — Set volume 0.0–3.375 linear (= 0–150 % on the cubic scale wpctl/GNOME show)
- `SetMute(id: UInt32, mute: Boolean)` — Mute/unmute a device or stream
- `SetStreamTarget(id: UInt32, name: String)` — Route a stream to a specific sink (advanced)
- `Refresh()` — Force a state refresh

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
