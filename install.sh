#!/bin/bash
# PipeDeck installation script — runs on the Linux target
# Usage: ./install.sh [--no-extension] [--no-service]

set -euo pipefail

SCRIPT_DIR
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly EXTENSION_UUID="pipedeck@jvandenbos.github.io"
readonly EXTENSION_SRC="${SCRIPT_DIR}/extension"
readonly EXTENSION_DEST="${HOME}/.local/share/gnome-shell/extensions/${EXTENSION_UUID}"
readonly SERVICE_DEST="${HOME}/.config/systemd/user/pipedeckd.service"
readonly DBUS_DEST="${HOME}/.local/share/dbus-1/services/dev.pipedeck.Daemon.service"
readonly BIN_DIR="${HOME}/.local/bin"

# Parse flags
NO_EXTENSION=false
NO_SERVICE=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-extension)
      NO_EXTENSION=true
      shift
      ;;
    --no-service)
      NO_SERVICE=true
      shift
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

echo "=== PipeDeck Installation ==="

# Build
echo "Building PipeDeck with cargo..."
cd "$SCRIPT_DIR"
cargo build --release --workspace

# Install binaries
echo "Installing binaries to ${BIN_DIR}..."
mkdir -p "$BIN_DIR"
install -Dm755 target/release/pipedeckd "$BIN_DIR/pipedeckd"
install -Dm755 target/release/pipedeck "$BIN_DIR/pipedeck"
echo "  ✓ pipedeckd"
echo "  ✓ pipedeck"

# Install systemd unit
if [[ "$NO_SERVICE" != "true" ]]; then
  echo "Installing systemd user unit..."
  mkdir -p "$(dirname "$SERVICE_DEST")"
  install -Dm644 packaging/pipedeckd.service "$SERVICE_DEST"
  echo "  ✓ ${SERVICE_DEST}"

  # Install D-Bus service file
  echo "Installing D-Bus service file..."
  mkdir -p "$(dirname "$DBUS_DEST")"
  install -Dm644 packaging/dev.pipedeck.Daemon.service "$DBUS_DEST"
  echo "  ✓ ${DBUS_DEST}"

  # Reload systemd user daemon
  echo "Reloading systemd user daemon..."
  systemctl --user daemon-reload

  # Enable and start the service
  echo "Enabling and starting pipedeckd..."
  systemctl --user enable pipedeckd.service
  systemctl --user start pipedeckd.service
  echo "  ✓ pipedeckd enabled and started"
else
  echo "Skipping systemd installation (--no-service)"
fi

# Install extension
if [[ "$NO_EXTENSION" != "true" ]]; then
  if [[ ! -d "$EXTENSION_SRC" ]]; then
    echo "Warning: extension/ directory not found at $EXTENSION_SRC" >&2
    echo "Extension installation skipped. Build phase 2 first." >&2
  else
    echo "Installing GNOME Shell extension..."
    # Remove old installation if present
    if [[ -d "$EXTENSION_DEST" ]]; then
      echo "  Removing old installation at ${EXTENSION_DEST}..."
      rm -rf "$EXTENSION_DEST"
    fi
    # Copy extension
    mkdir -p "$(dirname "$EXTENSION_DEST")"
    cp -r "$EXTENSION_SRC" "$EXTENSION_DEST"
    echo "  ✓ ${EXTENSION_DEST}"

    # Enable extension
    echo "Enabling GNOME Shell extension..."
    if ! gnome-extensions enable "$EXTENSION_UUID" 2>/dev/null; then
      # Fallback: gnome-extensions not available or extension not recognized yet
      # Use gsettings to add to enabled-extensions
      echo "  gnome-extensions not available, using gsettings fallback..."

      # Ensure DBUS_SESSION_BUS_ADDRESS is set for gsettings
      if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
        eval "$(dbus-launch --sh-syntax)"
      fi

      # Get current enabled extensions
      current_exts=$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo "[]")

      # Check if already enabled
      if ! echo "$current_exts" | grep -q "$EXTENSION_UUID"; then
        # Add to enabled extensions (preserve existing entries)
        # Use a Python one-liner to safely parse and modify the list
        new_exts=$(python3 -c "
import json
import sys
current = json.loads('''$current_exts''')
if isinstance(current, list):
  if '''$EXTENSION_UUID''' not in current:
    current.append('''$EXTENSION_UUID''')
else:
  current = ['''$EXTENSION_UUID''']
print(json.dumps(current))
")
        gsettings set org.gnome.shell enabled-extensions "$new_exts"
      fi
      echo "  ✓ ${EXTENSION_UUID} added to enabled-extensions"
    else
      echo "  ✓ ${EXTENSION_UUID} enabled"
    fi
  fi
else
  echo "Skipping extension installation (--no-extension)"
fi

echo ""
echo "=== Installation Complete ==="
echo "PipeDeck binaries installed to: $BIN_DIR"
echo "Extension UUID: $EXTENSION_UUID"
if [[ "$NO_SERVICE" != "true" ]]; then
  echo "Systemd user service: $SERVICE_DEST"
  echo "Status: systemctl --user status pipedeckd"
  echo "Logs: journalctl --user -u pipedeckd -f"
fi
echo ""
echo "To verify installation:"
echo "  systemctl --user status pipedeckd"
echo "  pipedeck status"
