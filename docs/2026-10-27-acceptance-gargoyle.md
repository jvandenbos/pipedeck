# Acceptance run — gargoyle, 2026-10-27 (SPEC-UI §9, daemon 1.4.0 user-level install)

Host: ThinkPad T14 Gen 1 AMD, Omarchy 4.5, PipeWire 1.6.9, WirePlumber 0.5.18, ALC257 on the
UCM profile (one speaker sink, no headphones plugged during the run — 22 not exercised).
Capture method as SPEC-UI §0: `pw-record` with `node.autoconnect=false` linked to the sink's
`monitor_*` ports, 100 Hz tone via `pw-play --target <sink>`, RMS in pure Python. The sink was
hardware-muted through PipeDeck's route mute during the EQ tone, so the test was silent.

| # | Result | Evidence |
|---|---|---|
| 20 | **PASS** | `+12 dB low shelf @400 Hz` on the real speaker sink: RMS 1390.1 → **5493.9** (×3.95 = +11.9 dB) → 1390.1 after `eq set off`. `pw-link -l` showed `pipedeck.eq.<sink>.out` feeding the sink; the default sink stayed the ALSA sink (`wpctl status`); 0 `pipedeck.eq` links after off. |
| 20b | **PASS** | With a preset active, `systemctl --user restart wireplumber`: the chain came back on its own (4 EQ links, status `{eq: Flat}`) on the sink's new node id (58 → 80). Suspend/resume not exercised. |
| 23 | **PASS** | Stream routed to a null sink `pdnull`: `-> pdnull`. Sink destroyed: `-> pdnull on: alsa_output…Speaker__sink` (requested kept, observed = default). Sink recreated: `-> pdnull` with no further call. |
| 24 | **PASS** | `set-notify pdnull` then a `media.role=event` stream → landed on `pdnull`. Manual `SetStreamTarget` to the speaker sink, then notify re-set → stream stayed on the speaker (manual precedence). |
| 21 | **PASS** | Daemon stopped: the window showed "PipeDeck daemon not running" + Start, red status dot, stayed alive. `StartServiceByName dev.pipedeck.Daemon` (the Start button's call) → unit active, window refilled with the real graph. |
| 22 | not exercised | no headphones plugged in during the run; the UCM profile exposes one speaker sink with one route. The place-flattening logic is covered by the offscreen tests. |
| 25 | **PASS** | 40 key presses at 50 Hz on a focused slider → `busctl monitor`: **30 SetVolume in 1.504 s (19.9/s)**, 0 SetMute. Level restored afterwards. |
| 26 | **PASS** | see 21: no crash, no queued writes replayed (the daemon came back with new node ids and the window re-read them). |
| 27 | partial | the window follows the live theme (rendered in the current purple accent, not the default yellow of the README shots); `omarchy theme set` / `omarchy language` were not run on Jan's desktop. Danish checked offscreen: no clipping. |
| 28 | **PASS** | `ui/bin/test` 55/55 three times; `makepkg -fd` builds `pipedeck-1.4.0-1-x86_64.pkg.tar.zst` with `check()` green; `pacman -U` is Jan's step (JAN-TODO). The launcher entry is live from `ui/bin/install-user`. |

Cleanup verified after each block: no `pdnull` nodes, notification sink back to `<default output>`,
0 EQ links, default sink and its 80 % volume unchanged, test preset removed from
`~/.config/pipedeck/eq/`.

Earlier the same night, with the v1.3.0 tree on a null sink: 4633.5 → 18313.2 → 4633.5 (SPEC-UI §0).
