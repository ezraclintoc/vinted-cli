//! `vinted mcp serve` — the Model Context Protocol server.
//!
//! Exposes the same capabilities as the CLI as MCP tools over stdio, reusing
//! the identical search pipeline (server filters + local filters + seller
//! quality) and the same saved-search/watch files as `vinted save|diff`.
//!
//! The clear search tool deliberately advertises only the filters Vinted's
//! anonymous endpoint honors (text query, price range, ordering, pagination,
//! local text filters, seller quality). The id-facet filters
//! (`--brand-ids`…) are ignored server-side upstream, so they are not part of
//! the tool schema — dead parameters would just teach agents to send them.

use crate::{SearchArgs, execute_search, now_secs, run_diff, search_json_value, storage};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// MCP server state. One process per harness session; tools create a
/// `VintedClient` per call (cheap: anonymous session fetch), like the CLI.
#[derive(Clone)]
pub(crate) struct VintedMcp {
    site: String,
    currency: Option<String>,
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------------------
// tool input types

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SearchParams {
    /// Free-text query. Vinted's server matches titles AND seller
    /// descriptions; the tool keeps items matched by either (use
    /// `titles_only` for titles only). Required unless `from_url` or a
    /// saved filter URL is given.
    #[serde(default)]
    pub(crate) query: Option<String>,
    /// Minimum price.
    pub(crate) price_from: Option<f64>,
    /// Maximum price.
    pub(crate) price_to: Option<f64>,
    /// One of: relevance, newest-first, price-low-to-high, price-high-to-low.
    pub(crate) order: Option<String>,
    /// Fetch N consecutive pages, merged (deeper = slower, more complete).
    pub(crate) pages: Option<u32>,
    /// Hide items whose title contains any of these terms (comma separated).
    pub(crate) exclude_text: Option<String>,
    /// Ignore matches found only in descriptions: keep only items whose
    /// title contains every query word.
    #[serde(default)]
    pub(crate) titles_only: bool,
    /// Only keep items that have a seller description (one request per item,
    /// noticeably slower).
    #[serde(default)]
    pub(crate) with_description: bool,
    /// Only items with photos.
    #[serde(default)]
    pub(crate) photo_only: bool,
    /// Enforce exact capitalization of the query words.
    #[serde(default)]
    pub(crate) case_sensitive: bool,
    /// Only keep items whose seller has at least one review.
    #[serde(default)]
    pub(crate) seller_has_reviews: bool,
    /// Only keep items whose seller has at least N feedback reviews.
    pub(crate) min_seller_feedback: Option<i64>,
    /// Only keep items whose seller rating (0..=1) is >= F, e.g. 0.95.
    pub(crate) min_seller_rating: Option<f64>,
    /// Only keep items from business/pro sellers.
    #[serde(default)]
    pub(crate) business_only: bool,
    /// Only keep items from private (non-business) sellers.
    #[serde(default)]
    pub(crate) amateur_only: bool,
    /// Exclude sellers by login or id, comma separated.
    pub(crate) exclude_seller: Option<String>,
    /// Build the whole filter from a pasted vinted.com search URL instead of
    /// individual params.
    pub(crate) from_url: Option<String>,
    /// Results per page (max 100).
    pub(crate) limit: Option<u32>,
    /// Marketplace tld override (com, fr, de, …). Default: the server-wide
    /// `--site` the process was launched with.
    pub(crate) site: Option<String>,
}

impl SearchParams {
    fn to_args(&self) -> SearchArgs {
        SearchArgs {
            query: self.query.clone(),
            price_from: self.price_from,
            price_to: self.price_to,
            order: self.order.as_deref().and_then(parse_order),
            pages: self.pages,
            exclude_text: self.exclude_text.clone(),
            case_sensitive: self.case_sensitive,
            limit: self.limit,
            photo: self.photo_only,
            with_description: self.with_description,
            titles_only: self.titles_only,
            from_url: self.from_url.clone(),
            seller_has_reviews: self.seller_has_reviews,
            min_seller_feedback: self.min_seller_feedback,
            min_seller_rating: self.min_seller_rating,
            business_only: self.business_only,
            amateur_only: self.amateur_only,
            exclude_seller: self.exclude_seller.clone(),
            concurrency: 8,
            ..Default::default()
        }
    }
}

/// `relevance | newest-first | price-low-to-high | price-high-to-low`,
/// tolerant of separator/spacing variants.
fn parse_order(s: &str) -> Option<crate::OrderArg> {
    let norm = s.trim().to_lowercase().replace(['-', ' '], "_");
    match norm.as_str() {
        "relevance" => Some(crate::OrderArg::Relevance),
        "newest_first" => Some(crate::OrderArg::NewestFirst),
        "price_low_to_high" => Some(crate::OrderArg::PriceLowToHigh),
        "price_high_to_low" => Some(crate::OrderArg::PriceHighToLow),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct ItemId {
    /// Numeric Vinted item id (from a listing URL).
    pub(crate) id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SellerId {
    /// Numeric Vinted member id (from a seller profile URL).
    pub(crate) id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct WatchName {
    /// Name of the saved search (`watch_save` / `vinted save`).
    pub(crate) name: String,
    /// Override the number of pages fetched for this run (deeper = more
    /// reliable add/sold detection, slower).
    pub(crate) pages: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SaveWatchParams {
    /// Name to save the search under (letters, digits, . - _ @).
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) search: SearchParams,
}

// ---------------------------------------------------------------------------
// helpers

fn err(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

fn snap_entry(id: i64, s: &crate::ItemSnap) -> serde_json::Value {
    json!({
        "id": id,
        "title": s.title,
        "price": s.price,
        "seller": s.seller,
        "url": s.url,
    })
}

// ---------------------------------------------------------------------------
// the server

#[tool_router]
impl VintedMcp {
    pub(crate) fn new(site: String, currency: Option<String>) -> Self {
        Self {
            site,
            currency,
            tool_router: Self::tool_router(),
        }
    }

    fn client(&self, site: Option<&str>) -> Result<crate::VintedClient, ErrorData> {
        let site = site.unwrap_or(&self.site);
        crate::VintedClient::new(site, self.currency.clone()).map_err(err)
    }

    #[tool(
        description = "Search Vinted listings. Honors: free-text query (matches titles AND descriptions server-side), price range, ordering, pagination, exclude-text, titles-only, seller-quality filters. Returns JSON items with price/seller/url. NOT supported upstream (do not try): brand/category/size/colour id filters — put brand words in the query instead."
    )]
    pub(crate) async fn search_listings(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client(p.site.as_deref())?;
        let args = p.to_args();
        let run = execute_search(&client, &args).await.map_err(err)?;
        let out = search_json_value(&run.enriched);
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&out).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Full details of one Vinted item (id from a listing URL): title, description, price, size, condition, colour, photos, seller id."
    )]
    pub(crate) async fn item_details(
        &self,
        Parameters(p): Parameters<ItemId>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client(None)?;
        let detail = client.item(p.id).await.map_err(err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&detail).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Vinted seller profile (id from a member URL): feedback counts, positive/neutral/negative split, overall reputation, profile link."
    )]
    pub(crate) async fn seller_profile(
        &self,
        Parameters(p): Parameters<SellerId>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client(None)?;
        let user = client.user(p.id).await.map_err(err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&user).map_err(err)?,
        )]))
    }

    #[tool(
        description = "List every saved search/watch on this machine (same files as `vinted save`/`vinted searches`). Names can be passed to watch_diff."
    )]
    pub(crate) async fn list_watches(&self) -> Result<CallToolResult, ErrorData> {
        let watches: Vec<serde_json::Value> = storage::list_searches()
            .into_iter()
            .map(|name| {
                storage::load_search(&name)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .map(|def| {
                        json!({
                            "name": name,
                            "site": def["site"],
                            "currency": def["currency"],
                            "query": def["args"]["query"],
                        })
                    })
                    .unwrap_or_else(|| json!({ "name": name, "unreadable": true }))
            })
            .collect();
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&json!({ "watches": watches })).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Run a saved search/watch and report what changed since the last run: newly listed items ('added') and items that left the results ('sold_removed' — sold or delisted or pushed out of the result window). The first run records a baseline. State is stored in vinted-cli's config dir; CLI 'vinted diff' uses the same files."
    )]
    pub(crate) async fn watch_diff(
        &self,
        Parameters(p): Parameters<WatchName>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = storage::sanitize(&p.name).map_err(err)?;
        let def_text = storage::load_search(&name).map_err(err)?;
        #[derive(Deserialize)]
        struct Saved {
            site: String,
            currency: Option<String>,
            args: SearchArgs,
        }
        let saved: Saved = serde_json::from_str(&def_text).map_err(err)?;
        let mut args = saved.args;
        if let Some(pages) = p.pages {
            args.pages = Some(pages);
            args.page = None;
        }
        let client =
            crate::VintedClient::new(&saved.site, self.currency.clone().or(saved.currency))
                .map_err(err)?;
        let out = run_diff(&name, &client, &args).await.map_err(err)?;
        let body = match out.baseline {
            Some(items) => json!({ "name": name, "baseline_recorded": true, "items": items }),
            None => json!({
                "name": name,
                "added": out
                    .added
                    .iter()
                    .map(|(id, s)| snap_entry(*id, s))
                    .collect::<Vec<_>>(),
                "sold_removed": out
                    .sold_removed
                    .iter()
                    .map(|(id, s)| snap_entry(*id, s))
                    .collect::<Vec<_>>(),
                "unchanged": out.unchanged,
            }),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&body).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Create/overwrite a saved search (watch) so an agent can poll it later with watch_diff. Same files `vinted save` writes. accepts the same filters as search_listings, plus from_url."
    )]
    pub(crate) async fn watch_save(
        &self,
        Parameters(p): Parameters<SaveWatchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = storage::sanitize(&p.name).map_err(err)?;
        let site = p.search.site.clone().unwrap_or_else(|| self.site.clone());
        let def = json!({
            "name": name,
            "site": site,
            "currency": self.currency,
            "created_at": now_secs(),
            "args": p.search.to_args(),
        });
        let existed =
            storage::save_search(&name, &serde_json::to_string_pretty(&def).map_err(err)?)
                .map_err(err)?;
        let note = if existed {
            format!("saved search `{name}` overwritten (previous watch state kept)")
        } else {
            format!(
                "saved search `{name}` created — poll it with watch_diff; the first run records a baseline"
            )
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(note)]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VintedMcp {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("vinted", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "Terminal-grade Vinted marketplace access: search listings (text query + price/order/title/seller filters — brand & category ids are silently ignored upstream, use words in the query), inspect items/sellers, and diff saved watches for new/sold listings. Prices come in the site's currency."
                .to_string(),
        );
        info
    }
}

pub(crate) async fn serve(
    site: &str,
    currency: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = VintedMcp::new(site.to_string(), currency)
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
