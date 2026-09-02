#!/bin/bash
# PipeDeck uninstallation script — runs on the Linux target
# Usage: ./uninstall.sh [--purge]
# --purge also removes ~/.config/pipedeck

set -euo pipefail

readonly EXTENSION_UUID="pipedeck@jvandenbos.github.io"
readonly EXTENSION_DEST="${HOME}/.local/share/gnome-shell/extensions/${EXTENSION_UUID}"
readonly SERVICE_DEST="${HOME}/.config/systemd/user/pipedeckd.service"
readonly DBUS_DEST="${HOME}/.local/share/dbus-1/services/dev.pipedeck.Daemon.service"
readonly BIN_DIR="${HOME}/.local/bin"
readonly CONFIG_DIR="${HOME}/.config/pipedeck"

# Parse flags
PURGE=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --purge)
      PURGE=true
      shift
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

echo "=== PipeDeck Uninstallation ==="

# Stop and disable service
if systemctl --user is-active pipedeckd.service &>/dev/null; then
  echo "Stopping pipedeckd service..."
  systemctl --user stop pipedeckd.service
fi

if systemctl --user is-enabled pipedeckd.service &>/dev/null; then
  echo "Disabling pipedeckd service..."
  systemctl --user disable pipedeckd.service
fi

# Remove systemd unit
if [[ -f "$SERVICE_DEST" ]]; then
  echo "Removing systemd user unit..."
  rm -f "$SERVICE_DEST"
  systemctl --user daemon-reload
  echo "  ✓ Removed"
fi

# Remove D-Bus service file
if [[ -f "$DBUS_DEST" ]]; then
  echo "Removing D-Bus service file..."
  rm -f "$DBUS_DEST"
  echo "  ✓ Removed"
fi

# Disable extension
echo "Disabling GNOME Shell extension..."
if gnome-extensions disable "$EXTENSION_UUID" 2>/dev/null; then
  echo "  ✓ ${EXTENSION_UUID} disabled"
else
  # Fallback: remove from gsettings enabled-extensions
  if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
    eval "$(dbus-launch --sh-syntax)"
  fi

  current_exts=$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo "[]")
  if echo "$current_exts" | grep -q "$EXTENSION_UUID"; then
    new_exts=$(python3 -c "
import json
current = json.loads('''$current_exts''')
if isinstance(current, list):
  current = [e for e in current if e != '''$EXTENSION_UUID''']
print(json.dumps(current))
")
    gsettings set org.gnome.shell enabled-extensions "$new_exts"
    echo "  ✓ ${EXTENSION_UUID} removed from enabled-extensions"
  fi
fi

# Remove extension directory
if [[ -d "$EXTENSION_DEST" ]]; then
  echo "Removing extension directory..."
  rm -rf "$EXTENSION_DEST"
  echo "  ✓ Removed"
fi

# Remove binaries
echo "Removing binaries..."
rm -f "$BIN_DIR/pipedeckd" "$BIN_DIR/pipedeck"
echo "  ✓ Removed"

# Remove config if --purge
if [[ "$PURGE" == "true" ]]; then
  if [[ -d "$CONFIG_DIR" ]]; then
    echo "Purging configuration directory..."
    rm -rf "$CONFIG_DIR"
    echo "  ✓ Removed"
  fi
fi

echo ""
echo "=== Uninstallation Complete ==="
if [[ "$PURGE" != "true" ]]; then
  echo "Configuration directory preserved at: $CONFIG_DIR"
  echo "Use --purge to remove it."
fi
