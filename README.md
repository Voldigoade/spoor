# Spoor

Terminal-first GitHub public-footprint inspection and research tool.

Spoor queries publicly accessible GitHub metadata, commit headers, and public keys to assemble sourced, structured reports with complete provenance. It distinguishes strictly between observed facts, mathematical derivations, and algorithmic inferences.

Designed for standalone terminal auditing and direct integration into interactive web portfolios.

---

## Example Terminal Session

```
$ spoor inspect Voldigoade

[init]    Voldigoade in fast
[resolve] target type: user
[query]   GitHub REST: Fetching user profile
[ok]      GitHub REST: Profile resolved for 'Voldigoade'
[query]   GitHub REST: Fetching public repositories (up to 30)
[ok]      GitHub REST: Retrieved 3 repositories
[scan]    [1/3] Inspecting commits in Voldigoade/portfolio
[scan]    [2/3] Inspecting commits in Voldigoade/recherche-messageries
[scan]    [3/3] Inspecting commits in Voldigoade/voldigoade
[query]   GitHub Keys: Fetching public GPG and SSH keys
[done]    completed in 2.90s

TARGET: Voldigoade
Canonical URL: https://github.com/Voldigoade
Mode: fast · Generated: 2026-10-14 18:34:11 UTC

── SUMMARY ─────────────────────────────────────────────────────────────
  Display Name:            Voldigoade
  Target Type:             GitHub User
  Account Created:         2025-11-24 18:13:40 UTC
  Repositories:            3
  Total Stars:             ★ 1
  Total Forks:             ⑂ 0
  Distinct Emails:         4
  Public Keys:             0
  Top Languages:           Python (1), TypeScript (1)

── PUBLIC IDENTITIES (COMMIT HEADERS & PROFILE) ─────────────────────────
  [1] 245930475+Voldigoade@users.noreply.github.com (GitHub Privacy NoReply) [VERIFIED GPG SIGNER]
      Committer: Voldigoade · Commits: 72 · Sources: Public Git Commit Metadata
  [2] noreply@github.com (Domain Email (Commit Metadata)) [VERIFIED GPG SIGNER]
      Committer: GitHub · Commits: 18 · Sources: Public Git Commit Metadata
  [3] github-actions@github.com (Domain Email (Commit Metadata))
      Committer: github-actions · Commits: 6 · Sources: Public Git Commit Metadata

── COVERAGE & STATUS ───────────────────────────────────────────────────
  Coverage: COMPLETE (49 commits, 3/3 repos)
  GitHub Profile           OK
  GitHub Repositories      OK
  Public Keys              OK               0 GPG keys, 0 SSH keys
  GitLeak                  SKIPPED          Skipped in FAST mode to preserve rate-limit quota
```

---

## Target Normalization

Spoor accepts four input formats and normalizes them into typed targets:

1. GitHub username: `torvalds`
2. GitHub profile URL: `https://github.com/torvalds`
3. Repository shorthand: `torvalds/linux`
4. Repository URL: `https://github.com/torvalds/linux`

Input validation enforces strict hostname checking (`github.com` or `www.github.com` only) and rejects non-GitHub URLs, credentials in URLs, and path traversal sequences.

---

## Analysis Modes

* **FAST (Default):** Tailored for interactive CLI and web terminal embedding. Uses cached data aggressively, queries GitHub REST and GraphQL, bounds commit inspection to top repositories. On the HTTP server, optional provider enrichment can be requested with `"enrich": true`.
* **DEEP (`--deep`):** Full footprint analysis. Widens commit sampling and requests GitLeak identity and activity heatmap enrichment. (Disabled by default on public HTTP instances to protect server resources).

---

## Installation & Build

Requires current stable Rust (1.85+ MSRV).

```bash
# Clone repository
git clone https://github.com/Voldigoade/spoor.git
cd spoor

# Build release binary
cargo build --release

# The compiled binary will be located at:
./target/release/spoor
```

---

## CLI Usage

### Inspect a Target

```bash
# Fast user inspection
spoor inspect torvalds

# Deep user inspection
spoor inspect torvalds --deep

# Repository inspection
spoor repo torvalds/linux
```

### Export Directly

```bash
# Export to JSON
spoor inspect torvalds -f json -o report.json

# Export to GitHub-flavored Markdown
spoor inspect torvalds -f md -o report.md

# Export to monospace plain text
spoor inspect torvalds -f txt -o report.txt

# Export to standalone SVG summary card
spoor inspect torvalds -f svg -o card.svg

# Export to clean multi-page PDF
spoor inspect torvalds -f pdf -o report.pdf
```

### Convert Existing Reports

```bash
spoor export saved_report.json --format pdf -o document.pdf
```

### Cache Management

```bash
# Inspect SQLite cache statistics
spoor cache stats

# Clear local cache
spoor cache clear
```

### Interactive Terminal Mode

Running `spoor` without subcommands launches the interactive terminal REPL:

```bash
$ spoor
spoor 0.1.2
public GitHub footprint explorer
by Voldigoade
Type 'help' for commands, 'exit' to quit.

spoor> inspect torvalds
...
spoor> export svg torvalds.svg
[ok] Export saved to torvalds.svg
spoor> quit
```

---

## HTTP Service Mode

Spoor includes an Axum-powered HTTP API with Server-Sent Events (SSE) for streaming progress to web terminal emulators:

```bash
spoor serve --bind 127.0.0.1 --port 8080
```

* Health check: `GET /healthz`
* Start lookup: `POST /v1/lookups`
* Stream progress: `GET /v1/lookups/{id}/events`
* Fetch report: `GET /v1/lookups/{id}/report`
* Download export: `GET /v1/lookups/{id}/export/{format}`

### Operational Architecture & Production Hardening

* **Single-Instance Replica:** Spoor v0.1 server maintains active lookups, SSE broadcasters, and IP rate-limiting in process-local memory. It must run as a single application replica (`replicas: 1`).
* **HTTP DEEP Gating:** Full unconstrained DEEP analysis over HTTP is disabled by default in production (`SPOOR_HTTP_DEEP_ENABLED=false`). Callers submitting `mode=deep` receive a structured `403 Forbidden` response.
* **Dual-Layer Rate Limiting:** Enforces both per-IP limits (default 10 lookups/hour via `SPOOR_RATE_LIMIT_PER_IP`) and global instance limits (default 100 lookups/hour via `SPOOR_GLOBAL_RATE_LIMIT`). Requests breaching limits return `429 Too Many Requests` with a standard `Retry-After` header.
* **GitLeak Upstream Quota Protection:** A process-local sliding window protects GitLeak from upstream quota exhaustion (default 40 requests/hour via `SPOOR_GITLEAK_BUDGET`). Cache hits (24h TTL) do not consume budget. On budget exhaustion, Spoor gracefully proceeds with GitHub metadata.
* **In-Memory Retention (TTL):** Completed and failed lookups are evicted from process memory after `SPOOR_LOOKUP_RETENTION_SECS` (default 3600s). Pending and running lookups are never evicted.
* **Trusted Reverse Proxies:** By default, Spoor exclusively uses direct socket peer IPs for rate limiting and strictly ignores client-supplied `X-Forwarded-For` headers. When deployed behind reverse proxies (e.g. Northflank ingress), configure `--trusted-proxies` or `SPOOR_TRUSTED_PROXIES` (e.g. `10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 127.0.0.1`). Spoor validates the proxy peer and performs a secure right-to-left traversal to extract authentic client IPs.
* **Production CORS:** Set `SPOOR_ALLOWED_ORIGINS` to a comma-separated list of exact production origins (e.g. `https://voldigoade.xyz, https://voldigoade-portfolio.vercel.app`). Wildcards are rejected in production.

For full API specifications, SSE event schemas, and reference client integration, see [`docs/EMBED.md`](docs/EMBED.md).

---

## Upstream Providers

* **GitHub REST API:** User profile, public repositories, commit headers, public events, GPG/SSH keys, languages, community hygiene files.
* **GitHub GraphQL API:** Aggregated repository and profile data when authenticated.
* **GitLeak API:** Public commit email enrichment and activity heatmap analysis (DEEP mode).

### Authentication

Set the `GITHUB_TOKEN` environment variable to authenticate with GitHub (increasing rate limits from 60 to 5,000 requests/hour):

```bash
export GITHUB_TOKEN="ghp_..."
```

Spoor functions anonymously if no token is configured, degrading gracefully to available unauthenticated REST quota.

---

## Provenance & Classification

Findings in Spoor reports carry explicit classifications:

* `OBSERVED`: Returned directly by an authoritative provider.
* `DERIVED`: Computed mathematically from observed data.
* `INFERRED`: Statistical deduction (e.g. commit activity hour windows). Inferences are never converted into facts.
* `PARTIAL`: Bounded by pagination or scan budgets.
* `UNKNOWN`: Unestablished information.

See [`LIMITATIONS.md`](LIMITATIONS.md) for full documentation of operational boundaries.

---

## Development & Testing

```bash
# Run complete workspace test suite
cargo test --workspace --all-features

# Run Clippy linter with denied warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Check code formatting
cargo fmt --check
```

---

## Public Attribution

Spoor by Voldigoade
