//! Local, client-side post-filtering of search results.
//!
//! Vinted's server search is always case-insensitive and has no keyword
//! blocklist; both are implemented here over the returned items so the
//! behaviour is shared between the CLI, the future MCP server and web front.

use crate::models::Item;

/// Controls how the local filters treat letter case.
///
/// Default = [`CaseMode::IgnoreCase`] — matching is forgiving: both the query
/// words and exclude terms match regardless of capitalization. Set
/// [`CaseMode::Exact`] to *force* the user's capitalization: matching is done
/// with exact letter case on titles/descriptions.
///
/// (The upstream catalogue search itself is always case-insensitive; case
/// enforcement can only happen locally after a page is fetched.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaseMode {
    /// Case does not matter — recommended and default.
    #[default]
    IgnoreCase,
    /// Exact capitalization required.
    Exact,
}

#[derive(Debug, Clone, Default)]
pub struct LocalFilter {
    /// Filter out items whose title contains any of these words/phrases.
    pub exclude_text: Vec<String>,
    /// Require each of these words/phrases to appear in the title (word-by-word
    /// containment). Used to enforce the user's capitalization on a
    /// case-sensitive query.
    pub require_words: Vec<String>,
    /// How case is handled for all local text matching.
    pub case: CaseMode,
}

impl LocalFilter {
    pub fn is_active(&self) -> bool {
        !self.exclude_text.is_empty() || !self.require_words.is_empty()
    }

    /// Filter items in place. Also returns how many were dropped.
    pub fn apply(&self, items: Vec<Item>) -> (Vec<Item>, usize) {
        if !self.is_active() {
            return (items, 0);
        }
        let before = items.len();
        let kept: Vec<Item> = items
            .into_iter()
            .filter(|item| !self.excluded(item))
            .collect();
        let dropped = before - kept.len();
        (kept, dropped)
    }

    fn excluded(&self, item: &Item) -> bool {
        // NOTE: item descriptions are not part of search results, so this
        // only filters on (visible) titles today.
        let hay = match self.case {
            CaseMode::IgnoreCase => item.title.to_lowercase(),
            CaseMode::Exact => item.title.clone(),
        };
        let blocked = self.exclude_text.iter().any(|term| {
            let needle = match self.case {
                CaseMode::IgnoreCase => term.to_lowercase(),
                CaseMode::Exact => term.clone(),
            };
            hay.contains(&needle)
        });
        if blocked {
            return true;
        }
        !self.require_words.iter().all(|word| {
            let needle = match self.case {
                CaseMode::IgnoreCase => word.to_lowercase(),
                CaseMode::Exact => word.clone(),
            };
            hay.contains(&needle)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Item, ItemUser, Price};

    fn item(title: &str) -> Item {
        Item {
            id: 1,
            title: title.to_string(),
            url: String::new(),
            price: Some(Price {
                amount: "1".into(),
                currency_code: "USD".into(),
            }),
            total_item_price: None,
            photo: None,
            photos: vec![],
            user: Some(ItemUser {
                id: 7,
                login: "u".into(),
                business: false,
            }),
            view_count: 0,
            favourite_count: 0,
            promoted: false,
            is_favourite: false,
        }
    }

    #[test]
    fn exclude_is_case_insensitive_by_default() {
        let lf = LocalFilter {
            exclude_text: vec!["fake".into()],
            require_words: Default::default(),
            ..Default::default()
        };
        let (kept, dropped) = lf.apply(vec![item("Real LEVI jeans"), item("FAKE leather")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(dropped, 1);
        assert_eq!(kept[0].title, "Real LEVI jeans");
    }

    #[test]
    fn exact_case_is_enforced_with_flag() {
        let lf = LocalFilter {
            exclude_text: vec!["Fake".into()],
            require_words: Default::default(),
            case: CaseMode::Exact,
        };
        let (kept, _dropped) = lf.apply(vec![item("fake watch"), item("Fake watch")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "fake watch");
    }

    #[test]
    fn inactive_filter_drops_nothing() {
        let (kept, dropped) = LocalFilter::default().apply(vec![item("anything")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn require_words_enforce_exact_case() {
        let lf = LocalFilter {
            exclude_text: Default::default(),
            require_words: vec!["Levi".into()],
            case: CaseMode::Exact,
        };
        let (kept, _dropped) = lf.apply(vec![item("LEVI 550 jeans"), item("Levi 550 jeans")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "Levi 550 jeans");
    }
}
