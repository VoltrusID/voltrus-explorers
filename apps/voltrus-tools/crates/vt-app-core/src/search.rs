//! Fuzzy matching for the command palette and pickers.

/// Score how well `query` matches `text` as a case-insensitive subsequence.
/// `None` when it doesn't match; higher is better. Consecutive characters and
/// matches at word starts score more, so "mbs" ranks "Modbus Simulator"
/// above "remote branches".
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    let mut score = 0;
    let mut previous: Option<usize> = None;
    // Positions count characters, so adjacency holds for any script.
    let mut chars = text.chars().enumerate();
    let mut last_char: Option<char> = None;
    'query: for wanted in query.chars().flat_map(char::to_lowercase) {
        if wanted.is_whitespace() {
            continue;
        }
        for (index, character) in chars.by_ref() {
            let before = last_char;
            last_char = Some(character);
            if character.to_lowercase().eq(std::iter::once(wanted)) {
                let word_start = before.is_none_or(|before| !before.is_alphanumeric());
                score += 1;
                if word_start {
                    score += 8;
                }
                if previous.is_some_and(|previous| previous + 1 == index) {
                    score += 5;
                }
                previous = Some(index);
                continue 'query;
            }
        }
        return None;
    }
    // Prefer shorter texts among equal matches.
    Some(score * 16 - i32::try_from(text.len().min(256)).unwrap_or(256) / 8)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn word_starts_beat_scattered_letters() {
        let simulator = score("mbs", "Modbus Simulator").unwrap();
        let scattered = score("mbs", "remote member status").unwrap();
        assert!(simulator > scattered, "{simulator} vs {scattered}");
    }

    #[test]
    fn missing_letters_do_not_match() {
        assert_eq!(score("xyz", "Modbus"), None);
        assert!(score("", "anything").is_some());
        assert!(score("DARK", "Voltrus dark theme").is_some());
    }
}
