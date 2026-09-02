//! `~/.config/pipedeck/config.toml` — load, validate, atomic save.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    /// Reserved for the v1.1 EQ feature. Unknown keys round-trip untouched.
    pub eq: toml::Table,
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

    /// Trim whitespace and drop empty entries so comparisons are predictable.
    pub fn normalize(&mut self) {
        self.notification_sink = self.notification_sink.trim().to_owned();
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
            eq: toml::Table::new(),
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
