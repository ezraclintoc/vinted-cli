//! `vinted-core` — async Rust client for Vinted's unofficial marketplace API.
//!
//! This crate is intentionally independent of any CLI/UI layer so it can be
//! reused by an MCP server or a web frontend later.

pub mod client;
pub mod details;
pub mod error;
pub mod models;
pub mod seller_enrichment;
pub mod session;
pub mod site;
pub mod ua;

pub use client::VintedClient;
pub use details::ItemDetail;
pub use error::{Error, Result};
pub use models::{Item, SearchFilter, SearchResponse, SellerInfo, SortOrder, User};
pub use seller_enrichment::{apply_seller_filter, EnrichedSearch, SellerFilter};
pub use site::Site;
