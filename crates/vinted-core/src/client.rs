//! Main HTTP client.
//!
//! One `VintedClient` owns a reusable `reqwest::Client` and lazily bootstraps
//! an anonymous session, transparently refreshing it on auth failures.

use crate::error::{Error, Result};
use crate::models::{Item, SearchFilter, SearchResponse, SellerInfo, User};
use crate::session::Session;
use crate::site::Site;
use reqwest::Client;
use std::sync::RwLock;
use std::time::Duration;

/// Async client for the unofficial Vinted API.
#[derive(Debug)]
pub struct VintedClient {
    http: Client,
    site: Site,
    currency: Option<String>,
    session: RwLock<Option<Session>>,
}

impl VintedClient {
    /// Create a client for `site` (e.g. `https://www.vinted.com`).
    ///
    /// `currency` optionally forces a currency code (Vinted prices can be
    /// converted server-side via the `currency` query parameter).
    pub fn new(site: &str, currency: Option<String>) -> Result<Self> {
        let site = Site::parse(site)?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()?;
        Ok(Self {
            http,
            site,
            currency,
            session: RwLock::new(None),
        })
    }

    pub fn site(&self) -> &Site {
        &self.site
    }

    async fn session(&self) -> Result<Session> {
        // Fast path: read existing.
        if let Some(s) = self.session.read().ok().and_then(|g| g.clone()) {
            return Ok(s);
        }
        let s = Session::fetch(&self.http, &self.site, None).await?;
        if let Ok(mut w) = self.session.write() {
            *w = Some(s.clone());
        }
        Ok(s)
    }

    async fn invalidate_session(&self) {
        if let Ok(mut w) = self.session.write() {
            *w = None;
        }
    }

    async fn with_retries<T, F, Fut>(&self, op: F) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        F: Fn(reqwest::header::HeaderMap) -> Fut,
        Fut: std::future::Future<Output = Result<(u16, reqwest::Response)>>,
    {
        let mut refreshed = false;
        loop {
            let session = self.session().await?;
            let origin = self.site.base_url();
            let headers = session.api_headers(&self.site, &origin, "")?;
            let (status, resp) = op(headers).await?;

            match status {
                200 => return parse_ok::<T>(resp).await,
                401 | 403 if !refreshed => {
                    refreshed = true;
                    self.invalidate_session().await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    continue;
                }
                404 => return Err(Error::NoResults(String::new())),
                code => {
                    let url = resp.url().to_string();
                    let body_text = resp.text().await.unwrap_or_default();
                    let head: String = body_text.chars().take(200).collect();
                    let body = match serde_json::from_str::<T>(&body_text) {
                        Ok(_) => String::new(),
                        Err(e) => format!("{e} :: raw: {head}"),
                    };
                    return Err(Error::Api {
                        status: code,
                        url,
                        body,
                    });
                }
            }
        }
    }

    /// Run a catalogue search.
    pub async fn search(&self, filter: &SearchFilter) -> Result<SearchResponse> {
        let filter = SearchFilter {
            currency: self
                .currency
                .clone()
                .or_else(|| filter.currency.clone())
                .or_else(|| Some(self.site.default_currency().to_string())),
            ..filter.clone()
        };

        let mut url = format!("{}/svc-catalogue/items", self.site.api_url());
        let pairs = filter.to_query_pairs();
        if !pairs.is_empty() {
            let qs: String = pairs
                .iter()
                .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
                .collect::<Vec<_>>()
                .join("&");
            url.push('?');
            url.push_str(&qs);
        }

        self.with_retries::<SearchResponse, _, _>(|headers| async {
            let resp = self.http.get(&url).headers(headers).send().await?;
            let status = resp.status().as_u16();
            Ok((status, resp))
        })
        .await
    }

    /// Fetch up to `pages` consecutive pages, merging and de-duplicating items.
    ///
    /// Stops early when the API reports no further pages or returns an empty
    /// page. A short pause between requests keeps us polite with upstream.
    pub async fn search_pages(&self, filter: &SearchFilter, pages: u32) -> Result<MergedSearch> {
        let mut all_items: Vec<Item> = Vec::new();
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut pages_fetched = 0u32;
        let mut total_entries = 0i64;
        let mut last_pagination: Option<crate::models::Pagination> = None;

        for page in 1..=pages.max(1) {
            let mut page_filter = filter.clone();
            page_filter.page = Some(page);
            let resp = self.search(&page_filter).await?;

            if let Some(ref pagination) = resp.pagination {
                total_entries = pagination.total_entries;
                last_pagination = Some(pagination.clone());
            }

            let empty = resp.items.is_empty();
            for item in resp.items {
                if seen.insert(item.id) {
                    all_items.push(item);
                }
            }
            pages_fetched += 1;

            if empty
                || last_pagination
                    .as_ref()
                    .map(|p| page >= p.total_pages)
                    .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }

        Ok(MergedSearch {
            response: SearchResponse {
                items: all_items,
                pagination: last_pagination,
            },
            pages_fetched,
            total_entries,
        })
    }

    /// Fetch a seller/user profile including review statistics.
    pub async fn user(&self, id_or_login: i64) -> Result<User> {
        let url = format!("{}/api/v2/users/{}", self.site.base_url(), id_or_login);
        self.with_retries::<UserPayload, _, _>(|headers| async {
            let resp = self.http.get(&url).headers(headers).send().await?;
            let status = resp.status().as_u16();
            Ok((status, resp))
        })
        .await?
        .user
        .ok_or_else(|| Error::NoResults(format!("user {id_or_login} not found")))
    }

    /// Full item metadata (title, price, brand, size, photos…).
    ///
    /// Served from the public item page's OpenGraph tags because the legacy
    /// JSON detail endpoint is bot-blocked.
    pub async fn item(&self, id: i64) -> Result<crate::details::ItemDetail> {
        let session = self.session().await?;
        crate::details::fetch_item_detail(&self.http, &self.site, &session, id).await
    }

    /// Whether an item's page carries a non-empty seller description.
    /// (Search results don't include descriptions; this fetches the item
    /// page, so treat it as the expensive per-item check it is.)
    pub async fn item_has_description(&self, id: i64) -> Result<bool> {
        Ok(crate::details::has_description(&self.item(id).await?))
    }

    /// Enrich a single seller (used by the seller-review filters).
    pub async fn seller_info(&self, id: i64) -> Result<SellerInfo> {
        let user: User = self.user(id).await?;
        Ok(SellerInfo {
            id: user.id,
            login: user.login,
            feedback_count: user.feedback_count,
            feedback_reputation: user.feedback_reputation,
            positive_feedback_count: user.positive_feedback_count,
            neutral_feedback_count: user.neutral_feedback_count,
            negative_feedback_count: user.negative_feedback_count,
            city: user.city,
            country_id: None,
            is_business: user.business,
        })
    }
}

/// Result of [`VintedClient::search_pages`].
#[derive(Debug, Clone)]
pub struct MergedSearch {
    /// Merged, de-duplicated items across fetched pages.
    pub response: SearchResponse,
    /// How many pages were actually fetched.
    pub pages_fetched: u32,
    /// Total items the server reports (across all pages).
    pub total_entries: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct UserPayload {
    #[serde(default)]
    user: Option<User>,
}

async fn parse_ok<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let bytes = resp.bytes().await.map_err(Error::Http)?;
    match serde_json::from_slice::<T>(&bytes) {
        Ok(v) => Ok(v),
        Err(mut e) => {
            // Attach a raw-body snippet to the serde message for debugging.
            let raw = String::from_utf8_lossy(&bytes);
            if !raw.is_empty() {
                e = serde::de::Error::custom(format!(
                    "{}\n  raw-head: {}",
                    e,
                    &raw[..raw.len().min(280)]
                ));
            }
            Err(Error::Json(e))
        }
    }
}

async fn _decode_unused() {}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod live_probe {
    //! Manual probes against the real API; not run in CI.
    //! `cargo test -p vinted-core -- --ignored --nocapture`

    use super::*;

    #[tokio::test]
    #[ignore = "hits the real API; run manually with --ignored"]
    async fn probe_brand_filter_post_bodies() {
        let http = Client::builder().build().unwrap();
        let site = crate::site::Site {
            tld: "com".to_string(),
        };
        let session = crate::session::Session::fetch(&http, &site, None)
            .await
            .unwrap();
        let origin = site.base_url();

        let bodies = [
            (
                "minimal",
                r#"{"brand_ids":[14],"search_text":"sweatshirt"}"#,
            ),
            (
                "full",
                r#"{"brand_ids":[14],"search_text":"sweatshirt","currency":"EUR","order":"price_high_to_low","per_page":4}"#,
            ),
        ];
        for (label, body) in &bodies {
            let url = format!("{}/svc-catalogue/items", site.api_url());
            let headers = session.api_headers(&site, &origin, "").unwrap();
            let resp = http
                .post(&url)
                .headers(headers)
                .header("Content-Type", "application/json")
                .body(*body)
                .send()
                .await
                .unwrap();
            let text = resp.text().await.unwrap_or_default();
            let out = match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => v
                    .get("items")
                    .and_then(|i| i.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .map(|it| {
                                let t = it["title"].as_str().unwrap_or("");
                                format!("{}:{:?} ", it["id"], &t[..t.len().min(28)])
                            })
                            .collect::<String>()
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "post got JSON but no items: {:?}",
                            &text[..text.len().min(120)]
                        )
                    }),
                Err(_) => format!("non-json: {}", &text[..text.len().min(120)]),
            };
            println!("POST ({label}) | {out}");
        }
    }

    #[tokio::test]
    #[ignore = "hits the real API; run manually with --ignored"]
    async fn probe_brand_filter_param_styles() {
        let http = Client::builder().build().unwrap();
        let site = crate::site::Site {
            tld: "com".to_string(),
        };
        let session = crate::session::Session::fetch(&http, &site, None)
            .await
            .unwrap();
        let origin = site.base_url();

        let styles = [
            ("none", String::new()),
            ("brand_ids=14", "brand_ids=14".into()),
            ("brand_ids[]=14", "brand_ids[]=14".into()),
            ("brand[]=14", "brand[]=14".into()),
            ("brand=14", "brand=14".into()),
            ("filter_brand_ids=14", "filter_brand_ids=14".into()),
        ];
        for (label, extra) in styles {
            let url = format!(
                "{}/svc-catalogue/items?currency=EUR&order=price_high_to_low\
                 &per_page=4&search_text=sweatshirt&{extra}",
                site.api_url()
            );
            let headers = session.api_headers(&site, &origin, "").unwrap();
            let resp = http.get(&url).headers(headers).send().await.unwrap();
            let text = resp.text().await.unwrap_or_default();
            let out = match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => v
                    .get("items")
                    .and_then(|i| i.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .map(|it| {
                                let t = it["title"].as_str().unwrap_or("");
                                format!("{}:{:?} ", it["id"], &t[..t.len().min(28)])
                            })
                            .collect::<String>()
                    })
                    .unwrap_or_else(|| format!("no-items key: {text:?}")),
                Err(_) => format!("non-json: {}", &text[..text.len().min(60)]),
            };
            println!("[q] {label:>19} | {out}");
        }
        // Same styles WITHOUT a text query — the endpoint may treat text
        // searches differently from pure facet searches.
        for (label, extra) in [
            ("none", String::new()),
            ("brand_ids[]=14", "brand_ids[]=14".into()),
            ("brand[]=14", "brand[]=14".into()),
            ("brand_ids=14", "brand_ids=14".into()),
            ("brand[]=adidas", "brand[]=adidas".into()),
            ("brand_ids[]=adidas", "brand_ids[]=adidas".into()),
            ("brand_slug[]=adidas", "brand_slug[]=adidas".into()),
            ("brand_title[]=adidas", "brand_title[]=adidas".into()),
            ("catalog[]=1203", "catalog[]=1203".into()),
            ("catalog_ids[]=1203", "catalog_ids[]=1203".into()),
        ] {
            let url = format!(
                "{}/svc-catalogue/items?currency=EUR&order=price_high_to_low&per_page=4&{extra}",
                site.api_url()
            );
            let headers = session.api_headers(&site, &origin, "").unwrap();
            let resp = http.get(&url).headers(headers).send().await.unwrap();
            let text = resp.text().await.unwrap_or_default();
            let out = match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => v
                    .get("items")
                    .and_then(|i| i.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .map(|it| {
                                let t = it["title"].as_str().unwrap_or("");
                                format!("{}:{:?} ", it["id"], &t[..t.len().min(28)])
                            })
                            .collect::<String>()
                    })
                    .unwrap_or_else(|| format!("no-items key: {text:?}")),
                Err(_) => format!("non-json: {}", &text[..text.len().min(60)]),
            };
            println!("[no-q] {label:>19} | {out}");
        }
    }
}
