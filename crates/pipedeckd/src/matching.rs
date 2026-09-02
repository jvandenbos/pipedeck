//! Which playback streams count as "notification sounds".
//!
//! SPEC §2.1: a stream matches when its `media.role` is `event` or
//! `Notification`, or when its `application.name` appears in the config's
//! `notification_apps` list.

/// `media.role` values that mean "this is an event/notification sound".
///
/// Compared case-insensitively: libcanberra and GNOME write `event`, some
/// applications write `Notification`, and a few get the capitalisation wrong.
const NOTIFICATION_ROLES: &[&str] = &["event", "notification"];

/// True when `media.role` alone marks the stream as a notification.
#[must_use]
pub fn role_is_notification(role: Option<&str>) -> bool {
    let Some(role) = role else { return false };
    let role = role.trim();
    NOTIFICATION_ROLES
        .iter()
        .any(|candidate| role.eq_ignore_ascii_case(candidate))
}

/// True when `application.name` is listed in the config's `notification_apps`.
///
/// Matched case-insensitively after trimming, because `application.name` is a
/// human-typed string in both the config file and the application.
#[must_use]
pub fn app_is_notification(app_name: Option<&str>, notification_apps: &[String]) -> bool {
    let Some(app_name) = app_name else {
        return false;
    };
    let app_name = app_name.trim();
    if app_name.is_empty() {
        return false;
    }
    notification_apps
        .iter()
        .any(|listed| listed.trim().eq_ignore_ascii_case(app_name))
}

/// The full SPEC §2.1 rule.
#[must_use]
pub fn is_notification_stream(
    role: Option<&str>,
    app_name: Option<&str>,
    notification_apps: &[String],
) -> bool {
    role_is_notification(role) || app_is_notification(app_name, notification_apps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn matches_canonical_roles() {
        assert!(is_notification_stream(Some("event"), None, &[]));
        assert!(is_notification_stream(Some("Notification"), None, &[]));
        assert!(is_notification_stream(Some("EVENT"), None, &[]));
        assert!(is_notification_stream(Some(" event "), None, &[]));
    }

    #[test]
    fn ignores_ordinary_roles() {
        assert!(!is_notification_stream(Some("Music"), None, &[]));
        assert!(!is_notification_stream(Some("Movie"), None, &[]));
        assert!(!is_notification_stream(Some("game"), None, &[]));
        assert!(!is_notification_stream(Some(""), None, &[]));
        assert!(!is_notification_stream(None, None, &[]));
    }

    #[test]
    fn matches_configured_app_names() {
        let list = apps(&["Discord", "Slack"]);
        assert!(is_notification_stream(
            Some("Music"),
            Some("Discord"),
            &list
        ));
        assert!(is_notification_stream(None, Some("slack"), &list));
        assert!(!is_notification_stream(None, Some("Firefox"), &list));
    }

    #[test]
    fn empty_app_name_never_matches() {
        let list = apps(&["", "Discord"]);
        assert!(!is_notification_stream(None, Some(""), &list));
        assert!(!is_notification_stream(None, None, &list));
    }

    #[test]
    fn role_and_app_are_independent() {
        assert!(role_is_notification(Some("event")));
        assert!(!role_is_notification(Some("Discord")));
        assert!(app_is_notification(Some("Discord"), &apps(&["discord"])));
        assert!(!app_is_notification(Some("Discord"), &[]));
    }
}
