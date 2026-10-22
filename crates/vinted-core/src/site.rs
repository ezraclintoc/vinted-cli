//! Vinted site (country domain) definitions.

use crate::error::{Error, Result};

/// One Vinted marketplace domain ("country"), e.g. `vinted.com` or `vinted.fr`.
///
/// The session is bootstrapped on the `www.` host, while the catalogue JSON API
/// lives on the `api.` sibling host. Seller/user lookups still live on the
/// legacy `www.` host under `/api/v2/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// Domain suffix after `vinted.`, e.g. `com`.
    pub tld: String,
}

/// Domains currently known to exist (mirrors live Vinted deployments).
pub const KNOWN_TLDS: &[&str] = &[
    "com", "fr", "de", "at", "it", "es", "nl", "pt", "lt", "lu", "cz", "sk", "pl", "se", "ro",
    "hu", "fi", "gr", "ie", "hr", "bg", "ee", "si", "hb", "co.uk", "be",
];

impl Site {
    /// Parse a site from a user-provided string. Accepts:
    /// `vinted.com`, `www.vinted.com`, `api.vinted.com`,
    /// `https://www.vinted.com`, `com`, `fr`, `co.uk`.
    pub fn parse(input: &str) -> Result<Self> {
        let mut s = input.trim().to_lowercase();
        if let Some(rest) = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))
        {
            s = rest.to_string();
        }
        let host = s.trim_end_matches('/').trim_start_matches("www.");
        let host = host.trim_start_matches("api.");
        let tld = match host.strip_prefix("vinted.") {
            Some(t) => t.to_string(),
            None => {
                if KNOWN_TLDS.contains(&host) {
                    host.to_string()
                } else {
                    return Err(Error::InvalidSite(input.to_string()));
                }
            }
        };
        if is_plausible_tld(&tld) {
            return Ok(Site { tld });
        }
        Err(Error::InvalidSite(input.to_string()))
    }

    /// The `www.` base URL, e.g. `https://www.vinted.com`.
    pub fn base_url(&self) -> String {
        format!("https://www.vinted.{}", self.tld)
    }

    /// The catalogue API base URL, e.g. `https://api.vinted.com`.
    pub fn api_url(&self) -> String {
        format!("https://api.vinted.{}", self.tld)
    }

    /// Default locale, e.g. `de-DE`. Used for the `Locale` / `Accept-Language`
    /// headers and to steer currency.
    pub fn default_locale(&self) -> &'static str {
        match self.tld.as_str() {
            "com" => "en-US",
            "co.uk" => "en-GB",
            "at" => "de-AT",
            "be" => "fr-BE",
            "cz" => "cs-CZ",
            "de" => "de-DE",
            "dk" => "da-DK",
            "ee" => "et-EE",
            "es" => "es-ES",
            "fi" => "fi-FI",
            "fr" => "fr-FR",
            "gr" => "el-GR",
            "hr" => "hr-HR",
            "hu" => "hu-HU",
            "ie" => "en-IE",
            "it" => "it-IT",
            "lt" => "lt-LT",
            "lu" => "fr-LU",
            "nl" => "nl-NL",
            "pl" => "pl-PL",
            "pt" => "pt-PT",
            "ro" => "ro-RO",
            "se" => "sv-SE",
            "sk" => "sk-SK",
            "si" => "sl-SI",
            _ => "en-US",
        }
    }

    /// Default currency for the marketplace, e.g. `EUR`.
    pub fn default_currency(&self) -> &'static str {
        match self.tld.as_str() {
            "com" => "USD",
            "co.uk" => "GBP",
            "se" | "dk" => "EUR",
            "pl" => "PLN",
            "cz" => "CZK",
            "hu" => "HUF",
            _ => "EUR",
        }
    }
}

fn is_plausible_tld(tld: &str) -> bool {
    KNOWN_TLDS.contains(&tld)
        || (tld.len() >= 2 && tld.len() <= 6 && tld.chars().all(|c| c.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_various_forms() {
        assert_eq!(Site::parse("vinted.com").unwrap().tld, "com");
        assert_eq!(Site::parse("www.vinted.fr").unwrap().tld, "fr");
        assert_eq!(
            Site::parse("https://www.vinted.co.uk").unwrap().tld,
            "co.uk"
        );
        assert_eq!(Site::parse("co.uk").unwrap().tld, "co.uk");
        assert_eq!(Site::parse("https://api.vinted.com").unwrap().tld, "com");
    }

    #[test]
    fn urls() {
        let s = Site::parse("com").unwrap();
        assert_eq!(s.base_url(), "https://www.vinted.com");
        assert_eq!(s.api_url(), "https://api.vinted.com");
        assert_eq!(s.default_locale(), "en-US");
        assert_eq!(s.default_currency(), "USD");
    }
}
