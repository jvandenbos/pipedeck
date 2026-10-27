# Omarchy front end — design review, 2026-10-26

The brief Claude (Fable 5.1) sent to Codex (codex-cli 0.164.0, read-only sandbox over this repo and `/usr/share/omarchy`), followed by Codex's reply verbatim. The decisions taken from it are in `SPEC-UI.md`.

---

## Brief

### Design review request: an Omarchy (Arch/Hyprland) front end for PipeDeck

You are reviewing a design before it is written up as a spec. Be a sceptical senior engineer:
rank your points by how much they would change the design, be concrete, and say when you
agree so we can stop debating it. Answer in markdown, under ~1500 words. You may read the
code under `pipedeck-build/` (the existing project) and `/usr/share/omarchy/` (the desktop
we are targeting, Omarchy 4.5.0rc1) to check claims.

#### Context

- **PipeDeck** (github.com/jvandenbos/pipedeck, v1.3.0) is a SoundSource-style audio control
  for PipeWire on GNOME: a Rust daemon `pipedeckd` (libpipewire via pipewire-rs, zbus D-Bus
  interface `dev.pipedeck.Daemon1`, systemd user service), a Rust CLI `pipedeck`, and a GNOME
  Shell Quick Settings extension. Read `pipedeck-build/SPEC.md` for the daemon's capabilities:
  default sink/source, per-stream volume/mute, **per-stream routing** (`SetStreamTarget`, in
  the API but never exposed in a UI), **notification-sink routing** (`media.role=event`
  streams go to a chosen sink), **ports** (headphones/line-out as ALSA Route params), **EQ**
  (parametric filter-chain inserted as a WirePlumber *smart filter*, AutoEq import), ALSA
  auto-mute handling, loudness safety (independent volume/mute writes; level cap after a
  port switch). `pipedeck-build/CLAUDE.md` lists hard-won PipeWire facts.
- **Verified today on the target machine** (ThinkPad T14 Gen 1 AMD, Omarchy 4.5.0rc1,
  PipeWire 1.6.9, WirePlumber 0.5.18, Hyprland 0.56, Qt 6.12, Quickshell 0.3.2): the daemon
  and CLI build unmodified with cargo 1.98 (69 s) and run: `pipedeck status/outputs/ports/
  automute` list the ALC257 speaker sink, two mic sources, the routes. UCM profile names
  (`HiFi__Speaker__sink`); the headphone jack will most likely surface as a *separate sink*,
  not a port of the speaker sink. EQ not yet tested here (no presets installed).
- **Omarchy already ships an audio surface**: omarchy-shell (Quickshell/QML) bar widget
  `omarchy.audio` (`/usr/share/omarchy/shell/plugins/panels/audio/Panel.qml`): master
  slider, output picker, input picker, per-app volume. Keybind SUPER+CTRL+A toggles it. It
  uses `Quickshell.Services.Pipewire` (nodes, volumes, mute, default sink/source,
  peak meters) plus shell scripts (`omarchy-audio-*`, pactl/wpctl). Quickshell's Pipewire
  module has **no** metadata writes (routing), no device Route params (ports), no module
  loading (filter-chain) and Quickshell has no generic D-Bus QML type. omarchy-shell has a
  third-party plugin system (`~/.config/omarchy/plugins/<id>/manifest.json`, kinds
  `bar-widget|panel|overlay|menu|service`, entry points are QML Items, `omarchy-shell shell
  toggle <id>` IPC). Plugins are unsandboxed QML; commands run via `Quickshell.Io.Process`.
- **Omarchy's own DSP precedent**: per-laptop *speaker tunings* are a PipeWire filter-chain
  hosted in a separate `pipewire -c omarchy-speaker-tuning.conf` user service as a *fronting
  sink* that becomes the default; `omarchy-audio-output-sink` resolves volume keys through
  the DSP sink to the physical one; `omarchy-audio-sink-availability` hides the fronted
  sink. Their docs say they **abandoned WirePlumber smart filters**: on PipeWire 1.6.8 /
  WirePlumber 0.5.15 "the graph loads and links correctly as a smart filter and then passes
  audio through unprocessed", root cause unknown. PipeDeck's EQ *is* a smart filter,
  verified only on PipeWire 1.6.2 / WirePlumber 0.5.13. No tuning matches the T14.
- **Omarchy app conventions** (the `omarchy-app` guide the user installed): one-window Qt
  Quick apps in C++ with qmake6 (`<name>.pro`, `src/main.cpp`, `backend.{h,cpp}` QObject
  with Q_PROPERTY/Q_INVOKABLE, `Main.qml`, Material dark style with the theme accent read
  live from `~/.local/state/omarchy/current/theme/colors.toml`, Hyprland rounding via
  hyprctl, i18n via qsTr + `~/.config/omarchy/language`), name = binary = package =
  .desktop = Wayland app id, installed via PKGBUILD/makepkg, keyboard first (`?` help, `Q`
  quit), headless subcommands with `--json` so agents can drive it (`<name> open` for the
  GUI). Reference apps: Hype, Omacut, Monologue, Signed.
- The user: one person, one laptop first (then the same on other Omarchy boxes). Values
  boring tools, deleting code, no dependency without a one-sentence reason, platform
  libraries only (no EasyEffects/pavucontrol dependency). The UI does **not** need to mimic
  SoundSource.

#### Proposed design (challenge this)

1. **Reuse `pipedeckd` unchanged as the engine.** It is the rules engine (routing on new
   streams, EQ re-apply on hotplug, safety caps) that must run with no window open, and it
   just proved portable. Add an Arch PKGBUILD + the user unit; nothing GNOME in it.
2. **New front end = a standalone Qt Quick app per the Omarchy app conventions**, talking to
   the daemon over **QtDBus** (typed, live `PropertiesChanged`, no polling), *not* an
   omarchy-shell plugin. Reasons: the shell plugin API is RC-stage and would force
   CLI-over-Process glue for everything the Quickshell Pipewire module lacks (which is
   exactly our feature set); a Qt app is testable offscreen with Qt Test, packaged, and
   could be upstreamed to omarchy-pkgs. Optional later: a 20-line bar-widget plugin whose
   click toggles the app. The stock `omarchy.audio` panel stays for quick volume.
3. **Name/binary**: the Omarchy package is `pipedeck`; the Qt binary is also `pipedeck` and
   implements the CLI surface (`pipedeck status --json`, `set-output`, `eq set` …, Hype-style)
   as thin D-Bus calls, with bare `pipedeck`/`pipedeck open` launching the window. The Rust
   CLI stays for GNOME/chronos until parity, then is retired; its AutoEq importer moves into
   the daemon as a D-Bus method so both stay thin. Alternative: GUI binary `pipedeck-ui`,
   keep the Rust CLI, no refactor.
4. **One repo** (`pipedeck`), new lane `omarchy/` beside `crates/` and `extension/`, one
   CHANGELOG; the D-Bus interface co-evolves with its clients. Alternative: new repo.
5. **Window**: floating, keyboard-driven, sections Outputs (one row per sink *and* per
   selectable port, default marked, volume+mute), Inputs, Apps (volume, mute, **route-to-
   output picker** per stream), Notifications sink, Equalizer (preset per output, import
   AutoEq via xdg portal file chooser, off), Safety (switch cap, allow >100 %). Opened by a
   keybind the user adds (e.g. SUPER+CTRL+SHIFT+A) and from the launcher; Hyprland
   float/pin/size rule documented, not installed.
6. **Feature set v1.0 = what the daemon already does** (above). **v1.1**: peak meters (in-app
   libpipewire capture streams while visible, or a daemon `Levels` signal?), a device-switch
   level cap (today only port switches are capped; on a laptop a BT headset appearing is the
   loud event), a bundled "laptop speakers" EQ preset, the bar-widget stub. **Non-goals**:
   per-app effects, LV2/AU plugin hosting UI, recording, Super-Volume-Keys (Omarchy's keys
   already drive any sink), card-profile/BT-codec switching (Omarchy's BT panel owns it).
7. **EQ mechanism risk**: keep PipeDeck's smart filter *if* an acceptance test on this box
   proves audio is actually processed (capture post-EQ with `node.autoconnect=false` linked
   to the sink monitor, compare spectra against a +12 dB shelf). Fallback: Omarchy's
   fronting-sink pattern, which then needs the fronted sink hidden from Omarchy's panel
   (its hint file `/usr/share/omarchy-platform/audio.json` is package-owned) and
   `omarchy-audio-output-sink`-compatible stream naming so volume keys still hit the
   physical sink.
8. **Known interaction**: any capture stream we create for meters will make Omarchy's
   Microphone widget show "mic in use" (it only excludes Quickshell's own
   `quickshell-peak-monitor` by name). Options: meters only while the window is open,
   monitor-only streams, or an upstream patch to Omarchy's `AudioNodesModel.js` to skip
   nodes with a generic marker property.

#### Questions

A. Is the standalone-Qt-app choice right, or should this be an omarchy-shell bar-widget
   plugin that *replaces* `omarchy.audio` (one icon, one popup) despite the Process glue?
   Or a plugin for the popup *and* a Qt app for the deep settings?
B. Daemon reuse vs. folding libpipewire into the Qt app (no D-Bus, no Rust): is there a
   real argument for the rewrite?
C. Naming/CLI unification (point 3): worth it, or `pipedeck-ui` and move on?
D. One repo vs two.
E. What is missing for a laptop user: Bluetooth headset appear/disappear behaviour,
   suspend/resume, dock HDMI, Flatpak app stream names, pipewire-pulse apps, the UCM
   separate-sink headphone jack (device-switch cap, remembered per-device volume/EQ)?
F. Meters: in-app streams vs daemon-provided levels; how to avoid the mic-in-use problem.
G. Anything in Omarchy's conventions or Hyprland (layer-shell vs xdg toplevel, window
   rules, focus/keyboard handling for a popup-like window) that makes the floating-window
   plan worse than it looks?
H. What would you cut from v1.0?

---

## Codex's review

**Keep the daemon, build one Qt Quick front end, retain the Rust CLI, and use one repo.** The major changes I’d make concern laptop behaviour and release scope. Ranked by design impact:

### 1. Make laptop switching behaviour a v1 contract

“Reuse the daemon unchanged” is a good starting point, not a release requirement. Building and enumerating devices proves portability; it does not prove the policies work on this hardware.

The current cap is narrower than proposed: [`set_port()`](pipedeck-build/crates/pipedeckd/src/pw.rs) records a pending cap only for switches **PipeDeck itself issues**. Automatic jack changes, WirePlumber default changes, and selecting another sink are outside that protection.

Move device-switch handling into v1, or explicitly ship without claiming general switch protection. Define these cases before implementation:

| Event | Decision needed |
|---|---|
| Headphones/BT appear | Automatically switch, or leave WirePlumber’s choice alone? |
| Selected headphones disappear | Follow the default, or keep explicitly routed streams silent? |
| Dock HDMI returns | Restore a prior preference, or retain the current output? |
| Suspend/resume | Recover routing and EQ without overriding newer user choices |
| Another controller changes output | Observe it; specify whether the cap applies |

I favour leaving device selection and ordinary volume restoration to WirePlumber initially. PipeDeck owns explicit routing, EQ and its documented cap policy. Avoid introducing a second complete device-preference engine.

A reactive cap can still allow a loud transient before it runs. Specify it as a **restored-level cap**, unless testing establishes stronger protection.

### 2. Treat EQ as a separate go/no-go gate

I agree with testing actual processing, and with the explicit post-EQ monitor links. Graph topology alone is insufficient.

Run the same deterministic signal with EQ off/on, allow headroom for the +12 dB shelf, and compare the frequency-dependent gain. Repeat after sink recreation, daemon restart and suspend/resume. Verify stereo channels and that disabling EQ restores the expected graph.

**Do not make the fronting-sink fallback an automatic part of v1.** It changes the integration substantially:

- Default and explicit stream routes must select the processed path.
- Notification routing must also traverse it.
- Internal processing streams must stay out of Apps.
- Physical-volume resolution must work when idle as well as playing.
- Machines with existing Omarchy speaker tuning need an explicit coexistence policy.

The existing [`omarchy-audio-output-sink`](/usr/share/omarchy/bin/omarchy-audio-output-sink) uses stream-name heuristics and falls back to the selected sink when unresolved. Naming compatibility needs an acceptance test.

Do not commandeer `/usr/share/omarchy-platform/audio.json`. If smart filters fail, defer EQ or implement the fallback as a separate, reviewed engine change with an appropriate Omarchy integration mechanism.

Also cut the generic “laptop speakers” preset. Speaker correction belongs to a measured hardware model.

### 3. Define routing semantics before exposing “Apps”

The existing API controls **streams**, not persistent application rules. One browser may have several streams; a restarted stream gets a new identity.

For v1:

- Label the section “Playback streams”, or clearly explain multiple rows per app.
- Show app name plus media name, with fallbacks for missing metadata.
- Offer “Follow default” explicitly.
- State that manual assignments last for the stream’s lifetime.
- Define precedence between a manual assignment and notification routing.
- Distinguish requested destination from observed destination where they differ.

The current `Streams` tuple provides one `target_name`; it cannot fully represent that last distinction. Do not present successful metadata submission as proof that audio moved.

Check native PipeWire, pipewire-pulse and Flatpak clients. Missing or unstable app names should affect presentation, not break controls. Persistent app rules can wait until there is a stable identity policy.

### 4. Standalone Qt is the right v1 choice — settle A and B

**Agreed: Qt Quick + QtDBus, with the daemon retained.** There is no compelling rewrite argument here. Moving libpipewire into the GUI either loses background policy or turns the GUI executable into another daemon arrangement while reimplementing tested code.

The reason to choose Qt is direct access to the complete API and separation from shell lifetime. Process glue is an inconvenience, not proof that a plugin cannot work.

Keep the stock panel. Do not build both a custom popup and a deep-settings app in v1: that adds another interaction surface and lifecycle. Revisit a bar launcher after using the app.

Qt Test is useful for models and interaction; offscreen tests cannot validate real audio routing, compositor focus or portal behaviour.

### 5. Keep `pipedeck-ui`; keep one repo — settle C and D

Replacing a working CLI buys almost nothing for this user. It adds parity work, command compatibility risk and a display-independent Qt startup path.

Use:

- `pipedeck`: existing Rust CLI.
- `pipedeckd`: existing daemon.
- `pipedeck-ui`: Qt executable, desktop ID and optional separate package.

The app guide’s naming convention should not force a rewrite. Add missing `--json` coverage to the existing CLI as needed.

Keep one repo and changelog, with the front end under `omarchy/` or `ui/`. Use the checked-in [D-Bus XML](pipedeck-build/crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml) as the binding contract. Co-location does not guarantee synchronized installed versions: preserve interface compatibility and handle unavailable methods.

### 6. Change the device layout and specify client recovery

Do not render sinks and ports as equivalent peer outputs. A sink owns volume/default state; ports select routes within it. Use one sink row with a port selector where meaningful. Separate UCM headphone sinks then fit naturally.

Add a client lifecycle contract:

- Asynchronous D-Bus calls and visible errors.
- Initial snapshot plus live changes.
- Daemon disappearance/reappearance and complete model replacement.
- Coalesced slider writes and reconciliation with observed values.
- Cancellation of pending actions when their node disappears.

Numeric IDs are ephemeral. Never replay queued slider writes across reconnection.

Packaging also needs more than copying the unit: the existing [service](pipedeck-build/packaging/pipedeckd.service) points to `%h/.local/bin/pipedeckd`; an Arch package needs its installed path and working D-Bus activation.

“Allow >100%” is not an existing daemon setting in the interface. Decide whether it merely expands this UI’s slider or enforces a shared policy.

### 7. Cut meters from v1; prefer in-app measurement later — F

Meters do not justify a daemon API yet. Later, create measurement streams only while the relevant view is visible.

Moving capture into the daemon does **not** fix the microphone indicator. The current [Microphone widget](/usr/share/omarchy/shell/plugins/bar/widgets/Microphone.qml) filters by stream classification, mute and explicit exclusions; it does not establish that a stream records a physical microphone. Monitor-only capture can therefore still trigger it.

An upstream marker-based exclusion is reasonable if Quickshell exposes the property needed to inspect it. Do not impersonate Quickshell’s excluded node names.

### 8. Use a normal window — G and H

Use an xdg toplevel. The proposed controls and file chooser benefit from ordinary focus and window management. Layer-shell would introduce panel-like keyboard and stacking behaviour without a clear need.

Document optional float/size rules; make pinning optional. Specify single-instance activation and test activation through the launcher and keybind. Escape should dismiss transient controls before closing anything; `Q` must not quit while typing in a field.

**My v1:** outputs/inputs, nested ports, stream volume/mute/routing, notification target, existing cap controls, and EQ preset/off only if validated. Cut meters, bar integration, CLI replacement, bundled generic EQ and in-app AutoEq import. Keep import in the existing CLI initially. That delivers the missing capabilities without turning this into a desktop audio-stack redesign.

---

## Pre-build quick review (same day, Codex)

### Brief

Quick pre-build review of SPEC-UI.md in this repo (PipeDeck's Qt front end for Omarchy). You reviewed the design earlier today (docs/2026-10-26-ui-design-review.md); the decisions are folded into SPEC-UI.md. We build it in the next hour with coding agents. I want the top implementability gaps only — things an implementer would hit or get wrong — ranked, under 450 words, no restating what is fine.

Facts since your last pass:
- Build host is this Omarchy 4.5 box: Qt 6.12 (qt6-base, qt6-declarative, qt6-svg, qt6-wayland), NO qt6-tools, so no qdbusxml2cpp and no lupdate/lrelease. Plan: a hand-written QDBusAbstractInterface proxy + a unit test that parses crates/pipedeckd/dbus/dev.pipedeck.Daemon1.xml and asserts the proxy covers every method/property/signal with matching signatures. Translations are JSON per the Omarchy app guide (~/.claude/skills/omarchy-app/SKILL.md + templates.md), not .ts.
- sudo needs a password, so the package is built with makepkg but installed by Jan; the session installs user-level (~/.local/bin, ~/.local/share/applications, ~/.config/systemd/user, ~/.local/share/dbus-1/services) to run it today.
- Target: pass omacom/omarchy-pkgs PR gates (README in /tmp/.../scratchpad/omarchy-pkgs/README.md; hype/omacut PKGBUILDs there too): release-tarball source + sha256, arch=(x86_64 aarch64), options=('!debug'), complete makedepends for a clean-chroot build (rust, clang, pkgconf, qt6-base, qt6-declarative), .install for icon cache, .omarchy/package.json with a GitHub release watch. PKGBUILD will live at packaging/arch/ (whole project = one package `pipedeck`), desktop file + svg under ui/pkgbuild/.
- Presets: the pacman package installs bundled presets to /usr/share/pipedeck/presets; the daemon today reads only ~/.config/pipedeck/eq. Plan: daemon also scans $XDG_DATA_DIRS/pipedeck/presets read-only, user dir wins on id collision (additive, §6).
- Keybind decided: SUPER+CTRL+SHIFT+A.

Look especially at: §5 reconnect/coalescing semantics vs what QtDBus actually gives you (PropertiesChanged on org.freedesktop.DBus.Properties — check whether the daemon emits it or only its own `Changed` signal: read crates/pipedeckd/src), §4.3 observed-vs-requested derivation from Link globals (edge cases: EQ filter in between — the stream links to pipedeck.eq.<sink>, not the sink; what should StreamSinks report?), §4.1 cubic slider vs the daemon's volume units (read SetVolume/Devices in the XML and src), the single-instance socket, and anything in §7/§8 that a clean chroot build would reject.

### Codex reply (verbatim)

1. **§4.3/§6: `Streams.target_name` is not requested state today.** In [pw.rs](/home/janv/Work/pipedeck/crates/pipedeckd/src/pw.rs:275), `resolve_target()` returns the first live link’s destination **before** consulting target metadata. Consequently, “Follow default” reports a sink, and fallback hides the absent requested sink. Split intent from observation explicitly; changing this projection is required alongside adding `StreamSinks`. For observed routing, collapse PipeDeck’s known EQ chain to its physical destination using the daemon’s EQ-instance mapping; reporting `pipedeck.eq.<sink>` would reference a hidden picker entry. Define unlinked as `""`, deduplicate channel links, and specify deterministic handling of multiple destinations.

2. **§5: specify one refresh/reconnect state machine.** [service.rs](/home/janv/Work/pipedeck/crates/pipedeckd/src/service.rs:748) emits **both** standard `PropertiesChanged` and custom `Changed`, including separate property emissions per graph update. Subscribe before the initial snapshot; explicitly connect the standard Properties interface. Funnel notifications into one coalescer with one refresh in flight and a trailing dirty flag. Tag replies with an owner generation and reject obsolete callbacks, including direct owner replacement. “Drop pending writes” must distinguish unsent work from already dispatched calls. Define release ordering too: an older observed value must not overwrite the final slider request before its echo arrives.

3. **§2/§7/§8 still direct agents toward the superseded build.** Fold in the supplied decisions before handoff: handwritten proxy plus XML coverage test; JSON translations; `packaging/arch/PKGBUILD`; `makepkg` without installation; separate user installation; XDG preset scanning instead of first-run copying. Pin the release source/checksum, architectures, `!debug`, icon hook, watch metadata, and complete dependencies. Specify `QT += dbus network …`; the socket needs Qt Network. The fake-bus test target must exclude production `main.cpp` rather than literally compiling every source.

4. **§5/§8: define QtDBus decoding, not just signature coverage.** Arrays of structs such as `a(usssbbdbs)` need registered tuple/list metatypes and D-Bus stream operators, or explicit `QDBusArgument` decoding. Require asynchronous `Properties.GetAll/Get` and wire-level fake-daemon round trips; a proxy/XML comparison alone cannot verify decoding.

5. **§2: socket ownership and crash recovery are unspecified.** Add a lock around primary election/stale-socket recovery, bounded activation acknowledgement, and restore/show/raise behavior. Never unlink merely because `listen()` fails: Qt explicitly warns against removing a running instance’s socket. [Qt documentation](https://doc.qt.io/qt-6/qlocalserver.html)

6. **§4.1/§4.6: finish the volume contract.** Specify writes as `(percent/100)^3`: 50% → `0.125`, 150% → `3.375`; `PortSwitchCap` instead takes integer percentages. Define displaying externally observed overdrive with the toggle off without writing a clamp back.