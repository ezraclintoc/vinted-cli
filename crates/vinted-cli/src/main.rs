//! `vinted` — search and filter the Vinted marketplace from your terminal.
//!
//! Also the reference consumer of `vinted-core` for a future MCP server or
//! web frontend.

use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::presets::ASCII_MARKDOWN;
use comfy_table::{Cell, Color, Table};
use serde_json::json;
use vinted_core::{
    models::Condition, SearchFilter, SellerFilter, SortOrder, VintedClient,
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
    #[arg(long, short = 's', global = true, default_value = "com", value_name = "TLD")]
    site: String,

    /// Force a currency code (USD, EUR, GBP, PLN, CZK, HUF).
    #[arg(long, global = true, value_name = "CODE")]
    currency: Option<String>,

    /// Output raw JSON instead of a formatted table.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Search catalogue items, with native filters plus custom seller filters.
    Search {
        /// Free-text search query.
        query: Option<String>,

        /// Minimum price.
        #[arg(long, value_name = "PRICE")]
        price_from: Option<f64>,

        /// Maximum price.
        #[arg(long, value_name = "PRICE")]
        price_to: Option<f64>,

        /// Category/catalog ids, comma separated (from a Vinted search URL).
        #[arg(long, value_name = "IDS")]
        catalog_ids: Option<String>,

        /// Brand ids, comma separated (from a Vinted search URL).
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

        /// Results per page (max 100).
        #[arg(long, short = 'n', value_name = "N")]
        limit: Option<u32>,

        /// Only items with photos.
        #[arg(long)]
        photo: bool,

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
}

#[derive(ValueEnum, Clone, Copy)]
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

#[derive(ValueEnum, Clone)]
enum OrderArg {
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

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let client = VintedClient::new(&cli.site, cli.currency.clone())?;

    match &cli.command {
        Command::Search {
            query,
            price_from,
            price_to,
            catalog_ids,
            brand_ids,
            size_ids,
            color_ids,
            condition,
            order,
            page,
            limit,
            photo,
            seller_has_reviews,
            min_seller_feedback,
            min_seller_rating,
            business_only,
            amateur_only,
            exclude_seller,
            exclude_seller_id,
            concurrency,
        } => {
            if query.is_none() && catalog_ids.is_none() && brand_ids.is_none() {
                eprintln!("Provide a QUERY or at least one of --catalog-ids/--brand-ids.");
                std::process::exit(2);
            }

            let filter = SearchFilter {
                search_text: query.clone(),
                catalog_ids: catalog_ids.clone(),
                brand_ids: brand_ids.clone(),
                size_ids: size_ids.clone(),
                color_ids: color_ids.clone(),
                conditions: condition.iter().map(|&c| Condition::from(c)).collect(),
                price_from: *price_from,
                price_to: *price_to,
                currency: cli.currency.clone(),
                order: order.clone().map(|o| o.into()),
                page: *page,
                per_page: *limit,
                with_photo: *photo,
                ..Default::default()
            };

            let response = client.search(&filter).await?;

            let seller_filter = SellerFilter {
                has_reviews: *seller_has_reviews,
                min_feedback: *min_seller_feedback,
                min_reputation: *min_seller_rating,
                business_only: Some(*business_only).filter(|_| *business_only),
                amateur_only: *amateur_only,
                exclude_logins: exclude_seller
                    .as_ref()
                    .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
                    .unwrap_or_default(),
                exclude_ids: exclude_seller_id
                    .as_ref()
                    .map(|s| s.split(',').map(|x| x.trim().parse::<i64>()).filter_map(|x| x.ok()).collect())
                    .unwrap_or_default(),
                concurrency: *concurrency,
                ..Default::default()
            };

            let enriched = if seller_filter.is_active() {
                eprintln!(
                    "→ applying custom seller filters (this checks seller profiles)…"
                );
                vinted_core::apply_seller_filter(&client, response, &seller_filter).await?
            } else {
                vinted_core::EnrichedSearch {
                    response,
                    sellers: Default::default(),
                    dropped: 0,
                }
            };

            if cli.json {
                print_search_json(&enriched);
            } else {
                print_search_table(&enriched);
            }
        }

        Command::Item { id } => {
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
                    println!("price:   {}", format_price(Some(p.as_str()), detail.currency.as_deref()));
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

        Command::User { id } => {
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

fn print_search_table(enriched: &vinted_core::EnrichedSearch) {
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
    let current_page = resp.pagination.as_ref().map(|p| p.current_page).unwrap_or(1);

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
        let (login, fb) = match (&item.user, item.user.as_ref().and_then(|u| enriched.sellers.get(&u.id))) {
            (Some(u), Some(info)) => (u.login.clone(), info.feedback_count),
            (Some(u), None) => (u.login.clone(), -1),
            (None, _) => ("?".into(), -1),
        };
        let fb_cell = if fb >= 0 { fb.to_string() } else { "—".into() };
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
        row.push(Cell::new(promoted).fg(if item.promoted { Color::Blue } else { Color::Reset }));
        table.add_row(row);
    }

    println!("{table}");
    if total_entries > 0 {
        println!(
            "{} items, page {}/{} ({} total)",
            resp.items.len(),
            current_page,
            total_pages,
            total_entries
        );
    }
    if enriched.dropped > 0 {
        println!("(dropped {} items by custom seller filters)", enriched.dropped);
    }
}

fn print_search_json(enriched: &vinted_core::EnrichedSearch) {
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
                v.as_object_mut().map(|o| o.insert("seller_info".into(), seller));
            }
            v
        })
        .collect();
    let pagination = serde_json::to_value(&enriched.response.pagination).unwrap_or(json!({}));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "items": items,
            "pagination": pagination,
            "filter_dropped": enriched.dropped,
        }))
        .unwrap()
    );
}
