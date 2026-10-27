# 2026-10-27 — adversarial review of the Phase 9 build (69f5bd9..3c97098)

Two independent reviewers, same diff, before the fix pass. The fixes landed in the commits that follow 3c97098; see CHANGELOG 1.4.0.

## Codex (brief: `codex-brief3.md`)

1. **CONFIRMED — P1 — `ui/src/backend.cpp:456`**: Volume deduplication suppresses legitimate writes after another controller changes the level. **Scenario:** Set a sink to 50%, change it to 80% with a media key, then request 50% again; `lastSentLinear` still matches 50%, so no D-Bus call leaves and the slider eventually returns to 80%. **Fix:** Deduplicate against pending writes or current observed state, and invalidate historical write values when external changes arrive.

2. **PLAUSIBLE — P1 — `ui/src/daemon.cpp:177`**: Calls address the well-known service name, allowing an old-generation node write to reach a replacement daemon. **Scenario:** The daemon restarts between the UI’s last snapshot and delivery of `NameOwnerChanged`; a slider write addressed to `dev.pipedeck.Daemon` reaches the new owner and changes an unrelated node with the reused ID. **Fix:** Address snapshots and writes to the tracked unique owner name so replacement cannot redirect an already-issued call.

3. **CONFIRMED — P1 — `ui/bin/install:10`**: The advertised package installation entry point cannot work because `packaging/arch/PKGBUILD` is absent. **Scenario:** Following the installation instructions runs `makepkg -fsi` in a directory containing only the two service files, so package creation fails before building anything; the metadata required by SPEC-UI §7 is also absent. **Fix:** Supply the pinned, checksummed PKGBUILD, install script, and `.omarchy/package.json` required for the package submission.

4. **CONFIRMED — P2 — `ui/src/backend.cpp:104`**: Missing-method capability state survives owner changes and permanently disables controls supported by a replacement daemon. **Scenario:** `SetEq` returns `UnknownMethod` on an older daemon, then that daemon is upgraded and restarted; `m_missingMethods` retains `SetEq`, so Equalizer stays hidden despite the new snapshot exposing its properties. **Fix:** Clear the missing-method cache whenever the daemon owner changes.

5. **CONFIRMED — P2 — `crates/pipedeckd/src/eq.rs:445`**: `PresetDirs::find()` uses different selection rules from the daemon’s merged preset loader. **Scenario:** A malformed user `bass.toml` shadows a valid system preset on disk; the daemon skips the malformed file and offers the system preset, but `pipedeck eq show bass` selects the malformed user file and fails; a valid `Bass Boost.toml` likewise loads as `bass-boost` but cannot be found by that ID. **Fix:** Share a resolver that applies the loader’s filename normalization, validation, and precedence rules while retaining the selected source path.

6. **CONFIRMED — P2 — `ui/src/Main.qml:49`**: Keyboard scrolling mixes content-local coordinates with Flickable coordinates and can leave focused controls clipped. **Scenario:** Tab to a control whose bottom falls within the final 16 pixels of the viewport; `mapToItem(content, ...)` excludes `content.y = 16`, so the visibility calculation incorrectly considers it visible. **Fix:** Map focused-item bounds into `flick.contentItem` coordinates before comparing them with `contentY`.

7. **CONFIRMED — P2 — `ui/src/backend.cpp:168`**: Successful snapshot calls never clear a previous D-Bus error, violating SPEC-UI §5. **Scenario:** One `GetAll` fails transiently, then a subsequent refresh succeeds and restores normal models; the old failure remains in the status line indefinitely unless a successful write occurs. **Fix:** Clear `lastError` on successful snapshot replies before applying the snapshot, allowing newly resolved errors to surface afterward.

## Claude Opus reviewer

Baseline: cargo test (24+136) and clippy -D warnings green; `ui/bin/test` 33/33 three times, and once under 2×nproc load.

1. **should-fix — `pw.rs` set_stream_target("") + `links.rs` requested_target.** "Follow default" cannot override a stream that asked for a sink when it connected (`PULSE_SINK`, `pw-play --target` set the node property `target.object`); deleting the metadata key lets WirePlumber's `find-defined-target.lua` fall back to the node property, and the picker snaps back. Fix: for "" write `target.object = "-1"` into the metadata (the Lua treats `-1` as "no target"), and map `-1` to "" in `requested_target`. CONFIRMED against WirePlumber 0.5.18's script.
2. **should-fix — `links.rs` requested_target.** The `-1` sentinel is reported raw; the deprecated `node.target` property and `target.node` metadata (node ids, not serials) are ignored though WirePlumber honours both. PLAUSIBLE.
3. **should-fix — `ui/src/backend.cpp` stream rows.** Against a 1.3.0 daemon the UI reads `target_name` as a request, but 1.3.0 reports the live link (with EQ on: `pipedeck.eq.<sink>`, shown as absent). Fix: when `StreamSinks` is missing, treat `target_name` as observed and collapse EQ names. CONFIRMED.
4. **should-fix — `backend.cpp` setOwner.** `m_missingMethods` never cleared on owner change; a control hidden by an old daemon stays hidden after the upgrade. CONFIRMED.
5. **should-fix — `LevelSlider.qml` + `backend.cpp` setVolume.** A press/release with no movement still sends SetVolume, and with the 150 % toggle off it writes the clamped value — violates §4.1 "never writes a clamp back". CONFIRMED.
6. **should-fix — `DeviceEntry.qml`, `NotifySection.qml`.** Radios and the auto-mute switch keep showing a choice the daemon rejected (the `checked:` binding is broken by the click). Reproduced with a QML probe. CONFIRMED.
7. **should-fix — `backend.cpp` first GetAll.** A failed GetAll leaves the window in "connecting" forever when the graph then stays idle. Fix: retry with backoff while the owner generation is unchanged. PLAUSIBLE.
8. **should-fix (tests).** `QVERIFY(qWaitFor(== 20, 250))` cannot fail (meant "stays 20"); the fake daemon encodes with the same operators the proxy decodes with, so same-typed field swaps are invisible. Timing: `releasedSliderOutlivesOlderEchoes` and the ≥15 sends/s assertion could flake on a slow chroot. CONFIRMED (the two defects).
9. **nit — `eq.rs` PresetDirs::find vs load; `service.rs` rescan_presets.** `find` returns the first file that exists while `load` skips a broken user file and serves the system one; the rescan doc says an unreadable dir keeps the previous library, but the code replaces it. CONFIRMED.
10. **nit — `backend.cpp` volume timers.** `deleteLater()` without `stop()`; a queued timeout can flush a re-created entry for a reused id. PLAUSIBLE.
11. **nit — `main.cpp` / `singleinstance.cpp`.** The second launch waits ≤1 s for "ok" but the primary only answers once `app.exec()` runs; a slow cold start or a shutting-down primary makes the keybind launch exit 1 with no window. PLAUSIBLE.
12. **nit — `daemon.cpp`.** Messages go to the well-known name with auto-start allowed: a trailing GetAll after `systemctl --user stop pipedeckd` re-activates the daemon. PLAUSIBLE.
13. **nit — `pw.rs` prop_target.** Set when the property appears, never cleared when it disappears from node info. CONFIRMED.
14. **nit — `backend.cpp` slider moves.** Every 60 Hz move calls `rebuildModels()` for all three models. PLAUSIBLE.

Silent departures from SPEC-UI: §4.3 vs §6.1 contradict on "Follow default" (item 1); §5 "hides that control" only after a failed click and never returns (item 4); §4.5 hint names only the user preset dir; §4.1 clamp rule (item 5); §2 member counts stale; §5 "re-reads only the properties it uses" — the UI always does a full GetAll; the daemon still reports Version 1.3.0 at review time.
