# PipeDeck — PLAN

Status legend: ☐ todo · ◐ in progress · ☑ done · ✗ blocked

## Phase 1 — daemon + CLI (Rust)  — agent: Opus
- ☐ Workspace skeleton: `Cargo.toml` workspace, `crates/pipedeckd` (lib + bin), `crates/pipedeck-cli`.
- ☐ PipeWire thread: `MainLoop` + `Context` + `Core` + `Registry`; track nodes (media.class
  sink/source/stream), their props, `Props` params (volume/mute), and the `default` metadata
  object. Publish snapshots into `Arc<RwLock<State>>` + notify channel.
- ☐ Commands PW-thread-side: set default (metadata write), set volume/mute (Props param via
  `pw_node_set_param`), set stream target (metadata `target.object`).
- ☐ Notification routing per SPEC §2.1 (matching rule unit-tested; re-apply on new stream, on
  sink appear, on config change).
- ☐ Config load/save (`~/.config/pipedeck/config.toml`, atomic write, round-trip test).
- ☐ zbus service `dev.pipedeck.Daemon1` per SPEC §2.2 (properties + PropertiesChanged + Changed
  signal coalesced ≤10/s).
- ☐ CLI per SPEC §2.3.
- ☐ `cargo test`, `clippy -D warnings`, `fmt` clean in the dev image.

## Phase 2 — GNOME Shell extension  — agent: Sonnet
- ☐ `extension/metadata.json` (uuid `pipedeck@jvandenbos.github.io`, shell-version ["50"]),
  `extension.js`, `stylesheet.css`, optional `prefs.js` (none needed for v1).
- ☐ D-Bus proxy to `dev.pipedeck.Daemon1` (introspection XML embedded in the extension so it
  works before the daemon starts), properties cached, `Changed` → refresh.
- ☐ Quick Settings item: Output / Input / Notifications radio sections + per-app sliders (cubic).
- ☐ Robust to daemon absence/restart; `gjs -m` parses.

## Phase 3 — packaging  — agent: Haiku
- ☐ `packaging/pipedeckd.service` (systemd user unit), `install.sh` (builds with cargo, installs
  binaries to `~/.local/bin`, unit to `~/.config/systemd/user`, extension to
  `~/.local/share/gnome-shell/extensions/<uuid>`, enables extension via gsettings if absent),
  `uninstall.sh`, `Makefile` wrapping the docker commands from CLAUDE.md, `README.md`, `.gitignore`.

## Phase 4 — integration on chronos  — main session only
- ☐ rsync repo → chronos, `cargo build --release`, run daemon under systemd user unit.
- ☐ Acceptance tests SPEC §5 (1–3, 5) via CLI. (4 waits for Jan at the desk.)

## v1.1 (later)
- ☐ EQ via filter-chain (SPEC §2.5), preset picker in panel, AutoEq importer.
- ☐ Optional null sink `pipedeck.notifications`.

## Handoffs / notes between agents
(append here)
