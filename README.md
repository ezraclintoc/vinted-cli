# vinted-cli

Search and filter [Vinted](https://www.vinted.com) — the second-hand fashion
marketplace — from your terminal. Built in Rust on top of the unofficial
marketplace API the web app itself uses.

```console
$ vinted search "levi 550" --price-to 40 --order price-low-to-high --limit 6
$ vinted search sneakers --min-seller-feedback 25 --min-seller-rating 0.95   # custom filter Vinted doesn't have
$ vinted item 10464769572
$ vinted user 3182616244
```

## Why

The eventual goal is an **MCP server** and a **better web front** on top of a
solid, reusable core:

```
crates/vinted-core   async client library (no UI opinion)
crates/vinted-cli    terminal UI on top of the core
```

`--json` output on every command keeps the core script/agent-friendly out of
the box.

## Features

### Native Vinted filters (`vinted search`)

| Flag | Meaning |
| --- | --- |
| `QUERY` (positional) | free-text search |
| `--price-from F`, `--price-to F` | price range (site currency) |
| `--catalog-ids`, `--brand-ids`, `--size-ids`, `--color-ids` | numeric ids (grab them from a vinted.com search URL) — repeatable, comma separated |
| `--condition` | `new-with-tag`, `new-no-tag`, `very-good`, `good`, `satisfactory` |
| `--order` | `relevance`, `newest-first`, `price-low-to-high`, `price-high-to-low` |
| `--page N`, `--limit N` (max 100) | pagination |
| `--photo` | only items with photos |

### Custom seller-quality filters *(not available on Vinted)*

| Flag | Meaning |
| --- | --- |
| `--seller-has-reviews` | only items from sellers with ≥ 1 review |
| `--min-seller-feedback N` | only sellers with at least N feedback |
| `--min-seller-rating F` | only sellers with reputation ≥ F, 0..=1 (e.g. `0.95`) |
| `--business-only` / `--amateur-only` | professional shops / private sellers |
| `--exclude-seller LOGINS`, `--exclude-seller-id IDS` | block-list sellers |

Seller profiles are fetched with bounded concurrency and cached across items in
the same search.

### Other commands

- `vinted item <ID>` — full item data (title, description, price, size,
  condition, colour, material, photo) parsed from the public item page
- `vinted user <ID>` — review counts and reputation for a seller

### Global options

- `--site <tld>` — pick your marketplace: `com` (default), `fr`, `de`, `it`,
  `es`, `co.uk`, `at`, `be`, `cz`, `dk`, `ee`, `fi`, `gr`, `hr`, `hu`, `ie`,
  `lt`, `lu`, `nl`, `pl`, `pt`, `ro`, `se`, `si`, `sk`
- `--currency CODE` — force currency (USD, EUR, GBP, PLN, CZK, HUF, …)
- `--json` — machine-readable output for scripts / MCP tooling

## Installation

- From source: `cargo install --path crates/vinted-cli` (Rust 1.85+)
- Or `cargo install vinted-cli` once published on crates.io
- Prebuilt binaries from [GitHub Releases](https://github.com/ezraclintoc/vinted-cli/releases)

## How it works (and its limits)

Vinted has **no official public API**. This tool reuses the endpoints the web
app calls with an anonymous session:

1. One landing-page request issues an `access_token_web` cookie + `X-Anon-Id`.
2. Catalogue search goes through `api.<site>/svc-catalogue/items`.
3. Seller profiles (for the review filters) live on `www.<site>/api/v2/users/:id`.
4. Item details come from the public item page (the JSON detail endpoint is
   bot-blocked); item pages are themselves sometimes geo-blocked, so we fall
   back through a couple of country domains.

Consequently:

- Infrequent breaks are expected when Vinted changes things.
- Be polite: this CLI does a handful of requests per search, no retries on
  rate limits beyond session refresh, and no proxy rotation. Heavy automated
  use may get your IP challenged; respect Vinted's terms of service.
- No sign-in, so no buying/selling/messaging — read-only discovery only.

## Roadmap

- [ ] MCP server (`vinted-cli mcp`) exposing `search` / `item` / `user` tools
- [ ] Saved searches & watch (diff new items since last run)
- [ ] More custom filters: exclude items without description, keyword blocklists, price-vs-brand outlier detection
- [ ] Web front on top of `vinted-core`

## License

MIT — see [LICENSE](LICENSE).
Not affiliated with, endorsed by, or sponsored by Vinted.
