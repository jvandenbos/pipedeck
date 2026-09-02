//! WirePlumber `default` metadata: key names and value (de)serialisation.
//!
//! The `default` metadata object stores default-device selections as JSON
//! objects of the shape `{"name":"<node.name>"}` under subject 0. Reading uses
//! the *effective* keys (`default.audio.sink`), writing uses the *configured*
//! keys (`default.configured.audio.sink`) — exactly what `wpctl set-default`
//! does.

use serde::{Deserialize, Serialize};

use crate::state::DeviceKind;

/// `metadata.name` of the metadata global we care about.
pub const METADATA_NAME_DEFAULT: &str = "default";

/// Effective default sink, as chosen by the session manager.
pub const KEY_DEFAULT_SINK: &str = "default.audio.sink";
/// Effective default source.
pub const KEY_DEFAULT_SOURCE: &str = "default.audio.source";
/// User-configured default sink (what we write).
pub const KEY_CONFIGURED_SINK: &str = "default.configured.audio.sink";
/// User-configured default source (what we write).
pub const KEY_CONFIGURED_SOURCE: &str = "default.configured.audio.source";
/// Per-stream routing key read by WirePlumber's node policy.
pub const KEY_TARGET_OBJECT: &str = "target.object";

/// SPA type string used for the JSON-encoded default-device values.
pub const TYPE_SPA_JSON: &str = "Spa:String:JSON";
/// SPA type string used for the plain-string `target.object` value.
///
/// WirePlumber 0.5 resolves `target.object` either as an `object.serial`
/// (numeric) or as a `node.name`. We always write `node.name` because it
/// survives re-plugs and session restarts, so a plain string type is correct.
pub const TYPE_SPA_STRING: &str = "Spa:String";

#[derive(Debug, Deserialize)]
struct NameIn {
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct NameOut<'a> {
    name: &'a str,
}

/// Parse a `{"name":"..."}` metadata value into the node name it points at.
///
/// Returns `None` for an absent name, an empty name, malformed JSON, or the
/// explicit `null` WirePlumber writes when a default is cleared.
#[must_use]
pub fn parse_name_value(value: &str) -> Option<String> {
    let parsed: NameIn = serde_json::from_str(value).ok()?;
    let name = parsed.name?;
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Render a node name as the `{"name":"..."}` metadata value, escaping properly.
#[must_use]
pub fn format_name_value(name: &str) -> String {
    serde_json::to_string(&NameOut { name }).expect("NameOut always serialises")
}

/// The effective (read-side) metadata key for a device kind.
#[must_use]
pub fn effective_key(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Sink => KEY_DEFAULT_SINK,
        DeviceKind::Source => KEY_DEFAULT_SOURCE,
    }
}

/// The configured (write-side) metadata key for a device kind.
#[must_use]
pub fn configured_key(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Sink => KEY_CONFIGURED_SINK,
        DeviceKind::Source => KEY_CONFIGURED_SOURCE,
    }
}

/// Map any of the four default keys back to the kind it describes.
#[must_use]
pub fn kind_for_effective_key(key: &str) -> Option<DeviceKind> {
    match key {
        KEY_DEFAULT_SINK => Some(DeviceKind::Sink),
        KEY_DEFAULT_SOURCE => Some(DeviceKind::Source),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wireplumber_values() {
        assert_eq!(
            parse_name_value(r#"{"name":"alsa_output.pci-0000_0c_00.4.analog-stereo"}"#),
            Some("alsa_output.pci-0000_0c_00.4.analog-stereo".to_owned())
        );
        // Whitespace and extra keys are tolerated.
        assert_eq!(
            parse_name_value(r#"{ "name": "x", "other": 1 }"#),
            Some("x".to_owned())
        );
    }

    #[test]
    fn parse_rejects_junk_and_empties() {
        assert_eq!(parse_name_value(""), None);
        assert_eq!(parse_name_value("null"), None);
        assert_eq!(parse_name_value("{}"), None);
        assert_eq!(parse_name_value(r#"{"name":null}"#), None);
        assert_eq!(parse_name_value(r#"{"name":""}"#), None);
        assert_eq!(parse_name_value(r#"{"name":42}"#), None);
        assert_eq!(parse_name_value("not json"), None);
    }

    #[test]
    fn formats_and_escapes() {
        assert_eq!(format_name_value("simple"), r#"{"name":"simple"}"#);
        assert_eq!(
            format_name_value(r#"weird"name\"#),
            r#"{"name":"weird\"name\\"}"#
        );
    }

    #[test]
    fn format_parse_round_trip() {
        for name in ["a", "alsa_output.pci-0000_0c_00.4.analog-stereo", "sp\"ace"] {
            assert_eq!(
                parse_name_value(&format_name_value(name)).as_deref(),
                Some(name)
            );
        }
    }

    #[test]
    fn key_helpers() {
        assert_eq!(effective_key(DeviceKind::Sink), "default.audio.sink");
        assert_eq!(effective_key(DeviceKind::Source), "default.audio.source");
        assert_eq!(
            configured_key(DeviceKind::Sink),
            "default.configured.audio.sink"
        );
        assert_eq!(
            configured_key(DeviceKind::Source),
            "default.configured.audio.source"
        );
        assert_eq!(
            kind_for_effective_key("default.audio.sink"),
            Some(DeviceKind::Sink)
        );
        assert_eq!(
            kind_for_effective_key("default.configured.audio.sink"),
            None
        );
    }
}
