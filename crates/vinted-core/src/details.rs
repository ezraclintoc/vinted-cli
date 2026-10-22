//! Item details.
//!
//! The legacy JSON endpoint `/api/v2/items/{id}` is currently bot-blocked
//! (403). The public item page (`/items/{id}`) is a normal document and
//! carries the useful metadata in OpenGraph `<head>` tags; we parse those.

use crate::error::Result;
use crate::session::Session;
use crate::site::Site;
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
    pub color: Option<String>,
    pub material: Option<String>,
}

/// Scrape full item metadata for `id`.
///
/// Item pages are sometimes bot-blocked on one country domain but reachable
/// via another (a 307 redirects to the slugged, item-specific page). We try
/// the user's site first, then a couple of fallback domains.
pub async fn fetch_item_detail(
    http: &Client,
    site: &Site,
    session: &Session,
    id: i64,
) -> Result<ItemDetail> {
    let mut attempted = vec![site.tld.clone()];
    for tld in ["fr", "com", "de"] {
        if !attempted.contains(&tld.to_string()) {
            attempted.push(tld.to_string());
        }
    }

    let mut last_status = 0u16;
    let mut last_url = String::new();
    for tld in attempted {
        let tmp = Site { tld };
        let url = format!("{}/items/{id}", tmp.base_url());
        match document_get(http, &tmp, session, &url).await {
            Ok(html) => {
                let detail = og_to_detail(id, &url, &html);
                // If OG tags are missing (page retained meta but no tokens),
                // still return what we parsed.
                return Ok(detail);
            }
            Err(crate::error::Error::Api { status, url, .. }) => {
                last_status = status;
                last_url = url;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    let _ = last_url;
    Err(crate::error::Error::Api {
        status: last_status,
        url: last_url,
        body: String::new(),
    })
}

/// GET a document-style page (session cookie attached), requiring 200.
async fn document_get(http: &Client, site: &Site, session: &Session, url: &str) -> Result<String> {
    let mut headers = reqwest::header::HeaderMap::new();
    let mut put = |name: &str, value: String| {
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(&value),
        ) {
            headers.insert(n, v);
        }
    };
    let base = site.base_url();
    put("User-Agent", session.user_agent.clone());
    put(
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".into(),
    );
    put("Accept-Language", format!("{},en;q=0.5", session.locale));
    put("Accept-Encoding", "gzip, deflate, br, zstd".into());
    put("Connection", "keep-alive".into());
    put("Upgrade-Insecure-Requests", "1".into());
    put("Sec-Fetch-Dest", "document".into());
    put("Sec-Fetch-Mode", "navigate".into());
    put("Sec-Fetch-Site", "none".into());
    put("Sec-Fetch-User", "?1".into());
    put("Referer", base);
    put(
        "Cookie",
        format!("access_token_web={}", session.access_token),
    );
    if let Some(anon) = &session.anon_id {
        put("X-Anon-Id", anon.clone());
    }

    let resp = http.get(url).headers(headers).send().await?;
    // The cookie (access_token_web) was scoped to our original site, so only
    // send it when the URL matches; Vinted tolerates a session cookie from
    // another geo but a wrong one can still trigger challenges.
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(crate::error::Error::Api {
            status,
            url: url.to_string(),
            body: String::new(),
        });
    }
    let html = resp.text().await?;
    Ok(html)
}

fn meta_tag(html: &str, property: &str) -> Option<String> {
    // Scan every `<meta …>` tag and pick the one whose `property`/`name`
    // attribute matches, in either attribute order.
    let mut from = 0;
    while let Some(start) = html[from..].find("<meta") {
        let start = from + start;
        let Some(gt) = html[start..].find('>') else {
            break;
        };
        let tag = &html[start..=start + gt];
        from = start + 5;

        let matches = tag.contains(&format!(r#"property="{property}""#))
            || tag.contains(&format!(r#"name="{property}""#));
        if !matches {
            continue;
        }
        let Some(cpos) = tag.find("content=") else {
            continue;
        };
        let after = &tag[cpos + 8..];
        let Some(quote) = after.chars().next() else {
            continue;
        };
        if quote == '"' || quote == '\'' {
            let Some(end) = after[1..].find(quote) else {
                continue;
            };
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
    let title = meta_tag(html, "og:title")
        .map(|t| {
            t.trim_end_matches(' ')
                .trim_end_matches("| Vinted")
                .trim_end()
                .to_string()
        })
        .unwrap_or_default();
    let (price, currency) = schema_price(html);

    ItemDetail {
        id,
        title,
        description: meta_tag(html, "og:description"),
        image: meta_tag(html, "og:image"),
        url: meta_tag(html, "og:url").unwrap_or_else(|| url.to_string()),
        price,
        currency,
        brand: None,
        size: micro_data(html, "size"),
        condition: micro_data(html, "status"),
        color: micro_data(html, "color"),
        material: micro_data(html, "material"),
    }
}

/// Extract `"price":X` + `"priceCurrency":"YYY"` from the schema.org JSON-LD
/// embedded in item pages.
fn schema_price(html: &str) -> (Option<String>, Option<String>) {
    let price = json_scalar_after(html, "\"price\":");
    let currency = json_scalar_after(html, "\"priceCurrency\":");
    (price, currency)
}

fn json_scalar_after(html: &str, key: &str) -> Option<String> {
    let at = html.find(key)?;
    let rest = &html[at + key.len()..];
    let first = rest.chars().next()?;
    if first == '"' {
        let end = rest[1..].find('"')? + 1;
        return Some(rest[1..end].to_string());
    }
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_string())
    }
}

/// Read `<div … itemProp="name"><span …>VALUE<…>` microdata blobs.
fn micro_data(html: &str, name: &str) -> Option<String> {
    let marker = format!(r#"itemProp="{name}""#);
    let at = html.find(&marker)?;
    let rest = &html[at..];
    // Take a small window to avoid scanning the whole document.
    let window = &rest[..rest.len().min(2000)];
    let segs = window.split('<');
    let mut value = String::new();
    for seg in segs {
        let Some(content) = seg.split_once('>') else {
            continue;
        };
        if content.0.starts_with("script") {
            break;
        }
        if !content.1.is_empty() && value.is_empty() {
            value = content.1.to_string();
            break;
        }
    }
    if value.is_empty() {
        return None;
    }
    let value = value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("\u{a0}", " ")
        .replace("&amp;", "&");
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_og_tags() {
        let html = r#"<head><meta property="og:title" content="Nike Air Max 90 | Vinted"></head>"#;
        let d = og_to_detail(1, "https://www.vinted.com/items/1", html);
        assert_eq!(d.title, "Nike Air Max 90");
    }

    #[test]
    fn parses_schema_price_and_microdata() {
        let html = "</script>\"priceCurrency\":\"USD\",\"price\":9,\"availability\"</script>"
            .to_string()
            + "<div itemProp=\"size\"><span class=\"x\">S / US 4-6</span></div>"
            + "<div itemProp=\"status\"><span class=\"b\">Very good</span></div>";
        let d = og_to_detail(1, "https://x.com/items/1", &html);
        assert_eq!(d.price.as_deref(), Some("9"));
        assert_eq!(d.currency.as_deref(), Some("USD"));
        assert_eq!(d.size.as_deref(), Some("S / US 4-6"));
        assert_eq!(d.condition.as_deref(), Some("Very good"));
    }
}
