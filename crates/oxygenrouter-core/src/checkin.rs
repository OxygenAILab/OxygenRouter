//! Daily check-in: the award draw.
//!
//! Ported from the reference's `model/checkin.go:73`: the award is uniform in
//! `[min, max]`, and a range whose maximum does not exceed its minimum pays the
//! minimum rather than refusing to draw.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use rand_core::{OsRng, RngCore};

/// Draw one daily award, in the same unit the caller quotes the range in.
///
/// The range is normalised rather than trusted: an operator who swaps the two
/// settings (max below min) gets the smaller value paid, which is the same
/// answer the reference reaches with its `max > min` guard.
pub fn random_award(min: i64, max: i64) -> i64 {
    let (low, high) = if max < min { (max, min) } else { (min, max) };
    if high <= low {
        return low;
    }
    // `next_u64 % span` has a negligible bias for the spans an award range can
    // have (at most a few orders of magnitude), and avoids pulling in a full
    // distribution crate for one draw per user per day.
    let span = (high - low) as u64 + 1;
    low + (OsRng.next_u64() % span) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_range_pays_its_only_value() {
        assert_eq!(random_award(500, 500), 500);
        assert_eq!(random_award(0, 0), 0);
        // A swapped range is normalised rather than drawn from nonsense.
        for _ in 0..100 {
            let award = random_award(900, 100);
            assert!((100..=900).contains(&award), "award {award} left the range");
        }
    }

    #[test]
    fn a_draw_stays_inside_its_range() {
        for _ in 0..500 {
            let award = random_award(2_000, 20_000);
            assert!(
                (2_000..=20_000).contains(&award),
                "award {award} left the range"
            );
        }
    }
}
