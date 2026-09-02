# PipeDeck — Claude instructions

Read `SPEC.md` first; it is the contract. `PLAN.md` is the work breakdown and status.

## Hard rules
- **Never SSH to chronos (or any fleet host) from a subagent.** Only the main session deploys and
  runs integration tests there. Subagents build and unit-test in the Docker image only.
- **Platform libraries only.** The daemon talks to PipeWire through libpipewire (pipewire-rs) and
  WirePlumber metadata. It never shells out to `wpctl`/`pactl`/`pw-cli`. No EasyEffects, no
  other extensions, no other apps as dependencies.
- Stay in your lane: the daemon agent owns `crates/`, the extension agent owns `extension/`,
  packaging owns `packaging/`, `install.sh`, `Makefile`, `README.md`. Don't edit another lane's
  files; leave a note in `PLAN.md` § Handoffs instead.
- Don't `git commit`; the main session commits. Don't touch `~/.config` or system dirs on the Mac.

## Build & test (from the repo root, on the Mac)
```bash
# one-time: docker build -t pipedeck-dev dev/     (already built)
docker run --rm -v "$PWD":/src -v pipedeck-cargo:/cargo pipedeck-dev cargo build --workspace
docker run --rm -v "$PWD":/src -v pipedeck-cargo:/cargo pipedeck-dev cargo test --workspace
docker run --rm -v "$PWD":/src -v pipedeck-cargo:/cargo pipedeck-dev cargo clippy --workspace -- -D warnings
docker run --rm -v "$PWD":/src -v pipedeck-cargo:/cargo pipedeck-dev cargo fmt --check
docker run --rm -v "$PWD":/src pipedeck-dev sh -c 'cd extension && for f in *.js; do gjs -m --help >/dev/null; node -e 0 2>/dev/null; gjs -c "import(\"./$f\")" 2>&1 | head -5; done'
```
The image has no PipeWire runtime — anything touching a live graph is tested on chronos by the
main session. Write pure logic so it is unit-testable without a graph (parse/format helpers,
matching rules, state diffing, D-Bus tuple mapping).

## Target facts (chronos)
Ubuntu 26.04.1 · GNOME Shell 50.1 (Wayland) · PipeWire 1.6.2 · WirePlumber 0.5.13 · Rust 1.93.1
(apt) · outputs: motherboard "Starship/Matisse HD Audio" analog + "TU104 HD Audio" HDMI.
