//! `~/.config/pipedeck/config.toml` — load, validate, atomic save.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::alsa_mixer::AutoMutePolicy;

/// Directory name under the XDG config root.
const APP_DIR: &str = "pipedeck";
/// File name of the config.
const FILE_NAME: &str = "config.toml";

/// Errors that can come out of config handling.
#[derive(Debug)]
pub enum ConfigError {
    /// Neither `XDG_CONFIG_HOME` nor `HOME` is set.
    NoConfigDir,
    /// The file could not be read or written.
    Io(std::io::Error),
    /// The file is not valid TOML, or has the wrong shape.
    Parse(toml::de::Error),
    /// The config could not be serialised back to TOML.
    Serialize(toml::ser::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NoConfigDir => {
                f.write_str("neither XDG_CONFIG_HOME nor HOME is set; cannot locate config")
            }
            ConfigError::Io(e) => write!(f, "config I/O error: {e}"),
            ConfigError::Parse(e) => write!(f, "config parse error: {e}"),
            ConfigError::Serialize(e) => write!(f, "config serialise error: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        ConfigError::Io(value)
    }
}

/// On-disk configuration.
///
/// Only `node.name` values are stored — never node ids, which change every
/// session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `node.name` of the sink notification sounds go to. Empty = follow the
    /// default output.
    pub notification_sink: String,
    /// Extra `application.name` values treated as notification sources.
    pub notification_apps: Vec<String>,
    /// EQ selections: `"<sink node.name>" = "<preset id>"` (SPEC §7.1).
    ///
    /// Kept as a raw table rather than a typed map so a hand-edited file with
    /// an unexpected value round-trips untouched instead of refusing to load.
    pub eq: toml::Table,
    /// ALSA mixer settings — the `Auto-Mute Mode` policy and the per-card
    /// choice the daemon remembers (SPEC §8.1).
    pub alsa: AlsaConfig,
    /// Loudness safety — the port-switch level cap (SPEC §9.2).
    pub safety: SafetyConfig,
}

/// The `[safety]` table (SPEC §9.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyConfig {
    /// Ceiling, as a **cubic-scale** percentage (the number `wpctl` and GNOME
    /// show), on the level a PipeDeck-initiated port switch may restore. `0`
    /// turns the rule off; anything above 150 normalises to 150, which is the
    /// daemon's own maximum volume and therefore can never clamp anything.
    pub port_switch_max_percent: u32,
}

/// SPEC §9.2's default cap: 60 % on the cubic scale.
pub const DEFAULT_PORT_SWITCH_MAX_PERCENT: u32 = 60;

/// Highest cap the daemon will store — [`crate::volume::MAX_VOLUME`] expressed
/// on the cubic scale.
pub const MAX_PORT_SWITCH_MAX_PERCENT: u32 = 150;

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            port_switch_max_percent: DEFAULT_PORT_SWITCH_MAX_PERCENT,
        }
    }
}

/// The `[alsa]` table (SPEC §8.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlsaConfig {
    /// `auto` (default) or `manual`. Anything else reads as `auto`, so a typo
    /// cannot stop the daemon from starting.
    pub auto_mute_policy: String,
    /// `"<card name>" = true|false` — the remembered `Auto-Mute Mode` choice,
    /// keyed by `alsa.card_name`/`api.alsa.card.longname` because card
    /// *indices* move between boots.
    ///
    /// A raw table for the same reason `[eq]` is one: a hand-edited value of
    /// the wrong type round-trips instead of refusing to load.
    pub auto_mute: toml::Table,
}

impl Default for AlsaConfig {
    fn default() -> Self {
        Self {
            auto_mute_policy: AutoMutePolicy::Auto.as_str().to_owned(),
            auto_mute: toml::Table::new(),
        }
    }
}

impl Config {
    /// The config file path: `$XDG_CONFIG_HOME/pipedeck/config.toml`, falling
    /// back to `$HOME/.config/pipedeck/config.toml`.
    pub fn path() -> Result<PathBuf, ConfigError> {
        Ok(Self::config_dir()?.join(FILE_NAME))
    }

    /// The directory holding the config file.
    pub fn config_dir() -> Result<PathBuf, ConfigError> {
        config_dir_from(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        )
    }

    /// Load from the default path, returning defaults when the file is absent.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&Self::path()?)
    }

    /// Load from an explicit path, returning defaults when the file is absent.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        match fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    /// Parse TOML text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let mut config: Config = toml::from_str(text).map_err(ConfigError::Parse)?;
        config.normalize();
        Ok(config)
    }

    /// Render to TOML text.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(ConfigError::Serialize)
    }

    /// Save to the default path.
    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&Self::path()?)
    }

    /// Save atomically: write a sibling temp file, fsync, then rename over the
    /// target so a crash can never leave a half-written config behind.
    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        let text = self.to_toml()?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir)?;

        let file_name = path.file_name().map_or_else(
            || FILE_NAME.to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let tmp = dir.join(format!(".{file_name}.tmp-{}", std::process::id()));

        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }

        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(ConfigError::Io(e));
        }
        Ok(())
    }

    /// The preset id configured for a sink, if any (SPEC §7.1's `[eq]` table).
    ///
    /// A non-string value is ignored rather than rejected, so a hand-edited
    /// file cannot stop the daemon from starting.
    #[must_use]
    pub fn eq_preset(&self, sink_name: &str) -> Option<&str> {
        self.eq
            .get(sink_name)
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// Set (or, with an empty preset, clear) the EQ preset for a sink.
    pub fn set_eq_preset(&mut self, sink_name: &str, preset: &str) {
        let preset = preset.trim();
        if preset.is_empty() {
            self.eq.remove(sink_name);
        } else {
            self.eq
                .insert(sink_name.to_owned(), toml::Value::String(preset.to_owned()));
        }
    }

    /// Every `(sink node.name, preset id)` pair in the `[eq]` table, sorted.
    #[must_use]
    pub fn eq_entries(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = self
            .eq
            .iter()
            .filter_map(|(k, v)| {
                let value = v.as_str()?.trim();
                (!value.is_empty()).then(|| (k.clone(), value.to_owned()))
            })
            .collect();
        entries.sort();
        entries
    }

    /// The `Auto-Mute Mode` policy (SPEC §8.1).
    ///
    /// An unrecognised spelling reads as the default `auto` rather than
    /// erroring — the file is hand-editable and a typo must not wedge the
    /// daemon.
    #[must_use]
    pub fn auto_mute_policy(&self) -> AutoMutePolicy {
        AutoMutePolicy::parse(&self.alsa.auto_mute_policy).unwrap_or_default()
    }

    /// The remembered `Auto-Mute Mode` choice for a card, if there is one.
    ///
    /// A non-boolean value is ignored rather than rejected, matching
    /// [`Config::eq_preset`].
    #[must_use]
    pub fn auto_mute(&self, card_name: &str) -> Option<bool> {
        self.alsa
            .auto_mute
            .get(card_name)
            .and_then(toml::Value::as_bool)
    }

    /// Remember a card's `Auto-Mute Mode` choice.
    pub fn set_auto_mute(&mut self, card_name: &str, enabled: bool) {
        self.alsa
            .auto_mute
            .insert(card_name.to_owned(), toml::Value::Boolean(enabled));
    }

    /// Every `(card name, enabled)` pair in `[alsa.auto_mute]`, sorted.
    #[must_use]
    pub fn auto_mute_entries(&self) -> Vec<(String, bool)> {
        let mut entries: Vec<(String, bool)> = self
            .alsa
            .auto_mute
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_bool()?)))
            .collect();
        entries.sort();
        entries
    }

    /// The port-switch level cap as a cubic-scale percentage (SPEC §9.2).
    ///
    /// Always in `0 ..= 150`, whatever the file said.
    #[must_use]
    pub fn port_switch_max_percent(&self) -> u32 {
        self.safety
            .port_switch_max_percent
            .min(MAX_PORT_SWITCH_MAX_PERCENT)
    }

    /// The cap as a **linear** volume, or `None` when it is off (`0`).
    ///
    /// This is the form the PipeWire side compares `channelVolumes` against.
    #[must_use]
    pub fn port_switch_cap(&self) -> Option<f64> {
        let percent = self.port_switch_max_percent();
        (percent > 0).then(|| crate::volume::percent_to_linear(f64::from(percent)))
    }

    /// Set the port-switch level cap; `0` turns it off. Values above 150 are
    /// clamped rather than rejected, matching every other hand-editable field.
    pub fn set_port_switch_max_percent(&mut self, percent: u32) {
        self.safety.port_switch_max_percent = percent.min(MAX_PORT_SWITCH_MAX_PERCENT);
    }

    /// Trim whitespace and drop empty entries so comparisons are predictable.
    pub fn normalize(&mut self) {
        self.notification_sink = self.notification_sink.trim().to_owned();
        self.alsa.auto_mute_policy = self.auto_mute_policy().as_str().to_owned();
        self.safety.port_switch_max_percent = self.port_switch_max_percent();
        self.notification_apps = self
            .notification_apps
            .iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
    }
}

/// Resolve the config directory from explicit environment values.
///
/// Split out from [`Config::config_dir`] so the precedence rules can be tested
/// without mutating the process environment.
pub fn config_dir_from(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf, ConfigError> {
    let base = match xdg_config_home {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => {
            let home = home
                .filter(|h| !h.is_empty())
                .ok_or(ConfigError::NoConfigDir)?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(base.join(APP_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_empty() {
        let config = Config::default();
        assert!(config.notification_sink.is_empty());
        assert!(config.notification_apps.is_empty());
        assert!(config.eq.is_empty());
        assert!(config.alsa.auto_mute.is_empty());
        // SPEC §8.1: `auto` is the default policy.
        assert_eq!(config.auto_mute_policy(), AutoMutePolicy::Auto);
        // SPEC §9.2: the cap defaults to 60 % cubic, i.e. on.
        assert_eq!(config.port_switch_max_percent(), 60);
    }

    /// SPEC §9.2: `[safety] port_switch_max_percent` round-trips, `0` means
    /// off, and an out-of-range value normalises instead of failing to load.
    #[test]
    fn safety_table_round_trips() {
        let config = Config::from_toml(
            r"
[safety]
port_switch_max_percent = 45
",
        )
        .expect("parses");
        assert_eq!(config.port_switch_max_percent(), 45);
        let cap = config.port_switch_cap().expect("cap is on");
        assert!((cap - crate::volume::percent_to_linear(45.0)).abs() < 1e-9);

        let again = Config::from_toml(&config.to_toml().expect("serialise")).expect("reparse");
        assert_eq!(again, config);

        // `0` is off, not "silence everything".
        let off = Config::from_toml(
            r"
[safety]
port_switch_max_percent = 0
",
        )
        .expect("parses");
        assert_eq!(off.port_switch_max_percent(), 0);
        assert_eq!(off.port_switch_cap(), None);

        // Above the daemon's own maximum, normalised on load.
        let silly = Config::from_toml(
            r"
[safety]
port_switch_max_percent = 4000
",
        )
        .expect("parses");
        assert_eq!(silly.port_switch_max_percent(), 150);
        assert_eq!(silly.safety.port_switch_max_percent, 150);

        let mut config = Config::default();
        config.set_port_switch_max_percent(200);
        assert_eq!(config.port_switch_max_percent(), 150);
        config.set_port_switch_max_percent(0);
        assert_eq!(config.port_switch_cap(), None);
    }

    /// A config written before v1.3 has no `[safety]` table at all; it must
    /// still load, and pick up the default cap.
    #[test]
    fn pre_v1_3_configs_still_load() {
        let config = Config::from_toml(
            r#"
notification_sink = "alsa_output.pci-0000_00_1f.3.analog-stereo"
notification_apps = ["Slack"]

[eq]
"alsa_output.pci-0000_00_1f.3.analog-stereo" = "hd650"

[alsa]
auto_mute_policy = "auto"

[alsa.auto_mute]
"HD-Audio Generic" = false
"#,
        )
        .expect("an older config still parses");
        assert_eq!(config.notification_apps, vec!["Slack".to_owned()]);
        assert_eq!(config.auto_mute("HD-Audio Generic"), Some(false));
        assert_eq!(
            config.port_switch_max_percent(),
            DEFAULT_PORT_SWITCH_MAX_PERCENT
        );
    }

    /// SPEC §8.1: `[alsa]` carries `auto_mute_policy` plus a card-name-keyed
    /// `[alsa.auto_mute]` map, and the whole thing survives a save/load.
    #[test]
    fn alsa_table_round_trips() {
        let mut config = Config::from_toml(
            r#"
[alsa]
auto_mute_policy = "manual"
[alsa.auto_mute]
"HDA Intel PCH" = false
"USB Audio" = true
"broken" = "yes"
"#,
        )
        .expect("parses");

        assert_eq!(config.auto_mute_policy(), AutoMutePolicy::Manual);
        assert_eq!(config.auto_mute("HDA Intel PCH"), Some(false));
        assert_eq!(config.auto_mute("USB Audio"), Some(true));
        // A hand-edited value of the wrong type reads as "no choice", never as
        // a parse error.
        assert_eq!(config.auto_mute("broken"), None);
        assert_eq!(config.auto_mute("absent"), None);
        assert_eq!(
            config.auto_mute_entries(),
            vec![
                ("HDA Intel PCH".to_owned(), false),
                ("USB Audio".to_owned(), true)
            ]
        );

        config.set_auto_mute("Dell AW3423DW", true);
        assert_eq!(config.auto_mute("Dell AW3423DW"), Some(true));
        config.set_auto_mute("Dell AW3423DW", false);
        assert_eq!(config.auto_mute("Dell AW3423DW"), Some(false));

        let again = Config::from_toml(&config.to_toml().expect("serialise")).expect("reparse");
        assert_eq!(again, config);
        assert!(again.alsa.auto_mute.contains_key("broken"));
    }

    /// An unrecognised policy reads as `auto` and normalises to it, so a typo
    /// cannot stop the daemon from starting.
    #[test]
    fn unknown_policy_falls_back_to_auto() {
        let config = Config::from_toml(
            r#"
[alsa]
auto_mute_policy = "aggressive"
"#,
        )
        .expect("parses");
        assert_eq!(config.auto_mute_policy(), AutoMutePolicy::Auto);
        assert_eq!(config.alsa.auto_mute_policy, "auto");

        // ... and the accepted spellings are normalised, not just parsed.
        let config = Config::from_toml(
            r#"
[alsa]
auto_mute_policy = "  MANUAL  "
"#,
        )
        .expect("parses");
        assert_eq!(config.alsa.auto_mute_policy, "manual");
    }

    /// A config written by v1.1 must still load, and must gain the defaults.
    #[test]
    fn a_config_without_an_alsa_table_still_loads() {
        let config = Config::from_toml(
            r#"
notification_sink = "sink-a"
notification_apps = ["Discord"]
[eq]
"alsa_output.analog-stereo" = "hd650"
"#,
        )
        .expect("parses");
        assert_eq!(config.notification_sink, "sink-a");
        assert_eq!(config.auto_mute_policy(), AutoMutePolicy::Auto);
        assert!(config.auto_mute_entries().is_empty());
    }

    #[test]
    fn parses_the_spec_example() {
        let config = Config::from_toml(
            r#"
notification_sink = ""
notification_apps = []
[eq]
"#,
        )
        .expect("parses");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let config = Config::from_toml("").expect("parses");
        assert_eq!(config, Config::default());
        let config = Config::from_toml(r#"notification_sink = "sink-a""#).expect("parses");
        assert_eq!(config.notification_sink, "sink-a");
        assert!(config.notification_apps.is_empty());
    }

    #[test]
    fn normalizes_whitespace_and_blanks() {
        let config = Config::from_toml(
            r#"
notification_sink = "  sink-a  "
notification_apps = ["  Discord ", "", "   "]
"#,
        )
        .expect("parses");
        assert_eq!(config.notification_sink, "sink-a");
        assert_eq!(config.notification_apps, vec!["Discord".to_owned()]);
    }

    #[test]
    fn rejects_wrong_types() {
        assert!(Config::from_toml("notification_sink = 5").is_err());
        assert!(Config::from_toml(r#"notification_apps = "Discord""#).is_err());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("config.toml");

        let mut original = Config {
            notification_sink: "alsa_output.pci-0000_0c_00.4.analog-stereo".to_owned(),
            notification_apps: vec!["Discord".to_owned(), "Slack".to_owned()],
            ..Config::default()
        };
        original.normalize();

        original.save_to(&path).expect("save");
        let loaded = Config::load_from(&path).expect("load");
        assert_eq!(loaded, original);

        // A second save over an existing file must also work.
        original.notification_sink = "other".to_owned();
        original.save_to(&path).expect("resave");
        assert_eq!(
            Config::load_from(&path).expect("reload").notification_sink,
            "other"
        );

        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .expect("readdir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "left temp files: {leftovers:?}");
    }

    #[test]
    fn eq_section_round_trips_unknown_keys() {
        let config = Config::from_toml(
            r#"
[eq]
preset = "flat"
"#,
        )
        .expect("parses");
        let text = config.to_toml().expect("serialise");
        let again = Config::from_toml(&text).expect("reparse");
        assert_eq!(
            again.eq.get("preset").and_then(toml::Value::as_str),
            Some("flat")
        );
    }

    /// SPEC §7.1: `[eq]` is `"<sink node.name>" = "<preset id>"`.
    #[test]
    fn eq_table_maps_sinks_to_presets() {
        let mut config = Config::from_toml(
            r#"
[eq]
"alsa_output.pci-0000_28_00.4.analog-stereo" = "hd650"
"alsa_output.hdmi" = "  "
"broken" = 5
"#,
        )
        .expect("parses");

        assert_eq!(
            config.eq_preset("alsa_output.pci-0000_28_00.4.analog-stereo"),
            Some("hd650")
        );
        // Blank and non-string values read as "no preset", never as an error.
        assert_eq!(config.eq_preset("alsa_output.hdmi"), None);
        assert_eq!(config.eq_preset("broken"), None);
        assert_eq!(config.eq_preset("absent"), None);
        assert_eq!(
            config.eq_entries(),
            vec![(
                "alsa_output.pci-0000_28_00.4.analog-stereo".to_owned(),
                "hd650".to_owned()
            )]
        );

        config.set_eq_preset("alsa_output.hdmi", "flat");
        assert_eq!(config.eq_preset("alsa_output.hdmi"), Some("flat"));
        config.set_eq_preset("alsa_output.hdmi", "");
        assert_eq!(config.eq_preset("alsa_output.hdmi"), None);

        // ... and the whole thing survives a trip through the file format.
        let again = Config::from_toml(&config.to_toml().expect("serialise")).expect("reparse");
        assert_eq!(
            again.eq_preset("alsa_output.pci-0000_28_00.4.analog-stereo"),
            Some("hd650")
        );
        assert!(again.eq.contains_key("broken"));
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Config::load_from(&dir.path().join("absent.toml")).expect("load");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn config_dir_precedence() {
        use std::ffi::OsString;

        assert_eq!(
            config_dir_from(
                Some(OsString::from("/xdg")),
                Some(OsString::from("/home/jan"))
            )
            .expect("dir"),
            PathBuf::from("/xdg/pipedeck")
        );
        assert_eq!(
            config_dir_from(None, Some(OsString::from("/home/jan"))).expect("dir"),
            PathBuf::from("/home/jan/.config/pipedeck")
        );
        assert_eq!(
            config_dir_from(Some(OsString::new()), Some(OsString::from("/home/jan"))).expect("dir"),
            PathBuf::from("/home/jan/.config/pipedeck")
        );
        assert!(matches!(
            config_dir_from(None, None),
            Err(ConfigError::NoConfigDir)
        ));
        assert!(matches!(
            config_dir_from(None, Some(OsString::new())),
            Err(ConfigError::NoConfigDir)
        ));
    }
}
