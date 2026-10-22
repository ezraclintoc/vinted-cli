//! An anonymous Vinted session: the identity Vinted's API expects.
//!
//! Flow (mirrors what a browser does):
//! 1. `GET https://www.vinted.{tld}/` with browser-like document headers.
//! 2. Store the `access_token_web` cookie set in the response.
//! 3. Store the `X-Anon-Id` response header.
//! 4. Send both on every subsequent API call (`Cookie` + `X-Anon-Id`).

use crate::error::{Error, Result};
use crate::site::Site;
use crate::ua;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Session {
    pub access_token: String,
    pub anon_id: Option<String>,
    pub locale: String,
    pub user_agent: String,
}

const SESSION_COOKIE: &str = "access_token_web";

impl Session {
    /// Fetch a fresh anonymous session from the site's landing page.
    pub async fn fetch(
        http: &reqwest::Client,
        site: &Site,
        locale: Option<String>,
    ) -> Result<Self> {
        let user_agent = ua::random_user_agent().to_string();
        let locale = locale.unwrap_or_else(|| site.default_locale().to_string());
        let url = format!("{}/", site.base_url());

        let mut last_status: Option<u16> = None;
        for attempt in 0..4u32 {
            let resp = http
                .get(&url)
                .headers(browser_document_headers(site, &user_agent, &locale)?)
                .send()
                .await?;

            let status = resp.status().as_u16();
            last_status = Some(status);

            if status == 200 {
                let mut token = None;
                for cookie in resp.cookies() {
                    if cookie.name() == SESSION_COOKIE {
                        token = Some(cookie.value().to_string());
                    }
                }
                let anon_id = resp
                    .headers()
                    .get("x-anon-id")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                if let Some(access_token) = token {
                    return Ok(Session {
                        access_token,
                        anon_id,
                        locale,
                        user_agent,
                    });
                }
            }

            // Landing page came back as a bot challenge or error: back off and retry.
            tokio::time::sleep(Duration::from_millis(400 * 2u64.saturating_pow(attempt))).await;
        }

        Err(Error::Session {
            status: last_status.unwrap_or(0),
            site: site.tld.clone(),
        })
    }

    /// Try to refresh an expired token in place.
    pub async fn refresh(&self, http: &reqwest::Client, site: &Site) -> Result<Self> {
        let url = format!("{}/auth/token_refresh", site.base_url());
        let resp = http
            .post(&url)
            .headers(self.api_headers(site, "https://www.vinted", "")?)
            .send()
            .await?;

        if resp.status().as_u16() == 200 {
            for cookie in resp.cookies() {
                if cookie.name() == SESSION_COOKIE {
                    return Ok(Session {
                        access_token: cookie.value().to_string(),
                        ..self.clone()
                    });
                }
            }
        }
        // Fall back to a full re-bootstrap.
        Session::fetch(http, site, Some(self.locale.clone())).await
    }

    /// Full header map for API requests, including the session identity.
    pub fn api_headers(
        &self,
        _site: &Site,
        origin: &str,
        _csrf: &str,
    ) -> Result<reqwest::header::HeaderMap> {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

        let mut h = HeaderMap::new();
        let put = |h: &mut HeaderMap, name: &str, value: String| {
            h.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("static header name"),
                HeaderValue::from_str(&value).expect("header value"),
            );
        };

        let ua = &self.user_agent;
        put(&mut h, "User-Agent", ua.clone());
        put(&mut h, "Accept", "application/json, text/plain, */*".into());
        put(
            &mut h,
            "Accept-Language",
            format!("{},en;q=0.5", self.locale),
        );
        put(&mut h, "Accept-Encoding", "gzip, deflate, br, zstd".into());
        put(&mut h, "Cache-Control", "no-cache".into());
        put(&mut h, "Pragma", "no-cache".into());
        put(&mut h, "Origin", origin.to_string());
        put(&mut h, "Referer", origin.to_string());
        put(&mut h, "Locale", self.locale.clone());
        put(&mut h, "Platform", "web".into());
        put(&mut h, "X-Next-App", "marketplace-web".into());
        put(&mut h, "Sec-Fetch-Dest", "empty".into());
        put(&mut h, "Sec-Fetch-Mode", "cors".into());
        put(&mut h, "Sec-Fetch-Site", "same-site".into());
        if let Some(anon) = &self.anon_id {
            put(&mut h, "X-Anon-Id", anon.clone());
        }
        put(
            &mut h,
            "Cookie",
            format!("{SESSION_COOKIE}={}", self.access_token),
        );
        Ok(h)
    }
}

/// Browser-like *document* headers used only for the landing page request.
fn browser_document_headers(
    site: &Site,
    user_agent: &str,
    locale: &str,
) -> Result<reqwest::header::HeaderMap> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let base = site.base_url();
    let mut h = HeaderMap::new();
    let put = |h: &mut HeaderMap, name: &str, value: String| {
        h.insert(
            HeaderName::from_bytes(name.as_bytes()).expect("static header name"),
            HeaderValue::from_str(&value).expect("header value"),
        );
    };
    put(&mut h, "User-Agent", user_agent.to_string());
    put(
        &mut h,
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".into(),
    );
    put(&mut h, "Accept-Language", format!("{locale},en;q=0.5"));
    put(&mut h, "Accept-Encoding", "gzip, deflate, br, zstd".into());
    put(&mut h, "Connection", "keep-alive".into());
    put(&mut h, "Upgrade-Insecure-Requests", "1".into());
    put(&mut h, "Sec-Fetch-Dest", "document".into());
    put(&mut h, "Sec-Fetch-Mode", "navigate".into());
    put(&mut h, "Sec-Fetch-Site", "none".into());
    put(&mut h, "Sec-Fetch-User", "?1".into());
    put(&mut h, "Origin", base.clone());
    put(&mut h, "Referer", base);
    Ok(h)
}
