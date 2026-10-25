//! Fuzzy matching for the command palette and pickers.

/// Points for a matched character, and extra for where it sits.
const MATCH: i32 = 1;
const WORD_START: i32 = 8;
const TEXT_START: i32 = 6;
const CONSECUTIVE: i32 = 5;
/// Lost per character skipped between two matches.
const GAP: i32 = 1;

/// Score how well `query` matches `text` as a case-insensitive subsequence.
/// `None` when it doesn't match; higher is better. Matches at word starts and
/// runs of consecutive characters score more, gaps cost, and the best
/// alignment wins, so "mbs" ranks "Modbus Simulator" above "remote member
/// status".
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().take(512).collect();
    let worth: Vec<i32> = (0..text.len())
        .map(|index| {
            let word_start = index == 0 || !text[index - 1].is_alphanumeric();
            MATCH + if word_start { WORD_START } else { 0 }
        })
        .collect();
    let matches =
        |wanted: char, index: usize| text[index].to_lowercase().eq(std::iter::once(wanted));
    let position = |index: usize| i32::try_from(index).unwrap_or(i32::MAX);

    // best[j]: best score with the current query character matched at text[j].
    let mut best: Vec<Option<i32>> = (0..text.len())
        .map(|index| {
            matches(query[0], index).then(|| worth[index] + if index == 0 { TEXT_START } else { 0 })
        })
        .collect();
    for wanted in &query[1..] {
        let mut next = vec![None; text.len()];
        // max over k <= j - 2 of best[k] + GAP * k: the gap to j costs
        // GAP * (j - k - 1), so the position cancels out of the running max.
        let mut earlier: Option<i32> = None;
        for index in 0..text.len() {
            if index >= 2 {
                let k = index - 2;
                earlier = earlier.max(best[k].map(|score| score + GAP * position(k)));
            }
            if !matches(*wanted, index) {
                continue;
            }
            let adjacent = index
                .checked_sub(1)
                .and_then(|previous| best[previous])
                .map(|score| score + CONSECUTIVE);
            let skipped = earlier.map(|score| score - GAP * (position(index) - 1));
            next[index] = adjacent.max(skipped).map(|score| score + worth[index]);
        }
        best = next;
    }
    let found = best.into_iter().flatten().max()?;
    // Prefer shorter texts among equal matches.
    Some(found * 16 - position(text.len().min(256)) / 8)
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
    fn the_best_alignment_wins_over_the_first() {
        // Greedy matching takes the "s" inside "Modbus"; the word start is better.
        let aligned = score("ms", "Modbus Simulator").unwrap();
        let only_inner = score("ms", "Modbus").unwrap();
        assert!(aligned > only_inner, "{aligned} vs {only_inner}");
    }

    #[test]
    fn missing_letters_do_not_match() {
        assert_eq!(score("xyz", "Modbus"), None);
        assert_eq!(score("sm", "Modbus"), None, "order matters");
        assert!(score("", "anything").is_some());
        assert!(score("DARK", "Voltrus dark theme").is_some());
    }
}
