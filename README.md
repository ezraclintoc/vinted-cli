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
| `--from-url URL` | build the whole filter from a pasted Vinted search URL (whatever filters you tapped in the browser are encoded in the URL); explicit flags override what the URL sets |
| `--condition` | `new-with-tag`, `new-no-tag`, `very-good`, `good`, `satisfactory` |
| `--order` | `relevance`, `newest-first`, `price-low-to-high`, `price-high-to-low` |
| `--page N`, `--limit N` (max 100) | pagination |
| `--pages N` | fetch N consecutive pages and merge the results (de-duplicated, polite 400 ms pauses) |

### Local text filters

Vinted's server search is case-insensitive and has no keyword blocklist, so
these are applied client-side on each result page:

| Flag | Meaning |
| --- | --- |
| `--exclude-text WORDS` | hide items whose title contains any term (comma separated) — case-insensitive by default |
| `--case-sensitive` | enforce exact capitalization locally: query words must appear in titles with the exact case you typed, and `--exclude-text` matches exact case |
| `--with-description` | only items that have a seller description. Search results carry no descriptions, so this fetches each item page afterwards — one request per item, noticeably slower |

Titles are what the catalogue API exposes (descriptions are not part of search
results).

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

### Saved searches & diff

Save a search once, then keep watching it. State lives under your config
directory (`$XDG_CONFIG_HOME/vinted-cli` by default):

```console
$ vinted save carhartt carhartt --price-to 60 --exclude-text kids --limit 40
saved search `carhartt` — run it and diff with: vinted diff carhartt
$ vinted searches                 # list saved searches
$ vinted diff carhartt            # first run records a baseline
Baseline recorded: 30 items seen. Run again to see added and sold/removed items.
$ vinted diff carhartt            # after a while: what changed
+ added (2):
  [10479508510] Boys size 5 … carhartt — 3.85 USD — jennyzeltner
- sold/removed (1):
  [10479468392] Baby Carhartt Vest — 8.05 USD — nscott605
28 items unchanged
$ vinted diff carhartt --export /tmp/diff.csv   # also write added+sold rows to CSV
$ vinted diff carhartt --pages 3                # deeper scan (slower, catches more)
$ vinted forget carhartt                        # remove it + its state
```

Notes:

- “sold/removed” means the item stopped appearing in the search results —
  it was sold, delisted, or pushed out of the (possibly filtered) result window
  between runs. Run `diff --pages` with a deeper page count for more reliable
  coverage.
- Items are always compared on the same saved flags, and “removed” items stay
  visible in later runs (they accumulate in the ledger; `forget` clears it).
- `--from-url` can also be saved; `vinted diff` re-uses everything stored with
  the search, same marketplace included.

### Categories & brands

Numeric facet filters (`--catalog-ids`, `--brand-ids`, etc.) are passed to the
API exactly like the web front sends them, and exact names cannot be resolved
anonymously (the brand/catalog typeahead endpoints are bot-blocked). Paste a
search URL instead: `--from-url "https://www.vinted.com/items?brand_ids[]=…"`
provides the same ids directly from the web UI.

**Known upstream limitation (October 2026):** the anonymous catalogue endpoint
we can use honour the text query, price range, order, currency and
pagination — but it *silently ignores* id-based facet filters (brand,
category, size, colour, condition) in every request shape we probed
(comma-joined ids, repeated arrays, slugs, even a POST body). The behaviour is
pinned by a manual (`#[ignore]`-gated) probe test in the repo, so if Vinted
changes the endpoint, the drift shows up. Until then treat the id-facet flags
as best-effort and lean on free-text queries plus the filters that do work
(price range, order, `--exclude-text`, seller quality, `--with-description`):
`vinted search "dell laptop"` self-identifies the brand well enough for
discovery purposes.

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

Vinted has **no official public API**, and this tool uses **no account** of
yours. It reuses the endpoints the web app calls with an **anonymous session**:
no login, no purchase/sale/messaging actions, read-only discovery only.

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

- [ ] MCP server (`vinted-mcp`) exposing `search` / `item` / `user` / `diff` tools, with a one-command install into Claude Code via `claude mcp add vinted -- vinted-mcp`
- [ ] More custom filters: price-vs-brand outlier detection, review-text mining (“last N reviews all positive”)
- [ ] Web front on top of `vinted-core`

## Contributing

Suggestions and bug reports are welcome: if something breaks or you want a
feature, please [open an issue](https://github.com/ezraclintoc/vinted-cli/issues)
or submit a pull request.

## License

MIT — see [LICENSE](LICENSE).
Not affiliated with, endorsed by, or sponsored by Vinted.
