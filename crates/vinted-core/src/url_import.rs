//! Build search filters from a pasted Vinted search URL.
//!
//! Vinted's web UI does not offer name-based lookups of categories/brands to
//! anonymous API consumers (the brand/catalog typeahead endpoints are
//! bot-blocked), but applying filters in the browser *does* put the exact ids
//! in the URL, e.g.:
//!
//! `https://www.vinted.com/items?catalog[]=1203&brand_ids[]=1078&price_to=50`
//!
//! Paste that here and we rebuild the equivalent [`SearchFilter`] — an exact
//! match with whatever the site would have shown, no id-hunting needed.

use crate::models::{Condition, SearchFilter, SortOrder};

/// Parse a `vinted.<tld>/items?...` URL into a partial [`SearchFilter`].
///
/// Errors if the URL is not a Vinted host. Only the recognised parameters are
/// copied; anything unknown is ignored.
pub fn parse_search_url(url: &str) -> Result<SearchFilter, String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("not a valid URL ({e})"))?;

    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    if !host.contains("vinted.") {
        return Err(format!("`{host}` is not a vinted.* URL"));
    }

    let mut filter = SearchFilter::default();
    let mut conditions: Vec<u32> = Vec::new();

    for (key, value) in parsed.query_pairs() {
        let val = value.trim().to_string();
        if val.is_empty() {
            continue;
        }
        match key.to_lowercase().as_str() {
            "q" | "search_text" => filter.search_text = Some(val),
            "catalog[]" | "catalog_ids[]" | "catalog_ids" => {
                filter.catalog_ids = push_id(filter.catalog_ids.take(), &val)
            }
            "brand_ids[]" | "brand_ids" => {
                filter.brand_ids = push_id(filter.brand_ids.take(), &val)
            }
            "size_ids[]" | "size_ids" => filter.size_ids = push_id(filter.size_ids.take(), &val),
            "color_ids[]" | "color_ids" => {
                filter.color_ids = push_id(filter.color_ids.take(), &val)
            }
            "status_ids[]" | "status_ids" => {
                if let Ok(id) = val.parse::<u32>() {
                    conditions.push(id);
                }
            }
            "price_from" => filter.price_from = val.parse().ok(),
            "price_to" => filter.price_to = val.parse().ok(),
            "currency" => filter.currency = Some(val),
            "page" => filter.page = val.parse().ok(),
            "order" => filter.order = parse_order(&val),
            "types[]" => {} // (men/women/kids) not mapped 1:1 in our filter
            _ => {}
        }
    }

    for id in conditions {
        if let Some(c) = condition_from_status_id(id)
            && !filter.conditions.contains(&c)
        {
            filter.conditions.push(c);
        }
    }
    Ok(filter)
}

fn push_id(existing: Option<String>, val: &str) -> Option<String> {
    let mut list = existing.unwrap_or_default();
    if !list.is_empty() {
        list.push(',');
    }
    list.push_str(val);
    Some(list)
}

fn parse_order(v: &str) -> Option<SortOrder> {
    match v.to_lowercase().as_str() {
        "relevance" => Some(SortOrder::Relevance),
        "newest_first" => Some(SortOrder::NewestFirst),
        "price_low_to_high" => Some(SortOrder::PriceLowToHigh),
        "price_high_to_low" => Some(SortOrder::PriceHighToLow),
        _ => None,
    }
}

/// Vinted status ids: 1 new-with-tag … 5 satisfactory.
fn condition_from_status_id(id: u32) -> Option<Condition> {
    match id {
        1 => Some(Condition::NewWithTag),
        2 => Some(Condition::NewNoTag),
        3 => Some(Condition::VeryGood),
        4 => Some(Condition::Good),
        5 => Some(Condition::Satisfactory),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_url() {
        let f = parse_search_url(
            "https://www.vinted.fr/items?catalog[]=1203&catalog[]=1789\
             &brand_ids[]=1078&q=levi%20jacket&price_to=50&status_ids[]=3\
             &order=newest_first&page=2&currency=GBP",
        )
        .unwrap();
        assert_eq!(f.search_text.as_deref(), Some("levi jacket"));
        assert_eq!(f.catalog_ids.as_deref(), Some("1203,1789"));
        assert_eq!(f.brand_ids.as_deref(), Some("1078"));
        assert_eq!(f.price_to, Some(50.0));
        assert_eq!(f.conditions, vec![Condition::VeryGood]);
        assert_eq!(f.order, Some(SortOrder::NewestFirst));
        assert_eq!(f.page, Some(2));
        assert_eq!(f.currency.as_deref(), Some("GBP"));
    }

    #[test]
    fn rejects_foreign_hosts() {
        assert!(parse_search_url("https://example.com/items?q=x").is_err());
    }

    #[test]
    fn ignores_unknown_params() {
        let f = parse_search_url("https://www.vinted.com/items?unknown=1&q=hat").unwrap();
        assert_eq!(f.search_text.as_deref(), Some("hat"));
    }
}
