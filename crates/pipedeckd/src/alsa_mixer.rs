//! ALSA "Auto-Mute Mode" — detection, policy and the mixer write (SPEC §8.1).
//!
//! **Finding (live, ALC892):** the codec's ALSA mixer enum `Auto-Mute Mode`
//! (`Enabled`/`Disabled`) hard-mutes the line-out whenever a headphone plug is
//! present, whatever port software selects — so "Line Out" in the panel
//! produced silence. It is not reachable through PipeWire at all: it is an
//! alsa-lib simple-mixer *enumerated* control on the card behind the sink.
//!
//! Everything except [`probe`] and [`set`] is pure and unit-tested. Those two
//! are the only functions here that touch alsa-lib; they block (briefly — one
//! mixer open, load and read), so the D-Bus side runs them on
//! `tokio::task::spawn_blocking` and the PipeWire thread never calls them at
//! all. The PipeWire thread's whole job is publishing each routed sink's
//! `alsa.card` index and card name.

use alsa::mixer::{Mixer, Selem, SelemChannelId, SelemId};
use tracing::debug;

/// Name of the simple-mixer control this module drives.
pub const AUTO_MUTE_CONTROL: &str = "Auto-Mute Mode";

/// Substring that marks a route as the headphones route, matched
/// case-insensitively against the route `name` (`analog-output-headphones`).
pub const HEADPHONES_MARKER: &str = "headphone";

/// How much freedom the daemon has to change `Auto-Mute Mode` by itself
/// (config key `alsa.auto_mute_policy`, SPEC §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoMutePolicy {
    /// Turn auto-mute off by itself when a non-headphones output port is
    /// selected while headphones are plugged in. Never turns it back on.
    #[default]
    Auto,
    /// Never touch the control unless `SetAutoMute` asks for it.
    Manual,
}

impl AutoMutePolicy {
    /// The spelling used in the config file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AutoMutePolicy::Auto => "auto",
            AutoMutePolicy::Manual => "manual",
        }
    }

    /// Parse the config spelling; unknown values are not accepted here so the
    /// caller can decide whether to fall back or complain.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(AutoMutePolicy::Auto),
            "manual" => Some(AutoMutePolicy::Manual),
            _ => None,
        }
    }
}

impl std::fmt::Display for AutoMutePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Is this route the card's headphones route?
///
/// Matched on the route `name` (`analog-output-headphones`), not the
/// description, because the name is the stable ALSA/ACP identifier while the
/// description is localised.
#[must_use]
pub fn is_headphones_route(route_name: &str) -> bool {
    route_name.to_ascii_lowercase().contains(HEADPHONES_MARKER)
}

/// SPEC §8.1's automatic switch, as one decision.
///
/// With policy `auto`, selecting an **output** port that is *not* the
/// headphones port while the headphones port is `available` (a plug is in) and
/// auto-mute is on means the speakers would stay hard-muted — so turn it off.
/// Every other combination leaves the control alone; in particular the daemon
/// never turns auto-mute back *on* by itself.
///
/// The caller guarantees the route is an output route by only ever having card
/// rows for sinks.
#[must_use]
pub fn should_disable_auto_mute(
    route_name: &str,
    headphones_route_available: bool,
    currently_enabled: bool,
    policy: AutoMutePolicy,
) -> bool {
    policy == AutoMutePolicy::Auto
        && currently_enabled
        && headphones_route_available
        && !is_headphones_route(route_name)
}

/// The alsa-lib mixer device name for a card index: `1` → `hw:1`.
#[must_use]
pub fn card_device(card: u32) -> String {
    format!("hw:{card}")
}

/// Map one enum item name onto the state it represents.
///
/// Matched case-insensitively, per SPEC §8.1's `Enabled`/`Disabled` items.
#[must_use]
pub fn enum_item_enabled(item: &str) -> Option<bool> {
    match item.trim().to_ascii_lowercase().as_str() {
        "enabled" | "enable" | "on" => Some(true),
        "disabled" | "disable" | "off" => Some(false),
        _ => None,
    }
}

/// Which enum index sets the control to `enabled`, given the card's item list.
#[must_use]
pub fn enum_index_for(items: &[String], enabled: bool) -> Option<u32> {
    items
        .iter()
        .position(|item| enum_item_enabled(item) == Some(enabled))
        .and_then(|idx| u32::try_from(idx).ok())
}

/// Open the card's mixer and hand the `Auto-Mute Mode` element to `f`.
///
/// A fresh `Mixer` per call is deliberate: the control is read and written
/// rarely, and a long-lived handle would need `handle_events` polling to stay
/// current with `alsactl`/`amixer` changes made behind our back.
fn with_selem<T>(
    card: u32,
    f: impl FnOnce(&Selem<'_>) -> std::result::Result<T, String>,
) -> std::result::Result<T, String> {
    let device = card_device(card);
    let mixer = Mixer::new(&device, false)
        .map_err(|e| format!("could not open the mixer for {device}: {e}"))?;
    let selem = mixer
        .find_selem(&SelemId::new(AUTO_MUTE_CONTROL, 0))
        .ok_or_else(|| format!("{device} has no `{AUTO_MUTE_CONTROL}` control"))?;
    if !selem.is_enumerated() {
        return Err(format!(
            "`{AUTO_MUTE_CONTROL}` on {device} is not an enumerated control"
        ));
    }
    f(&selem)
}

/// Read every enum item name off an element, in index order.
fn item_names(selem: &Selem<'_>) -> std::result::Result<Vec<String>, String> {
    let count = selem
        .get_enum_items()
        .map_err(|e| format!("could not count `{AUTO_MUTE_CONTROL}` items: {e}"))?;
    Ok((0..count)
        .map(|idx| selem.get_enum_item_name(idx).unwrap_or_default())
        .collect())
}

/// Read `Auto-Mute Mode` on one card.
///
/// `Some(enabled)` when the card has the control and its current item is one we
/// understand; `None` when the card has no such control, cannot be opened, or
/// reports an item that is neither `Enabled` nor `Disabled`. A `None` is a
/// cacheable answer — "this card is not one we can help with".
#[must_use]
pub fn probe(card: u32) -> Option<bool> {
    let result = with_selem(card, |selem| {
        let index = selem
            .get_enum_item(SelemChannelId::mono())
            .map_err(|e| format!("could not read `{AUTO_MUTE_CONTROL}`: {e}"))?;
        let item = selem
            .get_enum_item_name(index)
            .map_err(|e| format!("could not name `{AUTO_MUTE_CONTROL}` item {index}: {e}"))?;
        enum_item_enabled(&item)
            .ok_or_else(|| format!("`{AUTO_MUTE_CONTROL}` reported unexpected item `{item}`"))
    });
    match result {
        Ok(enabled) => Some(enabled),
        Err(e) => {
            debug!(card, "no usable auto-mute control: {e}");
            None
        }
    }
}

/// Write `Auto-Mute Mode` on one card.
///
/// # Errors
/// When the card has no such control, the mixer cannot be opened, the control
/// has no item for the wanted state, or alsa-lib rejects the write.
pub fn set(card: u32, enabled: bool) -> std::result::Result<(), String> {
    with_selem(card, |selem| {
        let items = item_names(selem)?;
        let index = enum_index_for(&items, enabled).ok_or_else(|| {
            format!(
                "`{AUTO_MUTE_CONTROL}` on {} has no `{}` item; it has: {}",
                card_device(card),
                if enabled { "Enabled" } else { "Disabled" },
                items.join(", ")
            )
        })?;
        selem
            .set_enum_item(SelemChannelId::mono(), index)
            .map_err(|e| format!("could not set `{AUTO_MUTE_CONTROL}` to item {index}: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_round_trips_through_the_config_spelling() {
        assert_eq!(AutoMutePolicy::parse("auto"), Some(AutoMutePolicy::Auto));
        assert_eq!(
            AutoMutePolicy::parse("  MANUAL "),
            Some(AutoMutePolicy::Manual)
        );
        assert_eq!(AutoMutePolicy::parse("nonsense"), None);
        assert_eq!(AutoMutePolicy::parse(""), None);
        assert_eq!(AutoMutePolicy::default(), AutoMutePolicy::Auto);
        assert_eq!(AutoMutePolicy::Manual.to_string(), "manual");
        assert_eq!(
            AutoMutePolicy::parse(AutoMutePolicy::Auto.as_str()),
            Some(AutoMutePolicy::Auto)
        );
    }

    #[test]
    fn headphone_routes_are_recognised_by_name() {
        assert!(is_headphones_route("analog-output-headphones"));
        assert!(is_headphones_route("Analog-Output-HEADPHONE"));
        assert!(!is_headphones_route("analog-output-lineout"));
        assert!(!is_headphones_route("hdmi-output-0"));
        assert!(!is_headphones_route(""));
    }

    /// SPEC §8.1: only `auto` + non-headphones output route + headphones
    /// available + currently enabled turns it off.
    #[test]
    fn the_automatic_switch_fires_only_on_the_spec_combination() {
        assert!(should_disable_auto_mute(
            "analog-output-lineout",
            true,
            true,
            AutoMutePolicy::Auto
        ));

        // Selecting the headphones port is exactly what auto-mute is for.
        assert!(!should_disable_auto_mute(
            "analog-output-headphones",
            true,
            true,
            AutoMutePolicy::Auto
        ));
        // Nothing plugged in: auto-mute is not muting anything.
        assert!(!should_disable_auto_mute(
            "analog-output-lineout",
            false,
            true,
            AutoMutePolicy::Auto
        ));
        // Already off — nothing to do, and we never turn it back on.
        assert!(!should_disable_auto_mute(
            "analog-output-lineout",
            true,
            false,
            AutoMutePolicy::Auto
        ));
        // `manual` means hands off, whatever the situation.
        assert!(!should_disable_auto_mute(
            "analog-output-lineout",
            true,
            true,
            AutoMutePolicy::Manual
        ));
    }

    #[test]
    fn card_index_maps_to_the_mixer_device_name() {
        assert_eq!(card_device(0), "hw:0");
        assert_eq!(card_device(1), "hw:1");
        assert_eq!(card_device(11), "hw:11");
    }

    #[test]
    fn enum_items_are_matched_case_insensitively() {
        assert_eq!(enum_item_enabled("Enabled"), Some(true));
        assert_eq!(enum_item_enabled("enabled"), Some(true));
        assert_eq!(enum_item_enabled(" DISABLED "), Some(false));
        assert_eq!(enum_item_enabled("Disabled"), Some(false));
        assert_eq!(enum_item_enabled("Speaker Only"), None);
        assert_eq!(enum_item_enabled(""), None);
    }

    #[test]
    fn enum_index_lookup_finds_the_wanted_state() {
        let items = vec!["Disabled".to_owned(), "Enabled".to_owned()];
        assert_eq!(enum_index_for(&items, false), Some(0));
        assert_eq!(enum_index_for(&items, true), Some(1));

        // Order is the card's, not ours.
        let flipped = vec!["Enabled".to_owned(), "Disabled".to_owned()];
        assert_eq!(enum_index_for(&flipped, true), Some(0));
        assert_eq!(enum_index_for(&flipped, false), Some(1));

        // A control whose items we do not understand yields nothing rather
        // than guessing at an index.
        let odd = vec!["Speaker Only".to_owned(), "Headphone Only".to_owned()];
        assert_eq!(enum_index_for(&odd, true), None);
        assert_eq!(enum_index_for(&odd, false), None);
        assert_eq!(enum_index_for(&[], true), None);
    }
}
