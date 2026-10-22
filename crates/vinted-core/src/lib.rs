//! `vinted-core` — async Rust client for Vinted's unofficial marketplace API.
//!
//! This crate is intentionally independent of any CLI/UI layer so it can be
//! reused by an MCP server or a web frontend later.

pub mod client;
pub mod details;
pub mod error;
pub mod local_filters;
pub mod models;
pub mod seller_enrichment;
pub mod session;
pub mod site;
pub mod ua;
pub mod url_import;

pub use client::MergedSearch;
pub use client::VintedClient;
pub use details::ItemDetail;
pub use details::has_description;
pub use error::{Error, Result};
pub use local_filters::{CaseMode, LocalFilter};
pub use models::{Item, SearchFilter, SearchResponse, SellerInfo, SortOrder, User};
pub use seller_enrichment::{EnrichedSearch, SellerFilter, apply_seller_filter};
pub use site::Site;
