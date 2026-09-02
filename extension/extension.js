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

// Volume convention, per SPEC §2.1/§2.4: linear 0.0-1.5 (daemon clamps),
// slider position is the cube root so the perceptual step size is even and
// matches GNOME's own overdrive-region slider feel.
const MAX_VOLUME = 1.5;
const MAX_SLIDER_VALUE = Math.cbrt(MAX_VOLUME);
const OVERDRIVE_START = 1.0;

// Debounce slider -> SetVolume to <=20/s (SPEC §2.4).
const SLIDER_DEBOUNCE_MS = 50;

const FALLBACK_APP_ICON = 'audio-x-generic-symbolic';

/**
 * Devices tuple -> object. Wire type a(usssbbdb):
 * (id, name, description, kind, is_default, virtual, volume, mute)
 */
function unpackDevice([id, name, description, kind, isDefault, isVirtual, volume, mute]) {
  return {id, name, description, kind, isDefault, isVirtual, volume, mute};
}

/**
 * Streams tuple -> object. Wire type a(ussssdb):
 * (id, app_name, binary, media_name, target_name, volume, mute)
 */
function unpackStream([id, appName, binary, mediaName, targetName, volume, mute]) {
  return {id, appName, binary, mediaName, targetName, volume, mute};
}

/** Best-effort app icon lookup; never throws, always returns a Gio.Icon or null. */
function lookupAppIcon(stream) {
  try {
    let appInfo = null;
    if (stream.binary)
      appInfo = Gio.DesktopAppInfo.new(`${stream.binary}.desktop`);
    if (!appInfo && stream.appName)
      appInfo = Gio.DesktopAppInfo.new(`${stream.appName}.desktop`);
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
    this._proxyOps.setVolume(this.id, volume);
  }

  _onMuteClicked() {
    if (this.id === null)
      return;
    this._proxyOps.setMute(this.id, !this._muted);
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
   * @param {{devices: object[], streams: object[], notificationSink: string}} state
   */
  rebuild(state) {
    const {devices, streams, notificationSink} = state;
    const sinks = devices.filter(d => d.kind === 'sink');
    const sources = devices.filter(d => d.kind === 'source');

    this._rebuildDeviceSection(this._outputSection, sinks);
    this._rebuildDeviceSection(this._inputSection, sources);
    this._rebuildNotificationSection(sinks, notificationSink);
    this._rebuildAppSection(streams);

    const defaultOut = sinks.find(d => d.isDefault);
    this.subtitle = defaultOut ? defaultOut.description : '';
  }

  _rebuildDeviceSection(section, devices) {
    section.removeAll();
    if (devices.length === 0) {
      const empty = new PopupMenu.PopupMenuItem('No devices', {activate: false});
      empty.setSensitive(false);
      section.addMenuItem(empty);
      return;
    }
    for (const device of devices) {
      const item = new PopupMenu.PopupMenuItem(device.description || device.name);
      item.setOrnament(device.isDefault ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
      item.connect('activate', () => this._proxyOps.setDefault(device.kind, device.name));
      section.addMenuItem(item);
    }
  }

  _rebuildNotificationSection(sinks, notificationSink) {
    this._notifSection.removeAll();

    const followItem = new PopupMenu.PopupMenuItem('Follow output');
    followItem.setOrnament(notificationSink === '' ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
    followItem.connect('activate', () => this._proxyOps.setNotificationSink(''));
    this._notifSection.addMenuItem(followItem);

    for (const sink of sinks) {
      const item = new PopupMenu.PopupMenuItem(sink.description || sink.name);
      item.setOrnament(notificationSink === sink.name ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
      item.connect('activate', () => this._proxyOps.setNotificationSink(sink.name));
      this._notifSection.addMenuItem(item);
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

  _callRemote(methodName, ...args) {
    if (!this._proxy) {
      console.error(`PipeDeck: ${methodName} called with no daemon proxy`);
      return;
    }
    try {
      this._proxy[methodName](...args, (result, error) => {
        if (error)
          console.error(`PipeDeck: ${methodName} failed: ${error.message}`);
      });
    } catch (e) {
      console.error(`PipeDeck: ${methodName} threw: ${e.message}`);
    }
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
      const state = {
        devices: (this._proxy.Devices ?? []).map(unpackDevice),
        streams: (this._proxy.Streams ?? []).map(unpackStream),
        notificationSink: this._proxy.NotificationSink ?? '',
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
