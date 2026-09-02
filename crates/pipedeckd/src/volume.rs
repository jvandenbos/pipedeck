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
}
