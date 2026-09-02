//! The daemon's error type, mapped straight onto the SPEC's D-Bus error names.

use crate::config::ConfigError;

/// Errors returned from D-Bus methods.
///
/// The derive maps each variant to `dev.pipedeck.Error.<Variant>`, matching
/// SPEC §2.2.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "dev.pipedeck.Error")]
pub enum Error {
    /// Transport-level failure; required by the derive.
    #[zbus(error)]
    ZBus(zbus::Error),
    /// No device, stream or sink by that id or name.
    NotFound(String),
    /// The arguments were well-formed D-Bus but meaningless here.
    InvalidArgument(String),
    /// Something went wrong talking to PipeWire, or the PW thread is gone.
    PipeWire(String),
}

impl Error {
    /// `NotFound` with a formatted message.
    #[must_use]
    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Error::NotFound(what.to_string())
    }

    /// `InvalidArgument` with a formatted message.
    #[must_use]
    pub fn invalid(what: impl std::fmt::Display) -> Self {
        Error::InvalidArgument(what.to_string())
    }

    /// `PipeWire` with a formatted message.
    #[must_use]
    pub fn pipewire(what: impl std::fmt::Display) -> Self {
        Error::PipeWire(what.to_string())
    }
}

impl From<ConfigError> for Error {
    fn from(value: ConfigError) -> Self {
        Error::PipeWire(format!("config: {value}"))
    }
}

/// Convenience alias for daemon results.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::DBusError as _;

    #[test]
    fn error_names_match_the_spec() {
        assert_eq!(
            Error::not_found("device 5").name(),
            "dev.pipedeck.Error.NotFound"
        );
        assert_eq!(
            Error::invalid("kind").name(),
            "dev.pipedeck.Error.InvalidArgument"
        );
        assert_eq!(
            Error::pipewire("loop gone").name(),
            "dev.pipedeck.Error.PipeWire"
        );
    }

    #[test]
    fn messages_are_preserved() {
        assert_eq!(Error::not_found("device 5").description(), Some("device 5"));
    }
}
