# vinted-cli

Search and filter [Vinted](https://www.vinted.com) — the second-hand fashion
marketplace — from your terminal, or let your AI agent do it via MCP. Built in
Rust on top of the unofficial marketplace API the web app itself uses.

**Read-only by design:** this tool only *reads* Vinted. It cannot list items,
reply to messages, favorite, buy, sell, or perform any other action on Vinted.
There is nowhere to log in, because no login is ever performed.

```console
$ vinted search "levi 550" --price-to 40 --order price-low-to-high --limit 6
$ vinted search sneakers --min-seller-feedback 25 --min-seller-rating 0.95   # a filter Vinted doesn't have
$ vinted item 10464769572
```

## Why it exists

Everything here is about **automating the search**: watching for new listings
with saved searches, filtering on things Vinted's UI can't filter on (seller
reputation, keyword blocklists, title-only matching), and letting agents poll
the same watches through MCP. Read details and act manually at your own pace —
or ask your agent to do the polling and report on changes.

## Quick start

Install from [crates.io](https://crates.io/crates/vinted-cli) (Rust 1.85+):

```console
$ cargo install vinted-cli
```

or install a prebuilt binary:

```console
# Linux (x86_64, static musl build)
$ curl -L https://github.com/ezraclintoc/vinted-cli/releases/latest/download/vinted-x86_64-unknown-linux-musl \
    -o vinted && chmod +x vinted

# macOS (Apple silicon)
$ curl -L https://github.com/ezraclintoc/vinted-cli/releases/latest/download/vinted-aarch64-apple-darwin \
    -o vinted && chmod +x vinted
```

On Windows, grab `vinted-x86_64-pc-windows-msvc.exe` from
[Releases](https://github.com/ezraclintoc/vinted-cli/releases) and rename it to
`vinted.exe`. (The prebuilt binaries are self-contained; no Rust toolchain needed.)

Then, two minutes to your first result:

```console
$ vinted search "levi 550" --price-to 40 --limit 6     # search your marketplace
$ vinted item 10464769572                              # one item's detail page price summary
$ vinted save levis "levi 550" --price-to 40           # save it as a watch (see below)
```

## AI agent (MCP)

`vinted` is also a **[Model Context Protocol](https://modelcontextprotocol.io)
server**: install it into your AI harness once and the agent gets six tools —
`search_listings`, `item_details`, `seller_profile`, `list_watches`,
`watch_diff`, `watch_save` — over the same search pipeline as the CLI,
sharing the same saved-search files. Save a watch once, then let your agent
do the staying-on-top-of-it for you.

Real example — buying hard drives. Sellers almost never say whether a drive
is CMR or SMR, but the model number (in the title, description or photos)
tells. Rather than squinting at each listing yourself, let the agent do the
scanning:

```console
# a typical listing, straight out of `vinted search "4tb hard drive"`:
#   “WD 4TB internal hard drive, perfect condition” — CMR? SMR? it doesn't say.
# hand the scanning to the agent instead:
> “search Vinted for 4 TB hard drives under 40 and classify every drive as
   CMR or SMR from its model number — cheapest CMRs first”
→ CMR: ST4000DM000 — 35 USD, WD4001FAEX — 40 USD …
→ SMR: ST4000DM004 — 30 USD (skip)
```

The agent can also poll a saved watch every evening and only surface actual
changes — no more manually refreshing the search. Buying still happens
manually in the browser: every MCP tool is read-only, same as the CLI.

### Supported harnesses

One command per harness — the installer writes the config file for you
(upserting one `vinted` entry, preserving everything else):

| Harness | Easy install | Where the entry lands (also fine to enter manually) |
| --- | --- | --- |
| Claude Code | `vinted mcp install --client claude` | `~/.claude.json` → `mcpServers.vinted` |
| Codex | `vinted mcp install --client codex` | `~/.codex/config.toml` → `[mcp_servers.vinted]` |
| Goose | `vinted mcp install --client goose` | `config.yaml` → `extensions.vinted` |
| [Freebuff](https://freebuff.com/?ref=ref-541424d3-0a42-4b24-b893-c71e84be7395) | `vinted mcp install --client freebuff` | `.agents/mcp.json` → `mcpServers.vinted` |
| Cursor | `vinted mcp install --client cursor` | `mcp.json` → `mcpServers.vinted` |
| Windsurf | `vinted mcp install --client windsurf` | `mcp.json` → `mcpServers.vinted` |

`vinted mcp install` with no flags registers every harness it detects,
`--all` forces all six, and `--dry-run` previews without writing; re-runs
are idempotent ("already installed, no changes"). Note there is **no IP or
port to enter anywhere**: harness support is stdio-based — the harness
launches `vinted mcp serve` itself locally. A manual entry is always
possible: command = `vinted`, args = `["mcp", "serve"]`.

*The Freebuff link above is a referral link: a small credit goes to the
author if you sign up, at no cost to you.*

**Your harness isn't in the table?**
[Open an issue](https://github.com/ezraclintoc/vinted-cli/issues) — support
is one config-file writer away for anything that speaks MCP over stdio.

## How to use

All commands take the global options `--site <tld>` (marketplace: `com`
default, `fr`, `de`, `es`, `co.uk`, …), `--currency CODE`, and `--json`
(machine-readable output, one stable shape per command).

### Search

```console
$ vinted search sneakers --price-from 20 --price-to 60 \
    --order newest-first --limit 30 --pages 2
```

| Flag | Meaning |
| --- | --- |
| `QUERY` (positional) | free-text search — matches titles **and** seller descriptions server-side |
| `--price-from F`, `--price-to F` | price range (site currency) |
| `--order` | `relevance`, `newest-first`, `price-low-to-high`, `price-high-to-low` |
| `--page N`, `--limit N` (max 100) | pagination |
| `--pages N` | fetch N consecutive pages, merged (polite pauses between) |
| `--from-url URL` | build the whole filter from a pasted Vinted search URL; explicit flags override what the URL sets |

**Local text filters** (applied client-side, since the server search has no
keyword blocklist): `--exclude-text WORDS` (blocklist), `--case-sensitive`
(exact capitalization required), `--titles-only` (drop matches found only in
descriptions), `--with-description` (only items that carry a description;
fetches one page per item, slower).

**Seller-quality filters** *(not available on Vinted's site)*:
`--seller-has-reviews`, `--min-seller-feedback N`, `--min-seller-rating 0.95`,
`--business-only` / `--amateur-only`, `--exclude-seller LOGINS`.

### Saved searches & diff

Save a search once, then keep watching it. State lives under your config
directory (`$XDG_CONFIG_HOME/vinted-cli` by default):

```console
$ vinted save carhartt carhartt --price-to 60 --exclude-text kids --limit 40
saved search `carhartt` — run it and diff with: vinted diff carhartt
$ vinted diff carhartt            # first run records a baseline
$ vinted diff carhartt            # after a while: what changed
+ added (2):
  [10479508510] Boys size 5 … carhartt — 3.85 USD — jennyzeltner
- sold/removed (1):
  [10479468392] Baby Carhartt Vest — 8.05 USD — nscott605
28 items unchanged
$ vinted searches                 # list saved searches
$ vinted forget carhartt          # remove it + its state
```

“Sold/removed” means the item stopped appearing in the results — sold,
delisted, or pushed out of the result window between runs. Use
`diff --pages N` for deeper coverage and `--export FILE.csv` to get rows out.

### AI agents (MCP)

The CLI doubles as an MCP server your AI agent can drive these same
searches through — see [AI agent (MCP)](#ai-agent-mcp).

### Other commands

- `vinted item <ID>` — full item data (title, description, price, size,
  condition, colour, material, photo)
- `vinted user <ID>` — review counts and reputation for a seller

## Caveats

**Id-facet filters are dead upstream (October 2026).** The anonymous
catalogue endpoint honors the text query, price range, order, currency and
pagination, but *silently ignores* id-based facet filters (`--brand-ids`,
`--catalog-ids`, sizes, colours, conditions) in every request shape we probed.
The behaviour is pinned by an on-demand probe test in the repo. Until it
changes, put brand/category words in the text query
(`vinted search "adidas hoodie"`) and use the filters that do work.
`--from-url` still encodes those ids for you — they're inert today.

**Be polite.** Vinted has no official public API, this tool uses no account,
and every request goes through the same anonymous session the web app gets
(`access_token_web` cookie). It makes a handful of requests per search, does
not rotate proxies, and retries nothing beyond session refresh. Heavy
automated hammering may get your IP rate-challenged or permanently blocked —
respect Vinted's Terms of Service.

## Legal notice

This is an **open-source community project, not affiliated with,
endorsed by, or sponsored by Vinted** in any way. "Vinted" is a trademark of
its owner and both names belong to them. Source here is provided as-is, without
warranty of any kind, and use of this tool is subject to Vinted's Terms of
Service for accessing their service. If Vinted asks this project to stop, the
project will comply. Keep your usage polite and read-only, and it should never
come to that.

## Roadmap

- [x] Saved searches with added/sold diff, CSV export, ledger (`v0.2.0`)
- [x] MCP server + one-command install into agent harnesses (`v0.2.0`)
- [ ] Watch notifications: webhook/e-mail report when a saved search changes
- [ ] More seller-intelligence filters: price-vs-brand outliers, review-text mining (“last 20 reviews mention scams”)
- [ ] Web frontend on top of `vinted-core`
- [ ] Homebrew formula / winget package

## Contributing

Suggestions and bug reports are welcome: open an
[issue](https://github.com/ezraclintoc/vinted-cli/issues) or submit a PR.

## License

MIT — see [LICENSE](LICENSE).
