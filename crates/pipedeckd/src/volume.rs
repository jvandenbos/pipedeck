//! Volume conversions.
//!
//! PipeWire stores per-channel volumes as **linear** amplitude factors in the
//! `Props` param's `channelVolumes` array. That is the number the daemon exposes
//! on D-Bus (`SetVolume` / the `volume` field of the `Devices` and `Streams`
//! properties), so clients never have to guess a scale.
//!
//! GNOME's own sliders — and `wpctl`'s displayed value — use a *cubic* scale,
//! which is perceptually closer to linear loudness. The helpers here do that
//! mapping so the CLI and the Shell extension can share one implementation.

/// Highest *linear* volume the daemon will accept: `1.5^3`, i.e. 150 % on the
/// cubic scale that `wpctl` and GNOME display (their "1.50" is a `channelVolumes`
/// entry of 3.375).
pub const MAX_VOLUME: f64 = 3.375;

/// Clamp a linear volume into the accepted `0.0 ..= 3.375` range.
///
/// NaN is treated as `0.0` rather than propagating into the graph.
#[must_use]
pub fn clamp_volume(volume: f64) -> f64 {
    if volume.is_nan() {
        0.0
    } else {
        volume.clamp(0.0, MAX_VOLUME)
    }
}

/// Linear volume -> cubic slider position (`pos = vol^(1/3)`).
#[must_use]
pub fn linear_to_cubic(volume: f64) -> f64 {
    let v = clamp_volume(volume);
    v.cbrt()
}

/// Cubic slider position -> linear volume (`vol = pos^3`).
#[must_use]
pub fn cubic_to_linear(position: f64) -> f64 {
    let p = if position.is_nan() {
        0.0
    } else {
        position.max(0.0)
    };
    clamp_volume(p * p * p)
}

/// Percentage on the cubic scale (0–150, as the CLI takes it and as `wpctl`
/// shows it) -> linear volume. 100 % == 1.0, 50 % == 0.125.
#[must_use]
pub fn percent_to_linear(percent: f64) -> f64 {
    cubic_to_linear(percent / 100.0)
}

/// Linear volume -> percentage on the cubic scale. 1.0 == 100 %, 0.125 == 50 %.
#[must_use]
pub fn linear_to_percent(volume: f64) -> f64 {
    linear_to_cubic(volume) * 100.0
}

/// How far above the cap a level has to sit, in cubic percent, before it is
/// worth a write (SPEC §9.2).
///
/// Two reasons this is not zero. A strict `>` compares two `f32`s that have
/// each been through `cubic_to_linear` and a `f64`->`f32` narrowing, so the
/// card echoing back the exact level we just wrote reads as a few ULPs *above*
/// the cap — chronos logged `restored 60%, above the 60% cap` on 2026-09-06
/// from precisely that. And because the clamp now re-fires for the whole
/// window, that echo would otherwise be an infinite ping-pong rather than a
/// one-off cosmetic wrinkle. Half a cubic percent is below `wpctl`'s own
/// two-decimal display precision, so nothing the user could see is ignored.
pub const PORT_SWITCH_CLAMP_EPSILON_PERCENT: f64 = 0.5;

/// SPEC §9.2's decision: does a port switch's restored level need clamping?
///
/// `current` is the route's `channelVolumes` — every channel, linear. (The
/// daemon's [`crate::route::RouteProps`] has already reduced that array to its
/// loudest entry, so the PipeWire side passes a one-element slice; the general
/// shape is kept because the rule is "no channel above the cap".) `cap` is the
/// linear ceiling, or `None` when `safety.port_switch_max_percent` is `0`.
///
/// Returns the linear level to write, or `None` when nothing needs doing —
/// which covers a disabled cap, an empty/`NaN`-only array, a route that came
/// back quieter than the cap, and a route sitting *at* the cap within
/// [`PORT_SWITCH_CLAMP_EPSILON_PERCENT`].
///
/// The comparison happens on the **cubic** scale, which is the scale the cap is
/// configured on, so the tolerance means the same thing at 20 % as at 120 %.
#[must_use]
pub fn port_switch_clamp(current: &[f32], cap: Option<f32>) -> Option<f32> {
    let cap = cap.filter(|c| c.is_finite() && *c >= 0.0)?;
    let loudest = current
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    if !loudest.is_finite() {
        return None;
    }
    let over = linear_to_percent(f64::from(loudest)) - linear_to_percent(f64::from(cap));
    (over > PORT_SWITCH_CLAMP_EPSILON_PERCENT).then_some(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn clamps_into_range() {
        assert!(close(clamp_volume(-1.0), 0.0));
        assert!(close(clamp_volume(0.0), 0.0));
        assert!(close(clamp_volume(0.25), 0.25));
        assert!(close(clamp_volume(1.5), 1.5));
        assert!(close(clamp_volume(9.0), MAX_VOLUME));
        assert!(close(clamp_volume(f64::NAN), 0.0));
    }

    #[test]
    fn cubic_round_trips() {
        for &v in &[0.0_f64, 0.001, 0.125, 0.5, 1.0, 1.5] {
            assert!(close(cubic_to_linear(linear_to_cubic(v)), v), "v = {v}");
        }
    }

    #[test]
    fn cubic_matches_wpctl_convention() {
        // wpctl shows 0.50 for a channelVolume of 0.125.
        assert!(close(linear_to_cubic(0.125), 0.5));
        assert!(close(cubic_to_linear(0.5), 0.125));
    }

    #[test]
    fn cubic_clamps_out_of_range_input() {
        assert!(close(cubic_to_linear(-2.0), 0.0));
        assert!(close(cubic_to_linear(5.0), MAX_VOLUME));
        assert!(close(linear_to_cubic(-1.0), 0.0));
    }

    #[test]
    fn percent_matches_wpctl_cubic_scale() {
        assert!(close(percent_to_linear(0.0), 0.0));
        assert!(close(percent_to_linear(100.0), 1.0));
        assert!(close(percent_to_linear(50.0), 0.125));
        assert!(close(percent_to_linear(150.0), 3.375));
        assert!(close(percent_to_linear(400.0), MAX_VOLUME));
        assert!(close(linear_to_percent(0.125), 50.0));
        assert!(close(linear_to_percent(MAX_VOLUME), 150.0));
    }

    /// SPEC §9.2: only a route louder than the cap is clamped, and only when
    /// the cap is on.
    #[test]
    fn port_switch_clamp_only_pulls_levels_down() {
        // The SPEC's own example: 82 % restored against a 60 % cap.
        let cap = percent_to_linear(60.0) as f32;
        let restored = percent_to_linear(82.0) as f32;
        let clamped = port_switch_clamp(&[restored, restored], Some(cap)).expect("clamps");
        assert!((f64::from(clamped) - percent_to_linear(60.0)).abs() < 1e-6);

        // 40 % is below the cap: untouched.
        let quiet = percent_to_linear(40.0) as f32;
        assert_eq!(port_switch_clamp(&[quiet, quiet], Some(cap)), None);

        // Exactly at the cap is not "above" it — no pointless write.
        assert_eq!(port_switch_clamp(&[cap, cap], Some(cap)), None);

        // ... and neither is the echo of our own clamp write, which comes back
        // a few ULPs off after a cubic_to_linear + f64->f32 round trip. This is
        // the chronos 2026-09-06 "restored 60%, above the 60% cap" case, and
        // with the re-clamping window it would otherwise ping-pong forever.
        let echo = (cubic_to_linear(0.60) as f32).to_bits() + 4;
        let echo = f32::from_bits(echo);
        assert!(echo > cap, "the echo really is numerically above the cap");
        assert_eq!(port_switch_clamp(&[echo, echo], Some(cap)), None);

        // The tolerance is only a tolerance: a level a user could actually see
        // as different still gets pulled down.
        let just_over = percent_to_linear(61.0) as f32;
        assert_eq!(port_switch_clamp(&[just_over], Some(cap)), Some(cap));

        // The *loudest* channel decides, not the first or an average.
        assert_eq!(port_switch_clamp(&[quiet, restored], Some(cap)), Some(cap));
    }

    /// `port_switch_max_percent = 0` turns the whole rule off, and a degenerate
    /// input can never produce a write.
    #[test]
    fn port_switch_clamp_is_off_or_silent_on_degenerate_input() {
        let loud = percent_to_linear(120.0) as f32;
        assert_eq!(port_switch_clamp(&[loud], None), None);
        assert_eq!(port_switch_clamp(&[], Some(0.5)), None);
        assert_eq!(port_switch_clamp(&[f32::NAN], Some(0.5)), None);
        assert_eq!(port_switch_clamp(&[loud], Some(f32::NAN)), None);
        assert_eq!(port_switch_clamp(&[loud], Some(-1.0)), None);
        // A cap of 0 % is a real setting: silence the port rather than ignore it.
        assert_eq!(port_switch_clamp(&[loud], Some(0.0)), Some(0.0));
        // One NaN channel does not hide a loud one.
        assert_eq!(port_switch_clamp(&[f32::NAN, loud], Some(0.5)), Some(0.5));
    }
}
