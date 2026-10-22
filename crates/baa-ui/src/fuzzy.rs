//! Command-bar matching: case-insensitive subsequence with bonuses for
//! consecutive runs and word starts, so "gh" ranks "GitHub" over "lightshow".

/// `None` when `query` is not a subsequence of `text`. Higher is better.
pub fn score(query: &str, text: &str) -> Option<u32> {
    let mut score = 0;
    let mut after_last_match = None;
    let mut prev_char = ' ';
    let mut chars = text.char_indices();
    for q in query.chars().filter(|c| !c.is_whitespace()) {
        loop {
            let (i, c) = chars.next()?;
            let word_start =
                !prev_char.is_alphanumeric() || (prev_char.is_lowercase() && c.is_uppercase());
            prev_char = c;
            if !c.to_lowercase().eq(q.to_lowercase()) {
                continue;
            }
            score += 1
                + if word_start { 8 } else { 0 }
                + if after_last_match == Some(i) { 5 } else { 0 };
            after_last_match = Some(i + c.len_utf8());
            break;
        }
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_required() {
        assert!(score("abc", "a_b_c").is_some());
        assert!(score("abd", "abc").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn case_insensitive() {
        assert!(score("GIT", "github").is_some());
    }

    #[test]
    fn word_starts_and_runs_win() {
        assert!(score("gh", "GitHub").unwrap() > score("gh", "lightshow").unwrap());
        assert!(score("doc", "docs.rs").unwrap() > score("doc", "adxoxc").unwrap());
    }
}
