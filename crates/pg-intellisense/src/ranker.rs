//! Scoring for candidate completion items.
//!
//! Match quality is scored on a 0–200 scale so composite ranking (context
//! weight + match quality + MRU bonus) fits comfortably in i32 without
//! needing fractional math.
//!
//! Match kinds in descending order:
//! - `200` exact case-insensitive equality
//! - `150` case-insensitive prefix
//! - `120` CamelHump — lowercased prefix chars align with word-start chars of
//!   the candidate treating capital letters as boundaries (so `uN` matches
//!   `userName`).
//! - `110` Snake initials — `un` matches `user_name` via `_` boundaries.
//! - ` 80` substring match
//! - ` 40` fuzzy subsequence (every prefix char appears in order)
//! - `None` otherwise
//!
//! A user-typed empty prefix returns a base 0 — every candidate is eligible.

pub fn prefix_score(prefix: &str, candidate: &str) -> Option<i32> {
    if prefix.is_empty() {
        return Some(0);
    }
    let p = prefix.to_ascii_lowercase();
    let c = candidate.to_ascii_lowercase();
    if c == p {
        return Some(200);
    }
    if c.starts_with(&p) {
        return Some(150);
    }
    if camel_hump(&p, candidate) {
        return Some(120);
    }
    if snake_initials(&p, candidate) {
        return Some(110);
    }
    if c.contains(&p) {
        return Some(80);
    }
    if fuzzy_subsequence(&p, &c) {
        return Some(40);
    }
    None
}

/// Match each prefix char against a word-initial in the candidate. Word
/// boundaries are (a) the first char, (b) any uppercase ASCII letter, and (c)
/// any char after `_` or digit→letter transitions.
fn camel_hump(lower_prefix: &str, candidate: &str) -> bool {
    let initials: Vec<char> = word_initials(candidate);
    subseq_prefix(lower_prefix, &initials)
}

fn snake_initials(lower_prefix: &str, candidate: &str) -> bool {
    let initials: Vec<char> = candidate
        .split('_')
        .filter_map(|w| w.chars().next().map(|c| c.to_ascii_lowercase()))
        .collect();
    subseq_prefix(lower_prefix, &initials)
}

fn subseq_prefix(prefix: &str, initials: &[char]) -> bool {
    let mut pi = prefix.chars();
    for ch in initials {
        match pi.clone().next() {
            Some(pc) if pc == *ch => {
                pi.next();
            }
            _ => {}
        }
    }
    pi.next().is_none()
}

/// CamelHump initials: first char plus any uppercase letters. `_` is NOT a
/// boundary here — snake-case matches are handled separately so the two
/// paths stay distinguishable and the scorer returns consistent tiers.
fn word_initials(s: &str) -> Vec<char> {
    let mut out = Vec::new();
    for (i, c) in s.chars().enumerate() {
        if c == '_' || c == ' ' {
            continue;
        }
        if i == 0 || c.is_ascii_uppercase() {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

fn fuzzy_subsequence(prefix_lower: &str, candidate_lower: &str) -> bool {
    let mut pi = prefix_lower.chars();
    let mut target = pi.next();
    for c in candidate_lower.chars() {
        if Some(c) == target {
            target = pi.next();
        }
    }
    target.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_beats_prefix() {
        assert!(prefix_score("user", "user").unwrap() > prefix_score("user", "user_id").unwrap());
    }

    #[test]
    fn prefix_beats_substring() {
        assert!(
            prefix_score("use", "user_id").unwrap() > prefix_score("use", "ab_use_cd").unwrap()
        );
    }

    #[test]
    fn camel_hump_matches() {
        assert_eq!(prefix_score("uN", "userName"), Some(120));
    }

    #[test]
    fn snake_initials_match() {
        assert_eq!(prefix_score("un", "user_name"), Some(110));
    }

    #[test]
    fn fuzzy_as_fallback() {
        assert_eq!(prefix_score("ale", "any_length"), Some(40));
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(prefix_score("xyz", "user_id"), None);
    }

    #[test]
    fn empty_prefix_matches_all() {
        assert_eq!(prefix_score("", "anything"), Some(0));
    }
}
