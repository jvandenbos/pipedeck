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

/// One stream as notification routing sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingCandidate<'a> {
    /// Stream node id.
    pub id: u32,
    /// Matches [`is_notification_stream`].
    pub is_notification: bool,
    /// Its current `target.object` metadata value, if any.
    pub target: Option<&'a str>,
    /// PipeDeck pointed it at the notification sink earlier.
    pub routed_by_us: bool,
    /// It received a `SetStreamTarget` this session (SPEC-UI §6.2).
    pub manual: bool,
}

/// Which streams notification routing must point at `sink`, and which it
/// must hand back to the default — `(to_route, to_release)`, each ordered by
/// stream id.
///
/// SPEC-UI §6.2: a stream with a manual target is never touched — not routed,
/// and not released either, since its current target is the user's, not ours.
/// That includes `SetStreamTarget(id, "")`: "follow default" chosen by hand is
/// still a choice.
#[must_use]
pub fn plan_notification_routing(
    candidates: &[RoutingCandidate<'_>],
    sink: &str,
    sink_present: bool,
) -> (Vec<u32>, Vec<u32>) {
    let mut to_route = Vec::new();
    let mut to_release = Vec::new();
    for c in candidates {
        if c.manual {
            continue;
        }
        if c.is_notification && sink_present {
            if c.target != Some(sink) {
                to_route.push(c.id);
            }
        } else if c.routed_by_us {
            to_release.push(c.id);
        }
    }
    to_route.sort_unstable();
    to_release.sort_unstable();
    (to_route, to_release)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        id: u32,
        target: Option<&str>,
        routed_by_us: bool,
        manual: bool,
    ) -> RoutingCandidate<'_> {
        RoutingCandidate {
            id,
            is_notification: true,
            target,
            routed_by_us,
            manual,
        }
    }

    /// SPEC-UI §6.2: both streams were sent to `speaker` by an earlier
    /// notification sink; then the user moved 2 by hand to `hdmi`. A later
    /// `SetNotificationSink("headphones")` moves 1 and leaves 2 alone.
    #[test]
    fn a_manual_target_beats_a_later_notification_sink() {
        let streams = [
            event(1, Some("speaker"), true, false),
            event(2, Some("hdmi"), false, true),
        ];
        let (route, release) = plan_notification_routing(&streams, "headphones", true);
        assert_eq!(route, vec![1]);
        assert!(release.is_empty());
    }

    /// "Follow default" picked by hand (`SetStreamTarget(id, "")`) counts as
    /// manual too, and survives switching notifications back to the default.
    #[test]
    fn manual_follow_default_is_left_alone() {
        let streams = [event(3, None, false, true)];
        assert_eq!(
            plan_notification_routing(&streams, "headphones", true),
            (vec![], vec![])
        );
        // Following the default output: an unrouted manual stream is not
        // "released" either.
        let streams = [event(3, Some("hdmi"), true, true)];
        assert_eq!(
            plan_notification_routing(&streams, "", false),
            (vec![], vec![])
        );
    }

    #[test]
    fn routing_without_manual_targets_is_unchanged() {
        let music = RoutingCandidate {
            id: 9,
            is_notification: false,
            target: None,
            routed_by_us: false,
            manual: false,
        };
        let streams = [
            event(5, Some("headphones"), true, false), // already there
            event(4, None, false, false),
            music,
        ];
        assert_eq!(
            plan_notification_routing(&streams, "headphones", true),
            (vec![4], vec![])
        );
        // Sink absent: leave streams alone, but give back what we routed.
        assert_eq!(
            plan_notification_routing(&streams, "headphones", false),
            (vec![], vec![5])
        );
    }

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
