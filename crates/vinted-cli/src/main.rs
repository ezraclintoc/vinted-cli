//! `vinted` — search and filter the Vinted marketplace from your terminal.
//!
//! Also the reference consumer of `vinted-core` for a future MCP server or
//! web frontend.

pub(crate) mod storage;

mod mcp_install;
mod mcp_server;

use clap::{Args, Parser, Subcommand, ValueEnum};
use comfy_table::presets::ASCII_MARKDOWN;
use comfy_table::{Cell, Color, Table};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use vinted_core::{
    EnrichedSearch, Item, LocalFilter, SearchFilter, SellerFilter, SortOrder, VintedClient,
    models::Condition, url_import,
};

#[derive(Parser)]
#[command(
    name = "vinted",
    version,
    about = "Search and filter Vinted from your terminal.",
    after_help = "Use enough queries politely: results come from Vinted's unofficial API and may be rate-limited."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Vinted country domain to use, e.g. com, fr, co.uk.
    #[arg(
        long,
        short = 's',
        global = true,
        default_value = "com",
        value_name = "TLD"
    )]
    site: String,

    /// Force a currency code (USD, EUR, GBP, PLN, CZK, HUF).
    #[arg(long, global = true, value_name = "CODE")]
    currency: Option<String>,

    /// Output raw JSON instead of a formatted table.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
// The Search variant legitimately carries many filter fields; boxing them all
// to appease clippy would hurt clarity of the clap definitions.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Search catalogue items, with native filters plus custom seller filters.
    Search {
        #[command(flatten)]
        args: SearchArgs,
    },

    /// Save a search so you can re-run and diff it later (`vinted diff NAME`).
    /// The definition is stored as JSON in your user config directory.
    Save {
        /// Name to save the search under (letters, digits, . - _ @).
        name: String,

        #[command(flatten)]
        args: SearchArgs,
    },

    /// Re-run a saved search and show what changed since the last run:
    /// newly listed items (added) and items that left the results
    /// (sold/removed). The first run just records a baseline.
    ///
    /// Uses the marketplace saved with the search (`--site` on `vinted save`).
    Diff {
        /// Name the search was saved under.
        name: String,

        /// Override the number of pages fetched (deeper = more reliable
        /// add/sold detection, slower).
        #[arg(long, value_name = "N")]
        pages: Option<u32>,

        /// Also write the diff to this CSV file
        /// (columns: status,id,title,price,seller,url).
        #[arg(long, value_name = "FILE")]
        export: Option<PathBuf>,
    },

    /// List saved searches.
    Searches,

    /// Delete a saved search (and any recorded watch state).
    Forget {
        /// Name the search was saved under.
        name: String,
    },

    /// Show full details for one item.
    Item {
        /// Numeric item id (from a Vinted item URL).
        id: i64,
    },

    /// Show a seller profile with their review stats.
    User {
        /// Numeric user id (from a Vinted member URL).
        id: i64,
    },

    /// Run the MCP (Model Context Protocol) server or register it with an
    /// agent harness — see `vinted mcp serve --help` / `vinted mcp install
    /// --help`.
    Mcp {
        #[command(subcommand)]
        mcp_command: McpCommand,
    },
}

/// Subcommands of `vinted mcp`.
#[derive(Subcommand, Debug)]
enum McpCommand {
    /// Serve the Vinted MCP tools over stdio (long-running; launched by the
    /// agent harness, not by hand).
    Serve,
    /// Register this binary's `vinted mcp serve` with one or more agent
    /// harnesses (Claude Code, Codex, Goose, Freebuff, Cursor, Windsurf).
    Install {
        /// Which harness(es) to register with; repeatable.
        #[arg(long, value_enum, value_name = "CLIENT")]
        client: Vec<McpClientArg>,

        /// Register with every supported harness regardless of whether its
        /// config is detected.
        #[arg(long)]
        all: bool,

        /// Print what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Harnesses `vinted mcp install` knows how to write configs for.
#[derive(ValueEnum, Clone, Copy, Debug)]
enum McpClientArg {
    /// Claude Code (user scope, ~/.claude.json)
    Claude,
    /// OpenAI Codex CLI (~/.codex/config.toml)
    Codex,
    /// Goose (~/.config/goose/config.yaml)
    Goose,
    /// Freebuff (.agents/mcp.json in the current project)
    Freebuff,
    /// Cursor (~/.cursor/mcp.json)
    Cursor,
    /// Windsurf (~/.codeium/windsurf/mcp_config.json)
    Windsurf,
}

/// Everything `vinted search` accepts; shared by `vinted save`/`diff` and the
/// MCP tools — saved searches persist (and reload) this exact set of flags.
#[derive(Args, Clone, Debug, Serialize, Deserialize, Default)]
pub(crate) struct SearchArgs {
    /// Free-text search query.
    query: Option<String>,

    /// Minimum price.
    #[arg(long, value_name = "PRICE")]
    price_from: Option<f64>,

    /// Maximum price.
    #[arg(long, value_name = "PRICE")]
    price_to: Option<f64>,

    /// Category/catalog ids, comma separated (copy them from a Vinted search
    /// URL, or use --from-url instead).
    #[arg(long, value_name = "IDS")]
    catalog_ids: Option<String>,

    /// Brand ids, comma separated (copy them from a Vinted search URL, or use
    /// --from-url instead).
    #[arg(long, value_name = "IDS")]
    brand_ids: Option<String>,

    /// Size ids, comma separated (from a Vinted search URL).
    #[arg(long, value_name = "IDS")]
    size_ids: Option<String>,

    /// Color ids, comma separated (from a Vinted search URL).
    #[arg(long, value_name = "IDS")]
    color_ids: Option<String>,

    /// Item condition filter(s). Repeatable.
    #[arg(long, value_enum, value_name = "COND")]
    condition: Vec<ConditionArg>,

    /// Result order.
    #[arg(long, value_enum, value_name = "ORDER")]
    order: Option<OrderArg>,

    /// Page number (1-based).
    #[arg(long, short = 'p', value_name = "N")]
    page: Option<u32>,

    /// Fetch multiple consecutive pages (merged, de-duplicated).
    #[arg(long, value_name = "N", conflicts_with = "page")]
    pages: Option<u32>,

    /// Hide items whose title contains any of these terms (comma
    /// separated; case-insensitive unless --case-sensitive).
    #[arg(long, value_name = "WORDS")]
    exclude_text: Option<String>,

    /// Enforce exact capitalization locally: query words must appear in
    /// titles (and --exclude-text must match) with the exact case given.
    #[arg(long)]
    case_sensitive: bool,

    /// Results per page (max 100).
    #[arg(long, short = 'n', value_name = "N")]
    limit: Option<u32>,

    /// Only items with photos.
    #[arg(long)]
    photo: bool,

    /// Only keep items that have a seller description. Search results don't
    /// carry descriptions, so this fetches each item page afterwards —
    /// one request per item, noticeably slower.
    #[arg(long)]
    with_description: bool,

    /// Ignore matches found only in seller descriptions. Vinted's text search
    /// matches titles AND descriptions server-side (descriptions are included
    /// by default); this drops items whose title alone doesn't contain the
    /// query.
    #[arg(long)]
    #[serde(default)]
    titles_only: bool,

    /// Build the filter from a pasted Vinted search URL instead of hunting
    /// for catalog/brand/size/colour ids: filters tapped in the browser are
    /// reflected in the URL and parsed from there. Explicit flags override
    /// what the URL sets.
    #[arg(long, value_name = "URL")]
    from_url: Option<String>,

    // ---- custom seller filters (not natively available on Vinted) ----
    /// Only keep items whose seller has at least one review.
    #[arg(long)]
    seller_has_reviews: bool,

    /// Only keep items whose seller has at least N feedback reviews.
    #[arg(long, value_name = "N")]
    min_seller_feedback: Option<i64>,

    /// Only keep items whose seller rating (positive-review ratio, 0..=1)
    /// is >= F, e.g. 0.95.
    #[arg(long, value_name = "F")]
    min_seller_rating: Option<f64>,

    /// Only keep items from business/pro sellers.
    #[arg(long, conflicts_with = "amateur_only")]
    business_only: bool,

    /// Only keep items from private (non-business) sellers.
    #[arg(long)]
    amateur_only: bool,

    /// Exclude sellers by login, comma separated.
    #[arg(long, value_name = "LOGINS")]
    exclude_seller: Option<String>,

    /// Exclude sellers by id, comma separated.
    #[arg(long, value_name = "IDS")]
    exclude_seller_id: Option<String>,

    /// Max concurrent seller-profile lookups (1..=16).
    #[arg(long, default_value_t = 8, value_name = "N")]
    concurrency: usize,
}

#[derive(ValueEnum, Clone, Copy, Debug, Serialize, Deserialize)]
enum ConditionArg {
    NewWithTag,
    NewNoTag,
    VeryGood,
    Good,
    Satisfactory,
}

impl From<ConditionArg> for Condition {
    fn from(a: ConditionArg) -> Self {
        match a {
            ConditionArg::NewWithTag => Self::NewWithTag,
            ConditionArg::NewNoTag => Self::NewNoTag,
            ConditionArg::VeryGood => Self::VeryGood,
            ConditionArg::Good => Self::Good,
            ConditionArg::Satisfactory => Self::Satisfactory,
        }
    }
}

#[derive(ValueEnum, Clone, Debug, Serialize, Deserialize)]
pub(crate) enum OrderArg {
    Relevance,
    NewestFirst,
    PriceLowToHigh,
    PriceHighToLow,
}

impl From<OrderArg> for SortOrder {
    fn from(a: OrderArg) -> Self {
        match a {
            OrderArg::Relevance => Self::Relevance,
            OrderArg::NewestFirst => Self::NewestFirst,
            OrderArg::PriceLowToHigh => Self::PriceLowToHigh,
            OrderArg::PriceHighToLow => Self::PriceHighToLow,
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Snapshot of an item as stored in watch state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ItemSnap {
    title: String,
    #[serde(default)]
    price: Option<String>,
    #[serde(default)]
    seller: Option<String>,
    #[serde(default)]
    url: String,
}

/// A previously-seen item that disappeared from the results.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SoldRec {
    item: ItemSnap,
    /// Unix seconds when the item was last seen missing.
    #[allow(dead_code)] // persisted for future got-notified-on tooling
    gone_at: u64,
}

/// Watch state for `vinted diff`: what was seen last run, plus an
/// accumulating ledger of everything that left the results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WatchState {
    active: BTreeMap<i64, ItemSnap>,
    sold: BTreeMap<i64, SoldRec>,
    #[serde(default)]
    last_run: u64,
}

pub(crate) fn snap(item: &Item) -> ItemSnap {
    let price = item
        .total_item_price
        .as_ref()
        .or(item.price.as_ref())
        .map(|p| format!("{} {}", p.amount, p.currency_code));
    ItemSnap {
        title: item.title.clone(),
        price,
        seller: item.user.as_ref().map(|u| u.login.clone()),
        url: item.url.clone(),
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match &cli.command {
        Command::Search { args } => {
            let client = VintedClient::new(&cli.site, cli.currency.clone())?;
            let run = execute_search(&client, args).await?;
            if cli.json {
                print_search_json(&run.enriched);
            } else {
                print_search_table(&run.enriched, run.pages_fetched);
            }
        }

        Command::Save { name, args } => {
            let name = storage::sanitize(name)?;
            let def = json!({
                "name": name,
                "site": cli.site,
                "currency": cli.currency,
                "created_at": now_secs(),
                "args": args,
            });
            let existed = storage::save_search(&name, &serde_json::to_string_pretty(&def)?)?;
            if existed {
                eprintln!("(overwriting previously saved search `{name}`)");
            }
            println!("saved search `{name}` — run it and diff with: vinted diff {name}");
        }

        Command::Searches => {
            let names = storage::list_searches();
            if names.is_empty() {
                println!(
                    "No saved searches yet. Create one with: vinted save NAME <query> [flags]"
                );
                return Ok(());
            }
            for name in names {
                let summary = storage::load_search(&name)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .map(|v| {
                        let site = v["site"].as_str().unwrap_or("?").to_string();
                        let q = v["args"]["query"].as_str().unwrap_or("-").to_string();
                        format!("site={site} query={q:?}")
                    })
                    .unwrap_or_else(|| "(unreadable definition)".to_string());
                println!("{name:<24} {summary}");
            }
        }

        Command::Forget { name } => {
            let name = storage::sanitize(name)?;
            let removed_search = storage::delete_search(&name);
            let removed_state = storage::delete_state(&name);
            if removed_search || removed_state {
                println!("forgot `{name}`");
            } else {
                println!("no saved search named `{name}`");
            }
        }

        Command::Diff {
            name,
            pages,
            export,
        } => {
            let def_text = storage::load_search(name)?;
            #[derive(Deserialize)]
            struct SavedSearch {
                site: String,
                currency: Option<String>,
                args: SearchArgs,
            }
            let def: SavedSearch = serde_json::from_str(&def_text)
                .map_err(|e| format!("saved search `{name}` is unreadable ({e}); re-save it"))?;
            let mut args = def.args;
            if let Some(p) = pages {
                args.pages = Some(*p);
                args.page = None;
            }
            let currency = cli.currency.clone().or(def.currency);
            let client = VintedClient::new(&def.site, currency)?;
            diff(&cli, name, &client, &args, export.clone()).await?;
        }

        Command::Item { id } => {
            let client = VintedClient::new(&cli.site, cli.currency.clone())?;
            let detail = client.item(*id).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                println!("id:      {}", detail.id);
                println!("title:   {}", detail.title);
                if let Some(b) = &detail.brand {
                    println!("brand:   {b}");
                }
                if let Some(s) = &detail.size {
                    println!("size:    {s}");
                }
                if let Some(c) = &detail.condition {
                    println!("status:  {c}");
                }
                if let Some(c) = &detail.color {
                    println!("color:   {c}");
                }
                if let Some(m) = &detail.material {
                    println!("material:{m}");
                }
                if let Some(p) = &detail.price {
                    println!(
                        "price:   {}",
                        format_price(Some(p.as_str()), detail.currency.as_deref())
                    );
                }
                if let Some(d) = &detail.description {
                    println!("desc:    {}", truncate(d, 300));
                }
                if let Some(i) = &detail.image {
                    println!("image:   {i}");
                }
                println!("url:     {}", detail.url);
            }
        }

        Command::Mcp { mcp_command } => match mcp_command {
            McpCommand::Serve => {
                mcp_server::serve(&cli.site, cli.currency.clone())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            McpCommand::Install {
                client,
                all,
                dry_run,
            } => {
                let report =
                    mcp_install::install(client, *all, *dry_run).map_err(|e| e.to_string())?;
                println!("{report}");
            }
        },

        Command::User { id } => {
            let client = VintedClient::new(&cli.site, cli.currency.clone())?;
            let user = client.user(*id).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&user)?);
            } else {
                println!("user:        {} ({})", user.login, user.id);
                if let Some(c) = &user.city {
                    println!("city:        {c}");
                }
                println!(
                    "reviews:     {} total ({} positive / {} neutral / {} negative)",
                    user.feedback_count,
                    user.positive_feedback_count,
                    user.neutral_feedback_count,
                    user.negative_feedback_count
                );
                println!("reputation:  {:.0}%", user.feedback_reputation * 100.0);
                if let Some(p) = &user.path {
                    println!("profile:     https://www.vinted.{}{p}", cli.site);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// search pipeline (shared by `search` and `diff`)

pub(crate) struct Run {
    pub(crate) enriched: EnrichedSearch,
    pub(crate) pages_fetched: u32,
}

/// Turn parsed flags into a server-side [`SearchFilter`].
///
/// `--from-url` provides the base; explicit flags win where they are set.
fn build_filter(args: &SearchArgs) -> Result<SearchFilter, String> {
    let mut filter = match &args.from_url {
        Some(url) => url_import::parse_search_url(url).map_err(|e| format!("--from-url: {e}"))?,
        None => SearchFilter::default(),
    };
    if filter.search_text.is_none() {
        filter.search_text = args.query.clone();
    }
    if filter.catalog_ids.is_none() {
        filter.catalog_ids = args.catalog_ids.clone();
    }
    if filter.brand_ids.is_none() {
        filter.brand_ids = args.brand_ids.clone();
    }
    if filter.size_ids.is_none() {
        filter.size_ids = args.size_ids.clone();
    }
    if filter.color_ids.is_none() {
        filter.color_ids = args.color_ids.clone();
    }
    if filter.conditions.is_empty() {
        filter.conditions = args.condition.iter().map(|&c| Condition::from(c)).collect();
    }
    if filter.price_from.is_none() {
        filter.price_from = args.price_from;
    }
    if filter.price_to.is_none() {
        filter.price_to = args.price_to;
    }
    if filter.order.is_none() {
        filter.order = args.order.clone().map(|o| o.into());
    }
    if filter.page.is_none() {
        filter.page = args.page;
    }
    if filter.per_page.is_none() {
        filter.per_page = args.limit;
    }
    if !filter.with_photo {
        filter.with_photo = args.photo;
    }
    Ok(filter)
}

fn local_filter(args: &SearchArgs) -> LocalFilter {
    // require_words keeps only items whose title contains every query word:
    // baseline behavior for --case-sensitive, and the whole point of
    // --titles-only (which strips description-only matches).
    let require_words = if args.case_sensitive || args.titles_only {
        args.query
            .as_ref()
            .map(|q| q.split_whitespace().map(|w| w.to_string()).collect())
            .unwrap_or_default()
    } else {
        Default::default()
    };
    LocalFilter {
        exclude_text: split_comma(&args.exclude_text),
        require_words,
        case: if args.case_sensitive {
            vinted_core::CaseMode::Exact
        } else {
            vinted_core::CaseMode::IgnoreCase
        },
    }
}

fn seller_filter(args: &SearchArgs) -> SellerFilter {
    SellerFilter {
        has_reviews: args.seller_has_reviews,
        min_feedback: args.min_seller_feedback,
        min_reputation: args.min_seller_rating,
        business_only: if args.business_only { Some(true) } else { None },
        amateur_only: args.amateur_only,
        exclude_logins: split_comma(&args.exclude_seller),
        exclude_ids: args
            .exclude_seller_id
            .as_ref()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().parse::<i64>())
                    .filter_map(|x| x.ok())
                    .collect()
            })
            .unwrap_or_default(),
        concurrency: args.concurrency,
        ..Default::default()
    }
}

fn split_comma(opt: &Option<String>) -> Vec<String> {
    opt.as_ref()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch + filter: server search, local text filters, seller-quality filters
/// and (optionally) the with-description check. Cheap local filters run
/// before the expensive per-item/page ones.
pub(crate) async fn execute_search(
    client: &VintedClient,
    args: &SearchArgs,
) -> Result<Run, Box<dyn std::error::Error>> {
    let filter = build_filter(args)?;
    if filter.search_text.is_none() && filter.catalog_ids.is_none() && filter.brand_ids.is_none() {
        return Err(
            "Provide a QUERY, --from-url, or at least one of --catalog-ids/--brand-ids.".into(),
        );
    }

    // Server-side search fetch (single page or merged multi-page).
    let (response, pages_fetched) = if let Some(pages) = args.pages {
        let merged = client.search_pages(&filter, pages).await?;
        let fetched = merged.pages_fetched;
        (merged.response, fetched)
    } else {
        (client.search(&filter).await?, 1)
    };
    if pages_fetched > 1 {
        eprintln!("→ fetched {pages_fetched} page(s)");
    }

    // Local text filters first (cheap), network seller filters after.
    let local = local_filter(args);
    let (items, locally_dropped) = local.apply(response.items);
    let response = vinted_core::SearchResponse {
        items,
        pagination: response.pagination,
    };

    let seller = seller_filter(args);
    let mut enriched = if seller.is_active() {
        eprintln!("→ applying custom seller filters (this checks seller profiles)…");
        vinted_core::apply_seller_filter(client, response, &seller).await?
    } else {
        EnrichedSearch {
            response,
            sellers: Default::default(),
            dropped: 0,
        }
    };
    enriched.dropped += locally_dropped;

    // Optional: drop items without a description. Descriptions are not in
    // search responses, so this fetches the item page for every survivor.
    if args.with_description && !enriched.response.items.is_empty() {
        let n = enriched.response.items.len();
        eprintln!("→ checking {n} item pages for descriptions (this may take a while)…");
        let conc = args.concurrency.clamp(1, 8);
        let items = std::mem::take(&mut enriched.response.items);
        let kept: Vec<Item> = futures::stream::iter(items)
            .map(|item| async move {
                let res = client.item_has_description(item.id).await;
                (res, item)
            })
            .buffer_unordered(conc)
            .filter_map(|(res, item)| async move {
                match res {
                    // Items without description are dropped; items whose page
                    // could not be checked are kept (fail-open).
                    Ok(true) | Err(_) => Some(item),
                    Ok(false) => None,
                }
            })
            .collect()
            .await;
        let missing_dropped = n - kept.len();
        enriched.dropped += missing_dropped;
        enriched.response.items = kept;
    }

    Ok(Run {
        enriched,
        pages_fetched,
    })
}

// ---------------------------------------------------------------------------
// diff

/// One watch-diff run's delta (shared by `vinted diff` and the MCP
/// `watch_diff` tool; printing and CSV export are the caller's job).
pub(crate) struct DiffOut {
    pub(crate) added: Vec<(i64, ItemSnap)>,
    pub(crate) sold_removed: Vec<(i64, ItemSnap)>,
    pub(crate) unchanged: usize,
    /// Set on the first run: a baseline was recorded with this many items.
    pub(crate) baseline: Option<usize>,
    pub(crate) pages_fetched: u32,
}

/// Execute the saved search `args`, compare against the watch state named
/// `name`, update the state, and return the delta.
pub(crate) async fn run_diff(
    name: &str,
    client: &VintedClient,
    args: &SearchArgs,
) -> Result<DiffOut, Box<dyn std::error::Error>> {
    let run = execute_search(client, args).await?;
    let current: Vec<(i64, ItemSnap)> = run
        .enriched
        .response
        .items
        .iter()
        .map(|item| (item.id, snap(item)))
        .collect();

    let now = now_secs();
    let prev: Option<WatchState> =
        storage::load_state(name).and_then(|text| serde_json::from_str(&text).ok());

    // First run: record a baseline instead of calling all N items "added".
    let Some(mut state) = prev else {
        let new_state = WatchState {
            active: current.iter().cloned().collect(),
            sold: BTreeMap::new(),
            last_run: now,
        };
        storage::save_state(name, &serde_json::to_string_pretty(&new_state)?)?;
        return Ok(DiffOut {
            added: Vec::new(),
            sold_removed: Vec::new(),
            unchanged: 0,
            baseline: Some(current.len()),
            pages_fetched: run.pages_fetched,
        });
    };

    let current_ids: HashSet<i64> = current.iter().map(|(id, _)| *id).collect();

    let mut added: Vec<(i64, ItemSnap)> = Vec::new();
    for (id, item) in &current {
        if !state.active.contains_key(id) {
            if state.sold.contains_key(id) {
                // Item bucked the "sold" prediction and is listed again.
                state.sold.remove(id);
            }
            added.push((*id, item.clone()));
        }
    }

    let mut removed: Vec<(i64, ItemSnap)> = Vec::new();
    for (id, item) in state.active.clone() {
        if !current_ids.contains(&id) {
            state.sold.insert(
                id,
                SoldRec {
                    item: item.clone(),
                    gone_at: now,
                },
            );
            removed.push((id, item));
        }
    }

    // Refresh the active snapshot (prices/titles can change between runs).
    state.active = current.iter().cloned().collect();
    state.last_run = now;
    storage::save_state(name, &serde_json::to_string_pretty(&state)?)?;

    let unchanged = current.len().saturating_sub(added.len());

    Ok(DiffOut {
        added,
        sold_removed: removed,
        unchanged,
        baseline: None,
        pages_fetched: run.pages_fetched,
    })
}

async fn diff(
    cli: &Cli,
    name: &str,
    client: &VintedClient,
    args: &SearchArgs,
    export: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let out = run_diff(name, client, args).await?;

    if let Some(baseline) = out.baseline {
        if cli.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "baseline_recorded": true,
                    "items": baseline,
                }))?
            );
        } else {
            println!(
                "Baseline recorded: {baseline} items seen. Run `vinted diff {name}` again to see added and sold/removed items."
            );
        }
    } else if cli.json {
        let entries = |list: &[(i64, ItemSnap)]| {
            list.iter()
                .map(|(id, s)| {
                    json!({
                        "id": id,
                        "title": s.title,
                        "price": s.price,
                        "seller": s.seller,
                        "url": s.url,
                    })
                })
                .collect::<Vec<_>>()
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "added": entries(&out.added),
                "sold_removed": entries(&out.sold_removed),
                "unchanged": out.unchanged,
            }))?
        );
    } else if out.added.is_empty() && out.sold_removed.is_empty() {
        println!(
            "No changes since the last run ({} items unchanged).",
            out.unchanged
        );
    } else {
        if !out.added.is_empty() {
            println!("+ added ({}):", out.added.len());
            for (id, item) in &out.added {
                println!(
                    "  [{id}] {} — {} — {}",
                    truncate(&item.title, 60),
                    item.price.as_deref().unwrap_or("—"),
                    item.seller.as_deref().unwrap_or("—")
                );
            }
        }
        if !out.sold_removed.is_empty() {
            println!("- sold/removed ({}):", out.sold_removed.len());
            for (id, item) in &out.sold_removed {
                println!(
                    "  [{id}] {} — {} — {}",
                    truncate(&item.title, 60),
                    item.price.as_deref().unwrap_or("—"),
                    item.seller.as_deref().unwrap_or("—")
                );
            }
        }
        println!("{} items unchanged", out.unchanged);
    }

    if let Some(path) = export {
        export_csv(&path, &out.added, &out.sold_removed)?;
        eprintln!("→ wrote diff to {}", path.display());
    }
    if out.pages_fetched > 1 {
        eprintln!("(checked {} page(s))", out.pages_fetched);
    }
    Ok(())
}

fn export_csv(
    path: &PathBuf,
    added: &[(i64, ItemSnap)],
    removed: &[(i64, ItemSnap)],
) -> Result<(), String> {
    let mut out = String::from("status,id,title,price,seller,url\n");
    let mut rows = 0usize;
    let push_row = |out: &mut String, status: &str, id: i64, item: &ItemSnap| {
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            status,
            id,
            csv_cell(&item.title),
            csv_cell(item.price.as_deref().unwrap_or("")),
            csv_cell(item.seller.as_deref().unwrap_or("")),
            csv_cell(&item.url),
        ));
    };
    for (id, item) in added {
        push_row(&mut out, "added", *id, item);
        rows += 1;
    }
    for (id, item) in removed {
        push_row(&mut out, "sold/removed", *id, item);
        rows += 1;
    }
    std::fs::write(path, out).map_err(|e| format!("writing {}: {e}", path.display()))?;
    eprintln!("wrote {rows} diff row(s) to {}", path.display());
    Ok(())
}

fn csv_cell(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

// ---------------------------------------------------------------------------
// output helpers

fn format_price(amount: Option<&str>, currency: Option<&str>) -> String {
    format!("{} {}", amount.unwrap_or("—"), currency.unwrap_or(""))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

fn print_search_table(enriched: &EnrichedSearch, pages_fetched: u32) {
    let resp = &enriched.response;
    if resp.items.is_empty() {
        println!("No items matched your filters.");
        return;
    }

    let show_reviews = !enriched.sellers.is_empty();
    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    if show_reviews {
        table.set_header(vec!["id", "title", "price", "seller", "reviews", "promo"]);
    } else {
        table.set_header(vec!["id", "title", "price", "seller", "promo"]);
    }

    let mut total_pages = 0;
    let mut total_entries = 0;
    if let Some(p) = &resp.pagination {
        total_pages = p.total_pages;
        total_entries = p.total_entries;
    }

    for item in &resp.items {
        let price = match (&item.price, &item.total_item_price) {
            (Some(p), Some(t)) if t.numeric() > p.numeric() => {
                format!("{} {}\n({} w/ fee)", p.amount, p.currency_code, t.amount)
            }
            (Some(_p), Some(t)) => format!("{} {}", t.amount, t.currency_code),
            (Some(p), None) => format!("{} {}", p.amount, p.currency_code),
            (None, Some(t)) => format!("{} {}", t.amount, t.currency_code),
            (None, None) => "—".into(),
        };
        let (login, fb) = match (
            &item.user,
            item.user.as_ref().and_then(|u| enriched.sellers.get(&u.id)),
        ) {
            (Some(u), Some(info)) => (u.login.clone(), info.feedback_count),
            (Some(u), None) => (u.login.clone(), -1),
            (None, _) => ("?".into(), -1),
        };
        let fb_cell = if fb >= 0 {
            fb.to_string()
        } else {
            "—".into()
        };
        let promoted = if item.promoted { "*" } else { "" };

        let mut row = vec![
            Cell::new(item.id),
            Cell::new(truncate(&item.title, 60)),
            Cell::new(price),
            Cell::new(login),
        ];
        if show_reviews {
            row.push(Cell::new(fb_cell));
        }
        row.push(Cell::new(promoted).fg(if item.promoted {
            Color::Blue
        } else {
            Color::Reset
        }));
        table.add_row(row);
    }

    println!("{table}");
    if total_entries > 0 {
        println!(
            "{} items ({}/{} pages of results, {} total on site)",
            resp.items.len(),
            if pages_fetched == 0 { 1 } else { pages_fetched },
            total_pages,
            total_entries
        );
    }
    if enriched.dropped > 0 {
        println!("(dropped {} items by custom filters)", enriched.dropped);
    }
}

/// Machine-readable view of one search run (shared by `vinted search --json`
/// and the MCP `search_listings` tool).
pub(crate) fn search_json_value(enriched: &EnrichedSearch) -> serde_json::Value {
    let items: Vec<serde_json::Value> = enriched
        .response
        .items
        .iter()
        .map(|item| {
            let seller = item
                .user
                .as_ref()
                .and_then(|u| enriched.sellers.get(&u.id))
                .map(|s| serde_json::to_value(s).unwrap_or(json!({})));
            let mut v = serde_json::to_value(item).unwrap_or(json!({}));
            if let Some(seller) = seller {
                v.as_object_mut()
                    .map(|o| o.insert("seller_info".into(), seller));
            }
            v
        })
        .collect();
    let pagination = serde_json::to_value(&enriched.response.pagination).unwrap_or(json!({}));
    json!({
        "items": items,
        "pagination": pagination,
        "filter_dropped": enriched.dropped,
    })
}

fn print_search_json(enriched: &EnrichedSearch) {
    println!(
        "{}",
        serde_json::to_string_pretty(&search_json_value(enriched)).unwrap()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn search_args(argv: &[&str]) -> SearchArgs {
        let mut full = vec!["vinted", "search", "barley used"];
        full.extend_from_slice(argv);
        match Cli::parse_from(full).command {
            Command::Search { args } => args,
            _ => panic!("expected search subcommand"),
        }
    }

    #[test]
    fn titles_only_keeps_only_title_matches() {
        let lf = local_filter(&search_args(&["--titles-only"]));
        assert_eq!(lf.require_words, vec!["barley", "used"]);
        // Case-insensitive by default, same as the server.
        assert_eq!(lf.case, vinted_core::CaseMode::IgnoreCase);
    }

    #[test]
    fn default_includes_description_matches() {
        let lf = local_filter(&search_args(&[]));
        assert!(lf.require_words.is_empty());
    }

    #[test]
    fn saved_search_roundtrip_keeps_titles_only() {
        let args = search_args(&["--titles-only", "--limit", "5"]);
        let saved = serde_json::to_string(&args).unwrap();
        let back: SearchArgs = serde_json::from_str(&saved).unwrap();
        assert!(back.titles_only, "titles_only must survive a save/reload");
        // Simulates an old saved-search file written before the flag existed.
        let legacy = serde_json::from_str::<SearchArgs>(
            &serde_json::to_string(&search_args(&[]))
                .unwrap()
                .replace("},\n\")titles_only\":false", "}")
                .replace("\"titles_only\":false,", ""),
        )
        .expect("old files without titles_only must still load");
        assert!(!legacy.titles_only);
    }
}
