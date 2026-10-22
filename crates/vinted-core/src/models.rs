//! Data models mirroring Vinted's catalogue API responses.
//!
//! Responses are deserialized leniently: unknown fields are ignored and most
//! optional fields are `Option` so live API changes don't break the crate.

use serde::{Deserialize, Serialize};

/// A physical article condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Condition {
    /// 1 — new with tag
    NewWithTag,
    /// 2 — new without tag
    NewNoTag,
    /// 3 — very good
    VeryGood,
    /// 4 — good
    Good,
    /// 5 — satisfactory
    Satisfactory,
}

impl Condition {
    pub fn id(self) -> u16 {
        match self {
            Self::NewWithTag => 1,
            Self::NewNoTag => 2,
            Self::VeryGood => 3,
            Self::Good => 4,
            Self::Satisfactory => 5,
        }
    }
}

/// Result ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SortOrder {
    /// 2020 — relevance (Vinted's own scoring)
    Relevance,
    /// new — newest first
    NewestFirst,
    /// price_low_to_high (also accepts price_low_to_high)
    PriceLowToHigh,
    /// price_high_to_low
    PriceHighToLow,
}

impl SortOrder {
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::NewestFirst => "newest_first",
            Self::PriceLowToHigh => "price_low_to_high",
            Self::PriceHighToLow => "price_high_to_low",
        }
    }
}

/// A search filter — equivalent to what you can set in Vinted's web UI.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchFilter {
    /// Free text query.
    pub search_text: Option<String>,
    /// Explicit query param (only supported by Vinted, kept for scripts).
    pub query: Option<String>,
    /// Catalog (category) ids, comma separated.
    pub catalog_ids: Option<String>,
    /// Brand ids, comma separated.
    pub brand_ids: Option<String>,
    /// Size ids, comma separated.
    pub size_ids: Option<String>,
    /// Color ids, comma separated.
    pub color_ids: Option<String>,
    /// Article conditions (one or many).
    pub conditions: Vec<Condition>,
    /// Minimum price (in the site's currency units).
    pub price_from: Option<f64>,
    /// Maximum price.
    pub price_to: Option<f64>,
    /// Currency code override (defaults to the site's currency).
    pub currency: Option<String>,
    /// Sort order.
    pub order: Option<SortOrder>,
    /// 1-based page number.
    pub page: Option<u32>,
    /// Items per page (max 100).
    pub per_page: Option<u32>,
    /// Only return items with photos.
    pub with_photo: bool,
    /// Only return items that are `is_visible` aka for sale.
    pub for_sale_only: bool,
}

impl SearchFilter {
    pub(crate) fn to_query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        if let Some(t) = &self.search_text {
            pairs.push(("search_text".into(), t.clone()));
        }
        if let Some(q) = &self.query {
            pairs.push(("query".into(), q.clone()));
        }
        for (key, val) in [
            ("catalog_ids", &self.catalog_ids),
            ("brand_ids", &self.brand_ids),
            ("size_ids", &self.size_ids),
            ("color_ids", &self.color_ids),
        ] {
            if let Some(v) = val {
                pairs.push((key.to_string(), v.clone()));
            }
        }
        if !self.conditions.is_empty() {
            let ids: Vec<String> = self.conditions.iter().map(|c| c.id().to_string()).collect();
            pairs.push(("status_ids".into(), ids.join(",")));
        }
        if let Some(p) = self.price_from {
            pairs.push(("price_from".into(), format!("{p}")));
        }
        if let Some(p) = self.price_to {
            pairs.push(("price_to".into(), format!("{p}")));
        }
        if let Some(c) = &self.currency {
            pairs.push(("currency".into(), c.clone()));
        }
        if let Some(o) = self.order {
            pairs.push(("order".into(), o.api_value().to_string()));
        }
        if let Some(p) = self.page {
            pairs.push(("page".into(), p.to_string()));
        }
        if let Some(p) = self.per_page {
            pairs.push(("per_page".into(), p.min(100).to_string()));
        }
        if self.with_photo {
            pairs.push(("with_photo".into(), "true".into()));
        }
        pairs
    }
}

/// Price with amount + currency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Price {
    #[serde(default)]
    pub amount: String,
    #[serde(default)]
    pub currency_code: String,
}

/// Seller as embedded in a search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemUser {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub login: String,
    /// True for professional/business accounts (`b/pro` shops).
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub business: bool,
}

fn de_bool_lenient<'de, D>(d: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().map(|i| i != 0).unwrap_or(false),
        serde_json::Value::String(s) => s == "true" || s == "1",
        _ => false,
    })
}

/// An image reference (catalogue API shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    #[serde(default)]
    pub id: i64,
    /// Large image URL (query-parameter signed CDN link).
    #[serde(default, rename = "full_size_url")]
    pub full_size_url: String,
    /// Pre-cut thumbnail links: {index, url, zoom_url}.
    #[serde(default)]
    pub thumbnails: Vec<PhotoThumb>,
    /// Main colour of the picture, e.g. `#84929D`.
    #[serde(default, rename = "dominant_color")]
    pub dominant_color_hex: Option<String>,
    #[serde(default)]
    pub high_resolution: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhotoThumb {
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "zoom_url")]
    pub zoom_url: Option<String>,
}

/// An item returned by a catalogue search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub price: Option<Price>,
    /// Buyer's total incl. service fee, when provided.
    #[serde(default)]
    pub total_item_price: Option<Price>,
    #[serde(default)]
    pub photo: Option<Photo>,
    #[serde(default)]
    pub photos: Vec<Photo>,
    #[serde(default)]
    pub user: Option<ItemUser>,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub view_count: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub favourite_count: i64,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub promoted: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_favourite: bool,
}

fn de_i64_lenient<'de, D>(d: D) -> std::result::Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

impl Price {
    pub fn numeric(&self) -> f64 {
        self.amount
            .replace(",", "")
            .trim()
            .parse()
            .unwrap_or(0.0)
    }
}

/// Seller statistics fetched from the user endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SellerInfo {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub login: String,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub feedback_count: i64,
    #[serde(default, deserialize_with = "de_f64_lenient")]
    pub feedback_reputation: f64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub positive_feedback_count: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub neutral_feedback_count: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub negative_feedback_count: i64,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub country_id: Option<i64>,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_business: bool,
}

fn de_f64_lenient<'de, D>(d: D) -> std::result::Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

/// Full profile of a Vinted user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default, deserialize_with = "de_f64_lenient")]
    pub feedback_reputation: f64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub feedback_count: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub positive_feedback_count: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub neutral_feedback_count: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub negative_feedback_count: i64,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub hide_feedback: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub business: bool,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Pagination block in the search response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pagination {
    #[serde(default, alias = "current", deserialize_with = "de_u32_lenient")]
    pub current_page: u32,
    #[serde(default, deserialize_with = "de_u32_lenient")]
    pub total_pages: u32,
    #[serde(default, deserialize_with = "de_u32_lenient")]
    pub per_page: u32,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub total_entries: i64,
}

fn de_u32_lenient<'de, D>(d: D) -> std::result::Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|f| f as u64))
            .map(|x| x as u32)
            .unwrap_or(0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

/// A catalogue search response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

impl SearchResponse {
    /// Best-effort title for an item's brand/size etc. is not part of search
    /// results anymore; interactions needing more detail should use the
    /// OpenGraph item page (see `VintedClient::item`).
    pub fn has_items(&self) -> bool {
        !self.items.is_empty()
    }
}
