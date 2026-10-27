//! Where a playback stream is *actually* playing (SPEC-UI §6.1).
//!
//! `Streams.target_name` is the request (the stream's `target.object`
//! metadata); this module derives the observation from `Link` globals, so a
//! client can show "-> Headphones · playing on Speaker" when the two differ —
//! e.g. a requested sink that has gone away and WirePlumber fell back.
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
