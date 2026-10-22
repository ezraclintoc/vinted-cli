//! Item details.
//!
//! The legacy JSON endpoint `/api/v2/items/{id}` is currently bot-blocked
//! (403). The public item page (`/items/{id}`) is a normal document and
//! carries the useful metadata in OpenGraph `<head>` tags; we parse those.

use crate::error::Result;
use crate::site::Site;
use crate::session::Session;
use reqwest::Client;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ItemDetail {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub url: String,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub brand: Option<String>,
    pub size: Option<String>,
    pub condition: Option<String>,
}

/// Scrape full item metadata for `id`.
pub async fn fetch_item_detail(
    http: &Client,
    site: &Site,
    session: &Session,
    id: i64,
) -> Result<ItemDetail> {
    let url = format!("{}/items/{id}", site.base_url());
    let headers = session
        .api_headers(site, &site.base_url(), "")?;
    // This goes to the `www.` host, so override the "same-site" fetch header.
    let mut headers = headers;
    if let Ok(v) = reqwest::header::HeaderValue::from_str("same-origin") {
        headers.insert(
            reqwest::header::HeaderName::from_static("sec-fetch-site"),
            v,
        );
    }

    let resp = http.get(&url).headers(headers).send().await?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(crate::error::Error::Api {
            status,
            url,
            body: None,
        });
    }
    let html = resp.text().await?;
    Ok(og_to_detail(id, &url, &html))
}

fn meta_tag<'a>(html: &'a str, property: &str) -> Option<String> {
    // Scan every `<meta …>` tag and pick the one whose `property`/`name`
    // attribute matches, in either attribute order.
    let mut from = 0;
    while let Some(start) = html[from..].find("<meta") {
        let start = from + start;
        let Some(gt) = html[start..].find('>') else { break };
        let tag = &html[start..=start + gt];
        from = start + 5;

        let matches = tag.contains(&format!(r#"property="{property}""#))
            || tag.contains(&format!(r#"name="{property}""#));
        if !matches {
            continue;
        }
        let Some(cpos) = tag.find("content=") else { continue };
        let after = &tag[cpos + 8..];
        let Some(quote) = after.chars().next() else { continue };
        if quote == '"' || quote == '\'' {
            let Some(end) = after[1..].find(quote) else { continue };
            if let Some(val) = json_unescape(&after[1..end + 1]) {
                return Some(val);
            }
        }
    }
    None
}

fn json_unescape(s: &str) -> Option<String> {
    // Several tags embed `&quot;`-style HTML entities; decode the common ones.
    let s = s
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    Some(s)
}

fn og_to_detail(id: i64, url: &str, html: &str) -> ItemDetail {
    ItemDetail {
        id,
        title: meta_tag(html, "og:title").unwrap_or_default(),
        description: meta_tag(html, "og:description"),
        image: meta_tag(html, "og:image"),
        url: meta_tag(html, "og:url").unwrap_or_else(|| url.to_string()),
        price: meta_tag(html, "product:price:amount"),
        currency: meta_tag(html, "product:price:currency"),
        brand: meta_tag(html, "product:brand"),
        size: meta_tag(html, "vinted:size_title"),
        condition: meta_tag(html, "vinted:status"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_og_tags() {
        let html = r#"<head><meta property="og:title" content="Nike Air Max 90"><meta content="42.50" property="product:price:amount"></head>"#;
        let d = og_to_detail(1, "https://www.vinted.com/items/1", html);
        assert_eq!(d.title, "Nike Air Max 90");
        assert_eq!(d.price.as_deref(), Some("42.50"));
    }
}
