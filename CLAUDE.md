# PipeDeck — Claude / agent instructions

Read `SPEC.md` first; it is the contract. `PLAN.md` is the work log and the live-test notes.
`CHANGELOG.md` is the release history.

## Rules
- **Platform libraries only.** The daemon talks to PipeWire through libpipewire (pipewire-rs) and
  WirePlumber metadata, and loads `libpipewire-module-filter-chain` in-process. It never shells
  out to `wpctl`/`pactl`/`pw-cli`; no other end-user audio project is a dependency.
- **Subagents never SSH to the integration host.** They build and unit-test in the Docker image
  only; the maintainer's main session deploys and runs the live acceptance tests.
- Lanes: `crates/` (daemon + CLI), `extension/` (GNOME Shell), `packaging/` + `install.sh` +
  `Makefile` + `README.md` + `presets/`. Stay in your lane; leave cross-lane notes in
  `PLAN.md § Handoffs`.
- Agents don't `git commit`; the main session does.

## Build & test
```bash
docker build -t pipedeck-dev dev/          # once
make check                                 # build + test + clippy -D warnings + fmt + presets
make ext-check                             # gjs syntax check of extension/*.js
```
The image has no PipeWire runtime — anything touching a live graph is tested on a real host
(`./install.sh` there, then the SPEC §5/§6.3/§7.5 acceptance lists). Keep pure logic in
graph-free modules (`route.rs`, `eq.rs`, `meta.rs`, `config.rs`, …) so it is unit-testable.

## Hard-won facts (see PLAN.md for the full stories)
- Registry global props are a **whitelist**; custom node props only arrive in the node `info`.
- Device `subscribe_params` never delivers Route changes on PipeWire 1.6 — re-enumerate from
  the device `info` event (what pipewire-pulse does).
- ALSA volume lives in the device `Route` param, not the node `Props`.
- WirePlumber routes a sink-monitor capture through a smart filter's *monitor* (pre-EQ); to
  measure post-EQ, capture with `node.autoconnect=false` and `pw-link` to `<sink>:monitor_*`.
- User services die with the last login session when `loginctl` lingering is off; test inside
  one SSH session.
