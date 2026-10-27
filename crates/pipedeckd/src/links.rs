//! Where a playback stream is *actually* playing (SPEC-UI §6.1).
//!
//! `Streams.target_name` is the request (the stream's `target.object` /
//! `target.node` metadata, else its own `target.object` / `node.target`
//! property); this module derives the observation from `Link` globals, so a
//! client can show "-> Headphones · playing on Speaker" when the two differ —
//! e.g. a requested sink that has gone away and WirePlumber fell back.
//!
//! [`requested_target`] is the other half: the request itself.
//!
//! Pure: the PipeWire thread hands over its link table and a node lookup.

use std::collections::BTreeSet;

use crate::eq;

/// What the resolver needs to know about a link's input node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedNode<'a> {
    /// `node.name`.
    pub name: &'a str,
    /// `media.class` is `Audio/Sink*` (includes virtual sinks and filters).
    pub is_sink: bool,
    /// One of PipeDeck's own EQ filter-chain nodes (SPEC §7.1).
    pub is_eq: bool,
}

/// The `node.name` of the sink a stream is linked to, or `""` when unlinked.
///
/// Rules:
/// - A stereo stream has one link per channel, all to the same node; those
///   collapse to one destination.
/// - Only sink destinations count. A capture stream recording straight off the
///   playback stream (`pw-record --target <stream>`) is a link too, but not a
///   destination anybody would call "where it plays".
/// - A destination that is one of our EQ filters (`pipedeck.eq.<sink>`) is
///   reported as the physical `<sink>` it fronts: the filter is hidden from
///   `Devices`, and SPEC §7.1's whole point is that the real sink stays the
///   thing the user sees. The fronted sink comes from the filter's name rather
///   than from following its `.out` node's links, because the filter's playback
///   side is pinned to that sink (`target.object`, `stream.dont-remix`) — and
///   reading the name keeps the answer stable during the re-link a preset
///   toggle causes.
/// - A stream linked to more than one distinct sink at once (a client that
///   fans out, or the instant of a move where old and new links coexist) is
///   reported on the **lowest node id** — arbitrary, but deterministic, so the
///   property does not flicker between two values on unrelated graph events.
pub fn observed_sink<'a, L, F>(stream_id: u32, links: L, lookup: F) -> String
where
    L: IntoIterator<Item = (u32, u32)>,
    F: Fn(u32) -> Option<LinkedNode<'a>>,
{
    let destinations: BTreeSet<u32> = links
        .into_iter()
        .filter(|(out_node, _)| *out_node == stream_id)
        .map(|(_, in_node)| in_node)
        .collect();
    destinations
        .into_iter()
        .filter_map(&lookup)
        .find(|node| node.is_sink)
        .map_or_else(String::new, |node| {
            if node.is_eq {
                eq::fronted_sink(node.name).unwrap_or(node.name).to_owned()
            } else {
                node.name.to_owned()
            }
        })
}

/// WirePlumber's "no target, follow the default" value, in either the
/// metadata or a node property (`find-defined-target.lua`).
pub const FOLLOW_DEFAULT: &str = "-1";

/// Every place a stream's target request can live, as raw strings.
///
/// WirePlumber 0.5.18's `linking/find-defined-target.lua` reads them in this
/// order, later ones overriding earlier ones:
///
/// 1. the node property `target.object` (an `object.serial` or a name), else
///    the deprecated node property `node.target` (a node **id** or a name);
/// 2. the `default` metadata key `target.object` (serial or name), else the
///    metadata key `target.node` (node id or name) — either replaces step 1.
///
/// A final value of `"-1"` means "no target: follow the default".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetSources<'a> {
    /// `target.object` in the `default` metadata — what `SetStreamTarget`,
    /// notification routing and pipewire-pulse's move write.
    pub meta_object: Option<&'a str>,
    /// `target.node` in the `default` metadata. WirePlumber itself writes
    /// `-1` here when it links a stream to the default.
    pub meta_node: Option<&'a str>,
    /// The stream node's own `target.object` property — `pw-play --target`,
    /// `PULSE_SINK`, pipewire-pulse clients at connect time.
    pub prop_object: Option<&'a str>,
    /// The stream node's deprecated `node.target` property.
    pub prop_node: Option<&'a str>,
}

/// How a winning value is to be resolved when it is numeric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericKey {
    Serial,
    NodeId,
}

impl<'a> TargetSources<'a> {
    /// The value WirePlumber acts on, and how a number in it resolves.
    fn effective(&self) -> Option<(&'a str, NumericKey)> {
        let from_meta = self
            .meta_object
            .map(|v| (v, NumericKey::Serial))
            .or(self.meta_node.map(|v| (v, NumericKey::NodeId)));
        let from_prop = self
            .prop_object
            .map(|v| (v, NumericKey::Serial))
            .or(self.prop_node.map(|v| (v, NumericKey::NodeId)));
        from_meta.or(from_prop)
    }

    /// Would WirePlumber pick a target for this stream if the `target.object`
    /// metadata key were deleted?
    #[must_use]
    pub fn defines_target_without_meta_object(&self) -> bool {
        Self {
            meta_object: None,
            ..*self
        }
        .effective()
        .is_some_and(|(raw, _)| {
            let raw = raw.trim();
            !raw.is_empty() && raw != FOLLOW_DEFAULT
        })
    }
}

/// What `SetStreamTarget(id, "")` ("Follow default", SPEC-UI §6.1) writes to
/// the stream's `target.object` metadata: `None` deletes the key, `Some` writes
/// that value.
///
/// Deleting is enough — and leaves the metadata clean — when nothing else
/// names a sink. When the node's own `target.object` / `node.target` property
/// or a `target.node` entry would otherwise win, deleting would hand the stream
/// straight back to that request, so this writes WirePlumber's `-1` "no target"
/// sentinel, which overrides all of them.
#[must_use]
pub fn follow_default_value(sources: &TargetSources<'_>) -> Option<&'static str> {
    sources
        .defines_target_without_meta_object()
        .then_some(FOLLOW_DEFAULT)
}

/// The `node.name` a stream *asked* for (`Streams.target_name`), or `""` for
/// "follow the default".
///
/// Precedence is WirePlumber's — see [`TargetSources`]: metadata wins over the
/// node's own properties (a `SetStreamTarget` has to override what the client
/// asked for), `target.object` over `target.node` / `node.target`. A winning
/// `-1` or empty value is "follow the default". A numeric `target.object` is an
/// `object.serial` (resolved through `by_serial`), a numeric `target.node` /
/// `node.target` is a node id (resolved through `by_id`); anything else is a
/// `node.name`. A value that resolves to nothing we know is reported raw rather
/// than dropped, so a request for an absent sink still shows as a request.
pub fn requested_target<'a, S, I>(sources: TargetSources<'_>, by_serial: S, by_id: I) -> String
where
    S: Fn(u64) -> Option<&'a str>,
    I: Fn(u32) -> Option<&'a str>,
{
    let Some((raw, key)) = sources.effective() else {
        return String::new();
    };
    let raw = raw.trim();
    if raw.is_empty() || raw == FOLLOW_DEFAULT {
        return String::new();
    }
    let resolved = match key {
        NumericKey::Serial => raw.parse::<u64>().ok().and_then(&by_serial),
        NumericKey::NodeId => raw.parse::<u32>().ok().and_then(&by_id),
    };
    resolved.map_or_else(|| raw.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SPEAKER: &str = "alsa_output.pci-0000_07_00.6.HiFi__Speaker__sink";

    fn graph() -> HashMap<u32, (String, bool, bool)> {
        HashMap::from([
            (58, (SPEAKER.to_owned(), true, false)),
            (70, ("hdmi-sink".to_owned(), true, false)),
            (80, (eq::eq_node_name(SPEAKER), true, true)),
            (81, (eq::eq_playback_node_name(SPEAKER), false, true)),
            (90, ("pw-record".to_owned(), false, false)),
        ])
    }

    fn resolve(stream: u32, links: &[(u32, u32)]) -> String {
        let nodes = graph();
        observed_sink(stream, links.iter().copied(), |id| {
            nodes.get(&id).map(|(name, is_sink, is_eq)| LinkedNode {
                name,
                is_sink: *is_sink,
                is_eq: *is_eq,
            })
        })
    }

    fn lookup(sources: TargetSources<'_>) -> String {
        requested_target(
            sources,
            |serial| (serial == 4711).then_some("hdmi-sink"),
            |id| (id == 58).then_some(SPEAKER),
        )
    }

    fn request(metadata: Option<&str>, node_prop: Option<&str>) -> String {
        lookup(TargetSources {
            meta_object: metadata,
            prop_object: node_prop,
            ..TargetSources::default()
        })
    }

    #[test]
    fn metadata_wins_over_the_node_property() {
        assert_eq!(request(Some(SPEAKER), Some("hdmi-sink")), SPEAKER);
        // A serial in the metadata resolves too, and still wins.
        assert_eq!(request(Some("4711"), Some(SPEAKER)), "hdmi-sink");
    }

    #[test]
    fn the_node_property_is_used_without_metadata() {
        assert_eq!(request(None, Some(SPEAKER)), SPEAKER);
        assert_eq!(request(None, Some("4711")), "hdmi-sink");
        // An unknown serial or an absent sink is reported raw.
        assert_eq!(request(None, Some("99")), "99");
        assert_eq!(request(None, Some("gone-sink")), "gone-sink");
    }

    #[test]
    fn no_request_reads_as_follow_default() {
        assert_eq!(request(None, None), "");
        assert_eq!(request(None, Some("  ")), "");
    }

    /// `-1` is WirePlumber's "no target", wherever it is.
    #[test]
    fn minus_one_reads_as_follow_default() {
        // Metadata -1 overrides a connect-time node property.
        assert_eq!(request(Some("-1"), Some(SPEAKER)), "");
        assert_eq!(request(Some(" -1 "), Some("4711")), "");
        // A node property of -1 on its own.
        assert_eq!(request(None, Some("-1")), "");
        // WirePlumber's own `target.node = -1` (written when it links to the
        // default) also overrides the node property, as in the Lua.
        assert_eq!(
            lookup(TargetSources {
                meta_node: Some("-1"),
                prop_object: Some(SPEAKER),
                ..TargetSources::default()
            }),
            ""
        );
    }

    /// `node.target` and `target.node` hold node ids, not serials.
    #[test]
    fn node_target_and_target_node_resolve_by_id() {
        let prop_node = TargetSources {
            prop_node: Some("58"),
            ..TargetSources::default()
        };
        assert_eq!(lookup(prop_node), SPEAKER);
        let meta_node = TargetSources {
            meta_node: Some("58"),
            ..TargetSources::default()
        };
        assert_eq!(lookup(meta_node), SPEAKER);
        // 4711 is a known *serial*; as an id it resolves to nothing.
        let not_a_serial = TargetSources {
            meta_node: Some("4711"),
            ..TargetSources::default()
        };
        assert_eq!(lookup(not_a_serial), "4711");
        // A name works in either key.
        let by_name = TargetSources {
            prop_node: Some("hdmi-sink"),
            ..TargetSources::default()
        };
        assert_eq!(lookup(by_name), "hdmi-sink");
    }

    /// The Lua's order: `target.object` before `target.node` in the metadata,
    /// `target.object` before `node.target` on the node, metadata before node.
    #[test]
    fn fallback_order_matches_wireplumber() {
        assert_eq!(
            lookup(TargetSources {
                meta_object: Some("4711"),
                meta_node: Some("58"),
                ..TargetSources::default()
            }),
            "hdmi-sink"
        );
        assert_eq!(
            lookup(TargetSources {
                prop_object: Some("4711"),
                prop_node: Some("58"),
                ..TargetSources::default()
            }),
            "hdmi-sink"
        );
        assert_eq!(
            lookup(TargetSources {
                meta_node: Some("58"),
                prop_object: Some("4711"),
                ..TargetSources::default()
            }),
            SPEAKER
        );
    }

    /// Follow default on a stream that asked for a sink at connect time
    /// writes -1, and the result reads back as "".
    #[test]
    fn follow_default_over_a_node_property_writes_minus_one() {
        let before = TargetSources {
            meta_object: Some("4711"),
            prop_object: Some(SPEAKER),
            ..TargetSources::default()
        };
        assert_eq!(lookup(before), "hdmi-sink");
        let written = follow_default_value(&before);
        assert_eq!(written, Some("-1"));
        let after = TargetSources {
            meta_object: written,
            ..before
        };
        assert_eq!(lookup(after), "");

        // No node property: the key is deleted instead, and that reads "" too.
        let plain = TargetSources {
            meta_object: Some(SPEAKER),
            ..TargetSources::default()
        };
        assert_eq!(follow_default_value(&plain), None);
        let after = TargetSources {
            meta_object: None,
            ..plain
        };
        assert_eq!(lookup(after), "");
    }

    /// Deleting `target.object` is enough only when nothing else names a sink.
    #[test]
    fn follow_default_needs_the_sentinel_only_against_another_request() {
        let bare = TargetSources {
            meta_object: Some(SPEAKER),
            ..TargetSources::default()
        };
        assert!(!bare.defines_target_without_meta_object());
        let with_prop = TargetSources {
            meta_object: Some(SPEAKER),
            prop_object: Some("hdmi-sink"),
            ..TargetSources::default()
        };
        assert!(with_prop.defines_target_without_meta_object());
        let with_node_target = TargetSources {
            prop_node: Some("58"),
            ..TargetSources::default()
        };
        assert!(with_node_target.defines_target_without_meta_object());
        // WirePlumber's own target.node = -1 already says "default".
        let wp_default = TargetSources {
            meta_node: Some("-1"),
            prop_object: Some("hdmi-sink"),
            ..TargetSources::default()
        };
        assert!(!wp_default.defines_target_without_meta_object());
        // ...but a real target.node id does not.
        let wp_pinned = TargetSources {
            meta_node: Some("58"),
            ..TargetSources::default()
        };
        assert!(wp_pinned.defines_target_without_meta_object());
    }

    #[test]
    fn per_channel_links_collapse_to_one_sink() {
        assert_eq!(resolve(100, &[(100, 58), (100, 58)]), SPEAKER);
    }

    #[test]
    fn unlinked_streams_report_nothing() {
        assert_eq!(resolve(100, &[]), "");
        // Links of *other* streams are not ours.
        assert_eq!(resolve(100, &[(101, 58)]), "");
        // A link into a node we do not track (a port of something unbound).
        assert_eq!(resolve(100, &[(100, 999)]), "");
    }

    #[test]
    fn the_eq_filter_resolves_to_the_sink_it_fronts() {
        // stream -> pipedeck.eq.<sink>; the filter's .out -> sink is a link of
        // node 81, not of the stream, and does not change the answer.
        assert_eq!(resolve(100, &[(100, 80), (100, 80), (81, 58)]), SPEAKER);
        assert_eq!(resolve(100, &[(100, 80)]), SPEAKER);
    }

    #[test]
    fn capture_streams_are_not_destinations() {
        assert_eq!(resolve(100, &[(100, 90)]), "");
        assert_eq!(resolve(100, &[(100, 90), (100, 70)]), "hdmi-sink");
    }

    #[test]
    fn two_sinks_at_once_pick_the_lowest_node_id() {
        assert_eq!(resolve(100, &[(100, 70), (100, 58)]), SPEAKER);
        assert_eq!(resolve(100, &[(100, 58), (100, 70)]), SPEAKER);
    }
}
