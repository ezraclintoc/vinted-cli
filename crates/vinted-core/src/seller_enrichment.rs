//! Custom "seller quality" filtering — beyond what Vinted's own search
//! supports. Vinted lets you search for items; here you can additionally
//! require the seller to have (enough, good) reviews, or exclude given
//! sellers.
//!
//! Seller profiles are fetched in bounded concurrency (default 8) with a
//! per-run cache so repeated sellers cost one request each.

use crate::client::VintedClient;
use crate::error::Result;
use crate::models::{SearchResponse, SellerInfo};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;

/// Knobs for custom seller filtering.
#[derive(Debug, Clone, Default)]
pub struct SellerFilter {
    /// Only keep items whose seller has at least one (positive) review.
    pub has_reviews: bool,
    /// Require at least N feedback (positive+neutral+negative counted).
    pub min_feedback: Option<i64>,
    /// Require feedback reputation >= X (0..=1, e.g. 0.95).
    pub min_reputation: Option<f64>,
    /// Only professional/business sellers.
    pub business_only: Option<bool>,
    /// Only amateur (non-business) sellers.
    pub amateur_only: bool,
    /// Excluded seller logins (case-insensitive).
    pub exclude_logins: Vec<String>,
    /// Excluded seller ids.
    pub exclude_ids: Vec<i64>,
    /// Max concurrent profile requests.
    pub concurrency: usize,
    /// How the number of remaining items is reported back to the user.
    pub show_filter_progress: bool,
}

impl SellerFilter {
    pub fn is_active(&self) -> bool {
        self.has_reviews
            || self.min_feedback.is_some()
            || self.min_reputation.is_some()
            || self.exclude_logins.iter().any(|_| true)
            || self.exclude_ids.iter().any(|_| false)
            || !self.exclude_ids.is_empty()
            || self.business_only.is_some()
            || self.amateur_only
    }
}

/// Result of applying a [`SellerFilter`].
pub struct EnrichedSearch {
    pub response: SearchResponse,
    /// Sellers fetched during enrichment, keyed by user id.
    pub sellers: HashMap<i64, SellerInfo>,
    /// How many items were dropped by the seller filter.
    pub dropped: usize,
}

/// Apply seller settings to a search response.
pub async fn apply_seller_filter(
    client: &VintedClient,
    response: SearchResponse,
    filter: &SellerFilter,
) -> Result<EnrichedSearch> {
    let mut items = response.items;
    let mut dropped = 0usize;

    // Cheap local checks first (no network).
    items.retain(|item| {
        let user = item.user.as_ref();
        let keep = match user {
            None => false,
            Some(u) => {
                if filter
                    .exclude_logins
                    .iter()
                    .any(|l| l.eq_ignore_ascii_case(&u.login))
                {
                    return false;
                }
                if filter.exclude_ids.contains(&u.id) {
                    return false;
                }
                match filter.business_only {
                    Some(true) if !u.business => return false,
                    _ => {}
                }
                if filter.amateur_only && u.business {
                    return false;
                }
                true
            }
        };
        if !keep {
            dropped += 1;
        }
        keep
    });

    // Profile-based checks need a network fetch per unique seller.
    let needs_profile =
        filter.has_reviews || filter.min_feedback.is_some() || filter.min_reputation.is_some();
    if needs_profile {
        let mut uniq: Vec<i64> = items
            .iter()
            .filter_map(|i| i.user.as_ref().map(|u| u.id))
            .collect();
        uniq.sort_unstable();
        uniq.dedup();

        let conc = filter.concurrency.clamp(1, 16);
        let client_ref: Arc<&VintedClient> = Arc::new(client);

        let fetches = futures::stream::iter(uniq)
            .map(|uid| {
                let c = client_ref.clone();
                async move { (uid, c.seller_info(uid).await.ok()) }
            })
            .buffer_unordered(conc);

        let mut cache: HashMap<i64, SellerInfo> = HashMap::new();
        let mut stream = std::pin::pin!(fetches);
        while let Some((uid, info)) = stream.next().await {
            if let Some(info) = info {
                cache.insert(uid, info);
            }
        }

        items.retain(|item| {
            let Some(u) = item.user.as_ref() else {
                return false;
            };
            let Some(info) = cache.get(&u.id) else {
                dropped += 1;
                return false; // profile fetch failed — drop rather than guess
            };
            let keep = passes(filter, info);
            if !keep {
                dropped += 1;
            }
            keep
        });

        return Ok(EnrichedSearch {
            response: SearchResponse {
                items,
                pagination: response.pagination,
            },
            sellers: cache,
            dropped,
        });
    }

    Ok(EnrichedSearch {
        response: SearchResponse {
            items,
            pagination: response.pagination,
        },
        sellers: HashMap::new(),
        dropped,
    })
}

fn passes(filter: &SellerFilter, info: &SellerInfo) -> bool {
    if filter.has_reviews && info.feedback_count <= 0 {
        return false;
    }
    if let Some(min) = filter.min_feedback
        && info.feedback_count < min
    {
        return false;
    }
    if let Some(min) = filter.min_reputation
        && info.feedback_reputation < min
    {
        return false;
    }
    true
}
