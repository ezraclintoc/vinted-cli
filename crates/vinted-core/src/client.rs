//! Main HTTP client.
//!
//! One `VintedClient` owns a reusable `reqwest::Client` and lazily bootstraps
//! an anonymous session, transparently refreshing it on auth failures.

use crate::error::{Error, Result};
use crate::site::Site;
use crate::models::{SearchFilter, SearchResponse, SellerInfo, User};
use crate::session::Session;
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
                200 => return Ok(parse_ok::<T>(resp).await?),
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
                    return Err(Error::Api { status: code, url, body });
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

/// Wrapper around the user endpoint payload.
#[derive(Debug, Clone, serde::Deserialize)]
struct UserPayload {
    #[serde(default)]
    user: Option<User>,
}async fn parse_ok<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T> {
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
