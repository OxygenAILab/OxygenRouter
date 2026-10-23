//! HTTP status-code range parsing, mirroring NewAPI's
//! `setting/operation_setting/status_code_ranges.go`.
//!
//! The reference exposes two editable rule strings — `AutomaticRetryStatusCodes`
//! and `AutomaticDisableStatusCodes` — as comma-separated codes and inclusive
//! ranges (`429,500-599`). This module is the one parser for both, so the
//! console's write-time validation and the proxy's read-time decision cannot
//! drift:
//!
//! * a full-width comma is accepted, like the reference;
//! * spaces inside a token are ignored;
//! * codes must lie in 100..=599 and ranges must not run backwards;
//! * ranges are sorted and merged, including adjacency (500-599 + 600 is
//!   impossible, but 429 + 430-431 merges into 429-431);
//! * an empty string parses to "no rules", which the reference treats as
//!   "nothing matches" rather than as an error.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::fmt::Write;

/// The reference's shipped retry ranges
/// (`setting/operation_setting/status_code_ranges.go`): 1xx, 3xx, 401-407,
/// 409-499, 500-503, 505-523, 525-599. Notably absent: 400, 408, 504, 524.
pub const DEFAULT_RETRY_STATUS_RANGES: &str =
    "100-199,300-399,401-407,409-499,500-503,505-523,525-599";

/// The reference's shipped disable range: a 401 from a channel disables it.
pub const DEFAULT_DISABLE_STATUS_RANGES: &str = "401";

/// Status codes that are never retried even when a range would match.
pub const ALWAYS_SKIP_RETRY_STATUS: [u16; 2] = [504, 524];

/// Parse a rule string into sorted, merged inclusive ranges.
pub fn parse_http_status_ranges(input: &str) -> Result<Vec<(u16, u16)>, String> {
    let normalized = input.trim().replace('\u{ff0c}', ",");
    if normalized.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut ranges: Vec<(u16, u16)> = Vec::new();
    let mut invalid: Vec<String> = Vec::new();
    for segment in normalized.split(',') {
        let token: String = segment.chars().filter(|c| !c.is_whitespace()).collect();
        if token.is_empty() {
            continue;
        }
        match parse_token(&token) {
            Ok(range) => ranges.push(range),
            Err(_) => invalid.push(segment.trim().to_string()),
        }
    }

    if !invalid.is_empty() {
        return Err(format!(
            "invalid http status code rules: {}",
            invalid.join(", ")
        ));
    }
    if ranges.is_empty() {
        return Ok(Vec::new());
    }

    ranges.sort_unstable();
    let mut merged: Vec<(u16, u16)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1.saturating_add(1) => {
                if end > last.1 {
                    last.1 = end;
                }
            }
            _ => merged.push((start, end)),
        }
    }
    Ok(merged)
}

fn parse_token(token: &str) -> Result<(u16, u16), String> {
    if let Some((start, end)) = token.split_once('-') {
        if start.is_empty() || end.is_empty() || end.contains('-') {
            return Err(format!("invalid range token: {token}"));
        }
        let start = parse_code(start)?;
        let end = parse_code(end)?;
        if start > end {
            return Err(format!("range start > end: {token}"));
        }
        return Ok((start, end));
    }
    let code = parse_code(token)?;
    Ok((code, code))
}

fn parse_code(raw: &str) -> Result<u16, String> {
    let code: u16 = raw
        .parse()
        .map_err(|_| format!("invalid status code: {raw}"))?;
    if !(100..=599).contains(&code) {
        return Err(format!("status code out of bounds: {raw}"));
    }
    Ok(code)
}

/// Whether a code falls in any of the parsed ranges. Codes outside 100..=599
/// never match, mirroring the reference's `shouldMatchStatusCodeRanges`.
pub fn matches_status_ranges(ranges: &[(u16, u16)], code: u16) -> bool {
    if !(100..=599).contains(&code) {
        return false;
    }
    ranges
        .iter()
        .any(|(start, end)| *start <= code && code <= *end)
}

/// Render ranges back into the reference's canonical string (`429`,
/// `500-599`, comma separated).
pub fn format_status_ranges(ranges: &[(u16, u16)]) -> String {
    let mut out = String::new();
    for (index, (start, end)) in ranges.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        if start == end {
            let _ = write!(out, "{start}");
        } else {
            let _ = write!(out, "{start}-{end}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped strings parse to the reference's rule *set*, and parsing a
    /// formatted string again is a no-op.
    ///
    /// Deliberately not a string round-trip: the shipped default spells two
    /// adjacent ranges apart (`409-499,500-503`) because that is how the
    /// reference's constants read, and both implementations merge adjacent
    /// ranges when they parse. The merged form covers exactly the same codes,
    /// which is the property that decides whether a request retries.
    #[test]
    fn the_shipped_defaults_parse_to_the_reference_rule_set() {
        let retry = parse_http_status_ranges(DEFAULT_RETRY_STATUS_RANGES).unwrap();
        // The four codes the reference deliberately leaves out, and the ones on
        // either side of each gap.
        for status in [408, 400, 504, 524] {
            assert!(
                !matches_status_ranges(&retry, status),
                "{status} must not be retried by default"
            );
        }
        for status in [100, 199, 300, 399, 401, 407, 409, 499, 500, 503, 505, 523, 525, 599] {
            assert!(
                matches_status_ranges(&retry, status),
                "{status} must be retried by default"
            );
        }
        // Formatting is canonical and stable, so a value the console writes back
        // does not keep changing shape.
        let formatted = format_status_ranges(&retry);
        assert_eq!(
            format_status_ranges(&parse_http_status_ranges(&formatted).unwrap()),
            formatted
        );

        let disable = parse_http_status_ranges(DEFAULT_DISABLE_STATUS_RANGES).unwrap();
        assert_eq!(disable, vec![(401, 401)]);
    }

    #[test]
    fn ranges_are_sorted_and_merged_including_adjacency() {
        let ranges = parse_http_status_ranges("431,429,500-599,430,200-201").unwrap();
        assert_eq!(ranges, vec![(200, 201), (429, 431), (500, 599)]);
    }

    #[test]
    fn whitespace_and_full_width_commas_are_accepted() {
        let ranges = parse_http_status_ranges(" 429 \u{ff0c} 500 - 503 ").unwrap();
        assert_eq!(ranges, vec![(429, 429), (500, 503)]);
    }

    #[test]
    fn invalid_tokens_are_refused_by_name() {
        let error = parse_http_status_ranges("429,600,abc,503-500").unwrap_err();
        assert!(error.contains("600"), "{error}");
        assert!(error.contains("abc"), "{error}");
        assert!(error.contains("503-500"), "{error}");
    }

    #[test]
    fn an_empty_string_means_no_rules() {
        assert!(parse_http_status_ranges("").unwrap().is_empty());
        assert!(parse_http_status_ranges(" , ").unwrap().is_empty());
    }

    #[test]
    fn matching_ignores_codes_outside_the_http_band() {
        let ranges = vec![(100, 599)];
        assert!(matches_status_ranges(&ranges, 200));
        assert!(!matches_status_ranges(&ranges, 99));
        assert!(!matches_status_ranges(&ranges, 99 + 600));
    }
}
