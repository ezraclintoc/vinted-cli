//! Unified error type for the crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("failed to decode Vinted response: {0}")]
    Json(#[from] serde_json::Error),

    #[error("could not obtain a Vinted session (status {status}) for site {site}")]
    Session { status: u16, site: String },

    #[error("unexpected API response (status {status}) from {url}\n  {body}")]
    Api {
        status: u16,
        url: String,
        /// Optional serde error / body snippet for debugging.
        body: String,
    },

    #[error("invalid site `{0}` (use e.g. vinted.com, vinted.fr, vinted.co.uk)")]
    InvalidSite(String),

    #[error("no results returned for `{0}`")]
    NoResults(String),
}
