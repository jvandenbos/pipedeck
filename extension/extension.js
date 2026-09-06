import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as QuickSettings from 'resource:///org/gnome/shell/ui/quickSettings.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Slider} from 'resource:///org/gnome/shell/ui/slider.js';

import {DaemonProxy, BUS_NAME, OBJECT_PATH} from './dbus.js';

// Volume convention, per SPEC §2.1/§2.4: linear 0.0-3.375 = 150 % cubic (daemon clamps),
// slider position is the cube root so the perceptual step size is even and
// matches GNOME's own overdrive-region slider feel.
const MAX_VOLUME = 3.375;
const MAX_SLIDER_VALUE = Math.cbrt(MAX_VOLUME);
const OVERDRIVE_START = 1.0;

// Debounce slider -> SetVolume to <=20/s (SPEC §2.4).
const SLIDER_DEBOUNCE_MS = 50;

const FALLBACK_APP_ICON = 'audio-x-generic-symbolic';

/**
 * Devices tuple -> object. Wire type a(usssbbdbs):
 * (id, name, description, kind, is_default, virtual, volume, mute, nick)
 * `nick` is a trailing field added on chronos after live testing (2026-09-01);
 * an older 9th-field-less daemon leaves it `undefined` here, so it falls
 * back to `description` -- callers should always read `.nick`, never
 * `.description`, when building a device label.
 */
function unpackDevice([id, name, description, kind, isDefault, isVirtual, volume, mute, nick]) {
  return {id, name, description, kind, isDefault, isVirtual, volume, mute, nick: nick || description};
}

/**
 * Streams tuple -> object. Wire type a(ussssdb):
 * (id, app_name, binary, media_name, target_name, volume, mute)
 */
function unpackStream([id, appName, binary, mediaName, targetName, volume, mute]) {
  return {id, appName, binary, mediaName, targetName, volume, mute};
}

/**
 * Ports tuple -> object. Wire type a(uussbb), per SPEC §6.1:
 * (node_id, route_index, name, description, available, active)
 */
function unpackPort([nodeId, routeIndex, name, description, available, active]) {
  return {nodeId, routeIndex, name, description, available, active};
}

/**
 * EqPresets tuple -> object. Wire type a(ss), per SPEC §7.3: (id, name)
 */
function unpackEqPreset([id, name]) {
  return {id, name};
}

/**
 * Eq tuple -> object. Wire type a(us), per SPEC §7.3:
 * (node_id, preset id or "") -- one row per output device.
 */
function unpackEq([nodeId, preset]) {
  return {nodeId, preset};
}

/**
 * AutoMute tuple -> object. Wire type a(ub), per SPEC §8.1:
 * (node_id, enabled) -- one row per sink whose card has an Auto-Mute Mode
 * ALSA control.
 */
function unpackAutoMute([nodeId, enabled]) {
  return {nodeId, enabled};
}

/** Map<nodeId, port[]> from a flat Ports array. `ports` may be null (v0.1 daemon, no Ports
 * property) — callers get an empty Map back, which makes every device render as v0.1 did. */
function groupPortsByNode(ports) {
  const map = new Map();
  if (!ports)
    return map;
  for (const port of ports) {
    let list = map.get(port.nodeId);
    if (!list) {
      list = [];
      map.set(port.nodeId, list);
    }
    list.push(port);
  }
  return map;
}

/**
 * GNOME Shell 46+ moved `Gio.DesktopAppInfo` to the platform-specific
 * `GioUnix` module; the old spelling still works but logs "has been moved to
 * a separate platform-specific library..." on every single call, which
 * spammed the journal on every menu rebuild. Resolve `GioUnix.DesktopAppInfo`
 * lazily via a dynamic import rather than a static one: a static
 * `import ... from 'gi://GioUnix'` would hard-fail this whole module's load
 * on a shell/build that lacks the GioUnix typelib (e.g. the syntax-check
 * container), where this dynamic form just rejects and the catch below
 * leaves the `Gio.DesktopAppInfo` fallback in place.
 */
let DesktopAppInfoCtor = Gio.DesktopAppInfo;
import('gi://GioUnix')
  .then(({default: GioUnix}) => {
    if (GioUnix?.DesktopAppInfo)
      DesktopAppInfoCtor = GioUnix.DesktopAppInfo;
  })
  .catch(() => {
    // GioUnix typelib not available here -- keep the Gio.DesktopAppInfo
    // fallback (functional, just noisier in the log).
  });

/** Best-effort app icon lookup; never throws, always returns a Gio.Icon or null. */
function lookupAppIcon(stream) {
  try {
    let appInfo = null;
    if (stream.binary)
      appInfo = DesktopAppInfoCtor.new(`${stream.binary}.desktop`);
    if (!appInfo && stream.appName)
      appInfo = DesktopAppInfoCtor.new(`${stream.appName}.desktop`);
    if (!appInfo && stream.binary) {
      const binaryBase = stream.binary.split('/').pop();
      appInfo = Gio.AppInfo.get_all().find(ai => {
        const exec = ai.get_executable();
        return exec && exec.split('/').pop() === binaryBase;
      }) ?? null;
    }
    return appInfo ? appInfo.get_icon() : null;
  } catch (e) {
    console.error(`PipeDeck: app icon lookup failed: ${e.message}`);
    return null;
  }
}

/**
 * One per-application row: icon, label, cubic volume slider, mute button.
 * Plain JS object (not a GObject subclass) that owns a PopupBaseMenuItem
 * actor (`this.item`) and all signal/timeout handles created for it.
 * `update()` is safe to call repeatedly (rebuild-in-place) and will not
 * clobber the slider position while the user is mid-drag.
 */
class AppVolumeRow {
  constructor(proxyOps) {
    this._proxyOps = proxyOps;
    this.id = null;
    this.dragging = false;
    this._muted = false;
    this._syncingSlider = false;
    this._debounceTimeoutId = 0;
    this._pendingVolume = null;

    this.item = new PopupMenu.PopupBaseMenuItem({activate: false});
    this.item.add_style_class_name('pipedeck-app-row');

    this._icon = new St.Icon({
      icon_name: FALLBACK_APP_ICON,
      icon_size: 16,
      style_class: 'pipedeck-app-icon',
    });
    this._label = new St.Label({
      text: '',
      style_class: 'pipedeck-app-label',
      y_align: Clutter.ActorAlign.CENTER,
    });
    this._slider = new Slider(0);
    this._slider.set({
      maximum_value: MAX_SLIDER_VALUE,
      overdrive_start: OVERDRIVE_START,
      x_expand: true,
      style_class: 'pipedeck-app-slider',
    });
    this._muteButton = new St.Button({
      style_class: 'pipedeck-mute-button icon-button',
      can_focus: true,
      child: new St.Icon({icon_name: 'audio-volume-high-symbolic', icon_size: 16}),
    });

    this.item.add_child(this._icon);
    this.item.add_child(this._label);
    this.item.add_child(this._slider);
    this.item.add_child(this._muteButton);

    this._sliderChangedId = this._slider.connect('notify::value', () => this._onSliderChanged());
    this._dragBeginId = this._slider.connect('drag-begin', () => {
      this.dragging = true;
    });
    this._dragEndId = this._slider.connect('drag-end', () => {
      this.dragging = false;
      this._flushVolume();
    });
    this._muteClickedId = this._muteButton.connect('clicked', () => this._onMuteClicked());
  }

  /** @param {{id:number, appName:string, binary:string, mediaName:string, volume:number, mute:boolean}} stream */
  update(stream) {
    this.id = stream.id;
    this._label.text = stream.appName || stream.mediaName || stream.binary || `Stream ${stream.id}`;

    const icon = lookupAppIcon(stream);
    if (icon)
      this._icon.gicon = icon;
    else
      this._icon.icon_name = FALLBACK_APP_ICON;

    if (!this.dragging) {
      const pos = Math.cbrt(Math.max(0, stream.volume));
      this._syncingSlider = true;
      this._slider.value = pos;
      this._syncingSlider = false;
    }

    this._muted = !!stream.mute;
    this._muteButton.child.icon_name =
      this._muted ? 'audio-volume-muted-symbolic' : 'audio-volume-high-symbolic';
  }

  _onSliderChanged() {
    if (this._syncingSlider || this.id === null)
      return;
    this._pendingVolume = Math.pow(this._slider.value, 3);
    this._scheduleDebounced();
  }

  _scheduleDebounced() {
    if (this._debounceTimeoutId)
      return;
    this._debounceTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, SLIDER_DEBOUNCE_MS, () => {
      this._debounceTimeoutId = 0;
      this._flushVolume();
      return GLib.SOURCE_REMOVE;
    });
  }

  _flushVolume() {
    if (this._pendingVolume === null || this.id === null)
      return;
    const volume = this._pendingVolume;
    this._pendingVolume = null;
    this._proxyOps.setVolume(this.id, volume).catch(() => {});
  }

  _onMuteClicked() {
    if (this.id === null)
      return;
    this._proxyOps.setMute(this.id, !this._muted).catch(() => {});
  }

  /** Disconnects everything and destroys the row's actor. Idempotent. */
  destroy() {
    if (this._debounceTimeoutId) {
      GLib.source_remove(this._debounceTimeoutId);
      this._debounceTimeoutId = 0;
    }
    if (this._sliderChangedId) {
      this._slider.disconnect(this._sliderChangedId);
      this._sliderChangedId = 0;
    }
    if (this._dragBeginId) {
      this._slider.disconnect(this._dragBeginId);
      this._dragBeginId = 0;
    }
    if (this._dragEndId) {
      this._slider.disconnect(this._dragEndId);
      this._dragEndId = 0;
    }
    if (this._muteClickedId) {
      this._muteButton.disconnect(this._muteClickedId);
      this._muteClickedId = 0;
    }
    this.item.destroy();
  }
}

/**
 * The Quick Settings item itself: a QuickMenuToggle titled "Audio" whose
 * subtitle tracks the current default output, opening a menu with
 * Output / Input / Notifications radio sections and per-app volume rows.
 * Holds no D-Bus state of its own — `rebuild()` is fed a plain snapshot by
 * the indicator, and `_proxyOps` is a small set of callbacks it uses to
 * issue commands back to the daemon.
 */
const PipeDeckToggle = GObject.registerClass(
class PipeDeckToggle extends QuickSettings.QuickMenuToggle {
  _init(proxyOps) {
    super._init({
      title: 'Audio',
      icon_name: 'audio-speakers-symbolic',
      toggle_mode: false,
    });

    this._proxyOps = proxyOps;
    this._appRows = new Map();

    // This toggle has no meaningful on/off state (see SPEC §2.4) -- clicking
    // the body opens the menu, same as clicking the arrow button does.
    this.connect('clicked', () => this.menu.open());
    this.connect('destroy', () => this._clearAppRows());

    this.menu.setHeader('audio-speakers-symbolic', 'Audio');
    // Live testing on chronos (2026-09-01): the default popup-menu width
    // clips long "<port> · <device nick>" labels -- widen it (stylesheet.css).
    this.menu.box.add_style_class_name('pipedeck-menu');

    this._unavailableItem = new PopupMenu.PopupMenuItem('PipeDeck daemon not running', {activate: false});
    this._unavailableItem.setSensitive(false);
    this._unavailableItem.add_style_class_name('pipedeck-unavailable-item');
    this.menu.addMenuItem(this._unavailableItem);

    this._outputHeader = new PopupMenu.PopupSeparatorMenuItem('Output');
    this._outputSection = new PopupMenu.PopupMenuSection();
    this.menu.addMenuItem(this._outputHeader);
    this.menu.addMenuItem(this._outputSection);

    this._inputHeader = new PopupMenu.PopupSeparatorMenuItem('Input');
    this._inputSection = new PopupMenu.PopupMenuSection();
    this.menu.addMenuItem(this._inputHeader);
    this.menu.addMenuItem(this._inputSection);

    this._notifHeader = new PopupMenu.PopupSeparatorMenuItem('Notifications');
    this._notifSection = new PopupMenu.PopupMenuSection();
    this.menu.addMenuItem(this._notifHeader);
    this.menu.addMenuItem(this._notifSection);

    // SPEC §7.4: Equalizer section, after Notifications, before the
    // per-app rows. Hidden entirely (header included) when the daemon has
    // no presets or there's no default output -- see _rebuildEqSection.
    this._eqHeader = new PopupMenu.PopupSeparatorMenuItem('Equalizer');
    this._eqSection = new PopupMenu.PopupMenuSection();
    this.menu.addMenuItem(this._eqHeader);
    this.menu.addMenuItem(this._eqSection);

    this._appsHeader = new PopupMenu.PopupSeparatorMenuItem();
    this._appsSection = new PopupMenu.PopupMenuSection();
    this.menu.addMenuItem(this._appsHeader);
    this.menu.addMenuItem(this._appsSection);

    this._noAppsItem = new PopupMenu.PopupMenuItem('No applications playing audio', {activate: false});
    this._noAppsItem.setSensitive(false);
    this._appsSection.addMenuItem(this._noAppsItem);

    this._sectionActors = [
      this._outputHeader, this._outputSection,
      this._inputHeader, this._inputSection,
      this._notifHeader, this._notifSection,
      this._eqHeader, this._eqSection,
      this._appsHeader, this._appsSection,
    ];

    this.setUnavailable();
  }

  setUnavailable() {
    this.subtitle = 'Not running';
    this._unavailableItem.visible = true;
    for (const actor of this._sectionActors)
      actor.visible = false;
    this._clearAppRows();
  }

  setAvailable() {
    this._unavailableItem.visible = false;
    for (const actor of this._sectionActors)
      actor.visible = true;
  }

  /**
   * @param {{devices: object[], streams: object[], notificationSink: string,
   *   ports: (object[]|null), eqPresets: (object[]|null), eq: (object[]|null),
   *   autoMute: (object[]|null)}} state
   *   `ports` is null on a v0.1 daemon (no Ports property) and every device
   *   then renders exactly as v0.1 did. `eqPresets`/`eq` are null on a
   *   pre-§7.3 daemon (no EqPresets/Eq properties) and the Equalizer
   *   section is hidden entirely. `autoMute` is null on a pre-§8.1 daemon
   *   (no AutoMute property) and no auto-mute switch is ever rendered.
   */
  rebuild(state) {
    const {devices, streams, notificationSink, ports, eqPresets, eq, autoMute} = state;
    const sinks = devices.filter(d => d.kind === 'sink');
    const sources = devices.filter(d => d.kind === 'source');
    const portsByNode = groupPortsByNode(ports);
    // Map<nodeId, enabled> -- SPEC §8.2 only shows the switch in the Output
    // section, so this is deliberately not passed to the Input call below.
    const autoMuteByNode = autoMute ? new Map(autoMute.map(a => [a.nodeId, a.enabled])) : null;

    this._rebuildDeviceSection(this._outputSection, sinks, portsByNode, autoMuteByNode);
    this._rebuildDeviceSection(this._inputSection, sources, portsByNode);
    this._rebuildNotificationSection(sinks, notificationSink);

    const defaultOut = sinks.find(d => d.isDefault);
    this._rebuildEqSection(defaultOut, eqPresets, eq);

    this._rebuildAppSection(streams);

    this.subtitle = this._describeOutput(defaultOut, portsByNode);
  }

  /** Toggle subtitle: "<active port> · <nick>" when the default output has
   * >=2 available ports, else just the device's nick (SPEC §6.2). */
  _describeOutput(device, portsByNode) {
    if (!device)
      return '';
    const label = device.nick;
    const availablePorts = (portsByNode.get(device.id) ?? []).filter(p => p.available === true);
    if (availablePorts.length < 2)
      return label;
    const activePort = availablePorts.find(p => p.active);
    return activePort ? `${activePort.description} · ${label}` : label;
  }

  /**
   * @param {Map<number, boolean>|null} [autoMuteByNode] present only for the
   *   Output section (SPEC §8.2); omitted for Input so the switch never
   *   renders there even if the daemon someday reports an input row.
   */
  _rebuildDeviceSection(section, devices, portsByNode, autoMuteByNode) {
    section.removeAll();
    if (devices.length === 0) {
      const empty = new PopupMenu.PopupMenuItem('No devices', {activate: false});
      empty.setSensitive(false);
      section.addMenuItem(empty);
      return;
    }
    for (const device of devices) {
      const availablePorts = (portsByNode.get(device.id) ?? []).filter(p => p.available === true);
      if (availablePorts.length >= 2) {
        for (const port of availablePorts)
          section.addMenuItem(this._buildPortItem(device, port));
      } else {
        section.addMenuItem(this._buildDeviceItem(device));
      }
      // SPEC §8.2: directly under a device's port rows (or its single row),
      // when AutoMute has an entry for this device -- hidden otherwise, and
      // hidden entirely on an older daemon (autoMuteByNode is null there).
      const autoMuteEnabled = autoMuteByNode?.get(device.id);
      if (autoMuteEnabled !== undefined)
        section.addMenuItem(this._buildAutoMuteItem(device.id, autoMuteEnabled));
    }
  }

  /**
   * SPEC §8.2 auto-mute switch. `PopupSwitchMenuItem` only emits `toggled`
   * from an actual user activation (click/keyboard) -- never from the
   * constructor's initial state, and this codebase never calls
   * `setToggleState()` on a live item (every section is torn down and
   * rebuilt fresh via `removeAll()` above), so a daemon-driven property
   * update that lands here as a freshly-constructed item with the new
   * `enabled` value can never loop back into `toggled` and re-issue the
   * same `SetAutoMute` call.
   */
  _buildAutoMuteItem(deviceId, enabled) {
    const item = new PopupMenu.PopupSwitchMenuItem(
      'Auto-mute speakers when headphones are plugged in', enabled);
    item.connect('toggled', (_item, state) => {
      this._proxyOps.setAutoMute(deviceId, state).catch(() => {});
    });
    return item;
  }

  _buildDeviceItem(device) {
    const item = new PopupMenu.PopupMenuItem(device.nick || device.name);
    item.label.add_style_class_name('pipedeck-device-label');
    item.setOrnament(device.isDefault ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
    item.connect('activate', () => {
      this._proxyOps.setDefault(device.kind, device.name).catch(() => {});
    });
    return item;
  }

  _buildPortItem(device, port) {
    const item = new PopupMenu.PopupMenuItem(`${port.description} · ${device.nick}`);
    item.label.add_style_class_name('pipedeck-device-label');
    item.setOrnament(
      device.isDefault && port.active ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
    item.connect('activate', () => this._activatePort(device, port));
    return item;
  }

  /** Selecting a port row: SetDefault first if the device isn't already
   * default, then SetPort if that port isn't already active. Sequential,
   * both awaited, per SPEC §6.2. */
  async _activatePort(device, port) {
    try {
      if (!device.isDefault)
        await this._proxyOps.setDefault(device.kind, device.name);
      if (!port.active)
        await this._proxyOps.setPort(device.id, port.routeIndex);
    } catch (e) {
      console.error(`PipeDeck: port selection failed: ${e.message}`);
    }
  }

  _rebuildNotificationSection(sinks, notificationSink) {
    this._notifSection.removeAll();

    const followItem = new PopupMenu.PopupMenuItem('Follow output');
    followItem.setOrnament(notificationSink === '' ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
    followItem.connect('activate', () => this._proxyOps.setNotificationSink('').catch(() => {}));
    this._notifSection.addMenuItem(followItem);

    for (const sink of sinks) {
      const item = new PopupMenu.PopupMenuItem(sink.description || sink.name);
      item.setOrnament(notificationSink === sink.name ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
      item.connect('activate', () => this._proxyOps.setNotificationSink(sink.name).catch(() => {}));
      this._notifSection.addMenuItem(item);
    }
  }

  /**
   * SPEC §7.4: "Off" row + one row per preset, applied to the current
   * default output. Hides the whole section (header included) when there
   * are no presets or no default output -- covers both a pre-§7.3 daemon
   * (eqPresets is null) and a §7.3 daemon with an empty presets dir.
   * @param {object|undefined} defaultOut the sink with isDefault === true
   * @param {object[]|null} eqPresets [{id, name}]
   * @param {object[]|null} eq [{nodeId, preset}], one row per output device
   */
  _rebuildEqSection(defaultOut, eqPresets, eq) {
    this._eqSection.removeAll();

    if (!defaultOut || !eqPresets || eqPresets.length === 0) {
      this._eqHeader.visible = false;
      this._eqSection.visible = false;
      return;
    }
    this._eqHeader.visible = true;
    this._eqSection.visible = true;

    const activeEntry = (eq ?? []).find(e => e.nodeId === defaultOut.id);
    const activePreset = activeEntry ? activeEntry.preset : '';

    const offItem = new PopupMenu.PopupMenuItem('Off');
    offItem.setOrnament(activePreset === '' ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
    offItem.connect('activate', () => this._activateEq(defaultOut.id, ''));
    this._eqSection.addMenuItem(offItem);

    for (const preset of eqPresets) {
      const item = new PopupMenu.PopupMenuItem(preset.name);
      item.setOrnament(activePreset === preset.id ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
      item.connect('activate', () => this._activateEq(defaultOut.id, preset.id));
      this._eqSection.addMenuItem(item);
    }
  }

  _activateEq(nodeId, presetId) {
    try {
      this._proxyOps.setEq(nodeId, presetId).catch(() => {});
    } catch (e) {
      console.error(`PipeDeck: EQ selection failed: ${e.message}`);
    }
  }

  _rebuildAppSection(streams) {
    const seen = new Set();
    for (const stream of streams) {
      seen.add(stream.id);
      let row = this._appRows.get(stream.id);
      if (!row) {
        row = new AppVolumeRow(this._proxyOps);
        this._appRows.set(stream.id, row);
        this._appsSection.addMenuItem(row.item);
      }
      row.update(stream);
    }
    for (const [id, row] of this._appRows) {
      if (!seen.has(id)) {
        row.destroy();
        this._appRows.delete(id);
      }
    }
    this._noAppsItem.visible = streams.length === 0;
  }

  _clearAppRows() {
    for (const row of this._appRows.values())
      row.destroy();
    this._appRows.clear();
  }
});

/**
 * Owns the D-Bus proxy lifecycle: watches the daemon's bus name, creates the
 * proxy when it appears, attempts one StartServiceByName activation when it
 * doesn't, and rebuilds the toggle on every property change / Changed
 * signal. Every proxy interaction is wrapped so a daemon crash or a bad
 * reply can never propagate into the Shell.
 */
const PipeDeckIndicator = GObject.registerClass(
class PipeDeckIndicator extends QuickSettings.SystemIndicator {
  _init() {
    super._init();

    this._proxy = null;
    this._propsChangedId = 0;
    this._changedSignalId = 0;
    this._activationAttempted = false;

    this._toggle = new PipeDeckToggle({
      setDefault: (kind, name) => this._callRemote('SetDefaultRemote', kind, name),
      setNotificationSink: name => this._callRemote('SetNotificationSinkRemote', name),
      setVolume: (id, volume) => this._callRemote('SetVolumeRemote', id, volume),
      setMute: (id, mute) => this._callRemote('SetMuteRemote', id, mute),
      setPort: (nodeId, routeIndex) => this._callRemote('SetPortRemote', nodeId, routeIndex),
      setEq: (nodeId, preset) => this._callRemote('SetEqRemote', nodeId, preset),
      setAutoMute: (nodeId, enabled) => this._callRemote('SetAutoMuteRemote', nodeId, enabled),
    });
    this.quickSettingsItems.push(this._toggle);

    this._nameWatchId = Gio.bus_watch_name(
      Gio.BusType.SESSION,
      BUS_NAME,
      Gio.BusNameWatcherFlags.NONE,
      () => this._onNameAppeared(),
      () => this._onNameVanished());

    this.connect('destroy', () => this._onDestroy());
  }

  /** Returns a Promise so callers that need a sequence (e.g. SetDefault then
   * SetPort, SPEC §6.2) can await it. Every failure is logged here, so
   * fire-and-forget call sites just need `.catch(() => {})` to avoid an
   * unhandled-rejection log line without losing the error message. */
  _callRemote(methodName, ...args) {
    return new Promise((resolve, reject) => {
      if (!this._proxy) {
        const error = new Error(`${methodName} called with no daemon proxy`);
        console.error(`PipeDeck: ${error.message}`);
        reject(error);
        return;
      }
      try {
        this._proxy[methodName](...args, (result, error) => {
          if (error) {
            console.error(`PipeDeck: ${methodName} failed: ${error.message}`);
            reject(error);
          } else {
            resolve(result);
          }
        });
      } catch (e) {
        console.error(`PipeDeck: ${methodName} threw: ${e.message}`);
        reject(e);
      }
    });
  }

  _onNameAppeared() {
    this._activationAttempted = false;
    this._createProxy();
  }

  _onNameVanished() {
    this._destroyProxy();
    this._toggle.setUnavailable();
    if (!this._activationAttempted) {
      this._activationAttempted = true;
      this._tryActivate();
    }
  }

  _tryActivate() {
    try {
      Gio.DBus.session.call(
        'org.freedesktop.DBus',
        '/org/freedesktop/DBus',
        'org.freedesktop.DBus',
        'StartServiceByName',
        new GLib.Variant('(su)', [BUS_NAME, 0]),
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
        (connection, result) => {
          try {
            connection.call_finish(result);
          } catch (e) {
            console.error(`PipeDeck: daemon activation failed: ${e.message}`);
          }
        });
    } catch (e) {
      console.error(`PipeDeck: daemon activation request failed: ${e.message}`);
    }
  }

  _createProxy() {
    if (this._proxy)
      return;
    try {
      new DaemonProxy(Gio.DBus.session, BUS_NAME, OBJECT_PATH, (proxy, error) => {
        if (error) {
          console.error(`PipeDeck: proxy creation failed: ${error.message}`);
          return;
        }
        this._proxy = proxy;
        this._propsChangedId = proxy.connect('g-properties-changed', () => this._queueRebuild());
        this._changedSignalId = proxy.connectSignal('Changed', () => this._queueRebuild());
        this._toggle.setAvailable();
        this._queueRebuild();
      });
    } catch (e) {
      console.error(`PipeDeck: proxy construction threw: ${e.message}`);
    }
  }

  _destroyProxy() {
    if (this._proxy) {
      try {
        if (this._propsChangedId)
          this._proxy.disconnect(this._propsChangedId);
        if (this._changedSignalId)
          this._proxy.disconnectSignal(this._changedSignalId);
      } catch (e) {
        console.error(`PipeDeck: proxy teardown failed: ${e.message}`);
      }
      this._proxy = null;
    }
    this._propsChangedId = 0;
    this._changedSignalId = 0;
  }

  _queueRebuild() {
    if (!this._proxy)
      return;
    try {
      // this._proxy.Ports/EqPresets/Eq/AutoMute are undefined when talking
      // to an older daemon (no such property in its introspection/cached
      // properties at all) -- normalize that to null so `rebuild()`
      // degrades gracefully rather than treating "unknown" the same as
      // "empty" (an empty array, which for EqPresets specifically must
      // also hide the section -- see SPEC §7.4).
      const state = {
        devices: (this._proxy.Devices ?? []).map(unpackDevice),
        streams: (this._proxy.Streams ?? []).map(unpackStream),
        notificationSink: this._proxy.NotificationSink ?? '',
        ports: this._proxy.Ports ? this._proxy.Ports.map(unpackPort) : null,
        eqPresets: this._proxy.EqPresets ? this._proxy.EqPresets.map(unpackEqPreset) : null,
        eq: this._proxy.Eq ? this._proxy.Eq.map(unpackEq) : null,
        autoMute: this._proxy.AutoMute ? this._proxy.AutoMute.map(unpackAutoMute) : null,
      };
      this._toggle.rebuild(state);
    } catch (e) {
      console.error(`PipeDeck: rebuild from proxy state failed: ${e.message}`);
    }
  }

  _onDestroy() {
    if (this._nameWatchId) {
      Gio.bus_unwatch_name(this._nameWatchId);
      this._nameWatchId = 0;
    }
    this._destroyProxy();
  }
});

export default class PipeDeckExtension extends Extension {
  enable() {
    this._indicator = new PipeDeckIndicator();
    Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator, 2);
  }

  disable() {
    this._indicator?.quickSettingsItems.forEach(item => item.destroy());
    this._indicator?.destroy();
    this._indicator = null;
  }
}
