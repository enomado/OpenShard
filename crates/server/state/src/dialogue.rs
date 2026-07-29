//! What a townsperson says: the keyword table a trade answers by.
//!
//! Shared substrate, not rules. The `npc` crate decides *when* an NPC speaks and
//! to whom; this is only the data it reads, so it lives below it the way
//! [`QuestDefs`](crate::QuestDefs) lives below the quest system.
//!
//! # Where the content comes from, and why the core still has some
//!
//! ServUO gives the *mechanism* — `VendorAI.OnSpeech` matches shop keywords within
//! four tiles, `HandlesOnSpeech` gates it, and XmlSpawner's `XmlDialog` attachment
//! is the keyword-and-response engine for talking NPCs — but almost no
//! per-profession content: a vendor's whole vocabulary is cliloc 500186
//! ("Greetings.  Have a look around.") and 501522 ("I shall not treat with scum
//! like thee!"). So the split is the one `magic::spells` and `quests` use: the
//! mechanism and a working default in the core, the real lines registered as data
//! by the pack, keyed by the trade.
//!
//! # Keyed by the trade string, never by an index
//!
//! The key is the [`Title`](crate::components::Title) the pack spawns the NPC with
//! — "the blacksmith". An index would silently move every line in the table the
//! day someone reorders the pack's list, and unlike a quest there is nothing in a
//! save to notice it went wrong.

use std::collections::HashMap;

/// Everything one trade says.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct SpeechTable {
    /// Lines it greets an approaching player with. `{name}` is replaced with the
    /// visitor's name; a line without it is used for a nameless visitor too.
    pub greetings: Vec<String>,
    /// What it says to itself when nobody is near — ambient colour. Empty is
    /// silence, which is the right default for most trades.
    pub barks: Vec<String>,
    /// Keyword groups and the answers to them. The first group with a match wins,
    /// so the pack's order is its precedence — a specific keyword goes above a
    /// general one.
    pub entries: Vec<SpeechEntry>,
    /// What it says when spoken to and nothing matched. `None` stays quiet, which
    /// is better than a shopkeeper answering every passing conversation.
    pub fallback: Option<String>,
}

/// One keyword group and its answers.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct SpeechEntry {
    /// The words that trigger it, lowercase. Matched as whole words against what
    /// was said — a substring match is how "unsellable" opened a shop.
    pub keywords: Vec<String>,
    /// The answers, one picked at random so an NPC asked twice does not repeat.
    pub lines: Vec<String>,
}

impl SpeechEntry {
    /// Whether `words` — already lowercased and split — contains any of the
    /// keywords. A multi-word keyword ("vendor buy") matches as a run.
    #[must_use]
    pub fn matches(&self, words: &[&str]) -> bool {
        self.keywords.iter().any(|keyword| {
            let wanted: Vec<&str> = keyword.split_whitespace().collect();
            match wanted.len() {
                0 => false,
                1 => words.contains(&wanted[0]),
                n => words.windows(n).any(|window| window == wanted.as_slice()),
            }
        })
    }
}

/// Every trade's speech, and the personal names the pack wants used, by key.
///
/// Replaced wholesale by the pack, like [`QuestDefs`](crate::QuestDefs): a hot
/// reload re-runs registration from the top, and merging would leave lines the pack
/// has deleted still being spoken.
#[derive(Clone, Default, Debug)]
pub struct Dialogue {
    tables: HashMap<String, SpeechTable>,
    /// Personal names the pack registered, male then female. Empty falls back to
    /// the core's own lists — see `npc::names`.
    male_names: Vec<String>,
    female_names: Vec<String>,
}

impl Dialogue {
    /// Replace every trade's table.
    pub fn set_tables(&mut self, tables: HashMap<String, SpeechTable>) {
        self.tables = tables;
    }

    /// Replace the personal-name lists. Either being empty leaves the core's own
    /// list in use for that gender, so a pack may supply one and not the other.
    pub fn set_names(&mut self, male: Vec<String>, female: Vec<String>) {
        self.male_names = male;
        self.female_names = female;
    }

    /// The table for a trade, if the pack defines one.
    #[must_use]
    pub fn table(&self, title: &str) -> Option<&SpeechTable> {
        self.tables.get(title)
    }

    /// The registered names for a gender, or an empty slice when the pack gave none.
    #[must_use]
    pub fn names(&self, female: bool) -> &[String] {
        if female {
            &self.female_names
        } else {
            &self.male_names
        }
    }

    /// Whether the pack registered anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty() && self.male_names.is_empty() && self.female_names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(keywords: &[&str]) -> SpeechEntry {
        SpeechEntry {
            keywords: keywords.iter().map(|k| (*k).to_owned()).collect(),
            lines: vec!["aye".to_owned()],
        }
    }

    #[test]
    fn a_keyword_matches_a_whole_word_and_not_a_substring() {
        // The bug this replaces: "sell" was matched as a substring, so saying
        // "that sword is unsellable" opened a shopkeeper's buy-back list.
        let e = entry(&["sell"]);
        assert!(e.matches(&["i", "wish", "to", "sell"]));
        assert!(!e.matches(&["unsellable"]));
        assert!(!e.matches(&["seller"]));
    }

    #[test]
    fn a_multi_word_keyword_matches_only_in_order() {
        // ServUO's `vendor buy` keyword is two words and means nothing apart.
        let e = entry(&["vendor buy"]);
        assert!(e.matches(&["vendor", "buy"]));
        assert!(e.matches(&["hey", "vendor", "buy", "please"]));
        assert!(!e.matches(&["buy", "vendor"]));
        assert!(!e.matches(&["vendor"]));
    }

    #[test]
    fn an_empty_keyword_never_matches() {
        // A stray comma in the pack's data would otherwise answer everything.
        assert!(!entry(&[""]).matches(&["anything"]));
    }

    #[test]
    fn a_table_is_replaced_wholesale_not_merged() {
        let mut dialogue = Dialogue::default();
        let mut first = HashMap::new();
        first.insert("the baker".to_owned(), SpeechTable::default());
        dialogue.set_tables(first);
        assert!(dialogue.table("the baker").is_some());

        let mut second = HashMap::new();
        second.insert("the smith".to_owned(), SpeechTable::default());
        dialogue.set_tables(second);
        assert!(
            dialogue.table("the baker").is_none(),
            "a reload must drop what the pack deleted"
        );
    }

    #[test]
    fn names_fall_back_per_gender() {
        let mut dialogue = Dialogue::default();
        dialogue.set_names(vec!["Aaron".to_owned()], Vec::new());
        assert_eq!(dialogue.names(false).len(), 1);
        assert!(
            dialogue.names(true).is_empty(),
            "an empty list means the core's own is used"
        );
    }
}
