# Spoor

Inspect public GitHub profiles, repositories, commit identities and keys from the terminal or an HTTP client.

```sh
spoor inspect Voldigoade
spoor repo Voldigoade/spoor
spoor inspect Voldigoade -f json -o report.json
```

Reports keep source references and distinguish observed data, derived values and inferences. An address found in a commit is historical evidence, not necessarily a current contact address.

## Build

Rust 1.85 or later is required.

```sh
git clone https://github.com/Voldigoade/spoor.git
cd spoor
cargo build --release
./target/release/spoor --help
```

## Usage

Targets can be usernames, `owner/repo`, GitHub profile URLs or repository URLs. Other hosts, URL credentials and traversal paths are rejected.

```sh
spoor inspect torvalds
spoor inspect torvalds --deep
spoor repo torvalds/linux
spoor export report.json --format pdf -o report.pdf
spoor cache stats
spoor cache clear
```

Run `spoor` without arguments for the interactive prompt. Version 0.1.3 supports `inspect`, `repo`, `export`, `help` and `quit` there.

## Modes

FAST is the default. For user targets it samples up to 30 commits from each of 3 repositories, preferring non-forks. Enriched FAST keeps those bounds and requests GitLeak data through the HTTP option `enrich: true`. DEEP samples up to 50 commits from each of 8 repositories and requests enrichment. Repository targets do not query GitLeak.

GitHub supplies profile data, repositories, commit headers, public activity and keys. GraphQL is used when a token is available. Optional GitLeak data is cached and subject to a separate upstream budget; unavailable enrichment does not discard the GitHub report.

## Exports

Use `-f json|md|txt|svg|pdf` with `-o <path>` on a lookup, or convert a saved JSON report with `spoor export`. JSON retains the canonical report; the other formats provide readable reports or a summary card.

## HTTP service

```sh
spoor serve --bind 127.0.0.1 --port 8080
```

Create a lookup with `POST /v1/lookups`, poll its status, stream progress through SSE, then fetch its report or export. Public HTTP DEEP is disabled by default. [The embed guide](docs/EMBED.md) covers requests, events, errors and browser integration.

## Configuration

Set `GITHUB_TOKEN` on the server for authenticated GitHub quota. Without it Spoor uses unauthenticated REST requests. `SPOOR_CACHE_PATH` selects the SQLite cache file.

For a public HTTP service, configure exact `SPOOR_ALLOWED_ORIGINS` and only the proxy subnets you trust in `SPOOR_TRUSTED_PROXIES`. Run one replica: lookup state, event channels and rate limits are process-local. Defaults are 10 lookups per IP/hour, 100 per instance/hour, 4 concurrent analyses and 40 uncached GitLeak requests/hour. Completed and failed lookup records expire after one hour, measured from creation.

See [.env.example](.env.example) and [operations](docs/OPERATIONS.md) for every setting and its default.

## Privacy and limitations

Spoor reads public metadata and never executes code from inspected repositories. Provider credentials stay on the server. The HTTP limiter keeps salted IP hashes in memory; hosting and proxy logs have their own policies. SQLite caches reports and provider responses, which can include observed addresses.

Results depend on scan bounds, upstream retention, cache age and provider availability. Public identities do not prove ownership or current contact details. Activity and timezone inferences do not establish location. See [limitations](LIMITATIONS.md) and [security reporting](SECURITY.md).

## Development

```sh
node scripts/check-prose.mjs
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --release
cargo audit
docker build -t spoor .
```
