# Spoor Portfolio Integration Contract

This document specifies the exact contract for integrating Spoor into web portfolios, terminal emulators, and external applications.

Attribution: **Spoor by Voldigoade**  
Repository: `https://github.com/Voldigoade/spoor`

---

## 1. Overview & Architecture

Spoor provides a versioned, asynchronous HTTP and Server-Sent Events (SSE) API designed to back interactive terminal frontends (such as xterm.js or custom web shells).

```
┌─────────────────────────────────┐
│ Web Portfolio / Terminal UI    │
└──────────────┬──────────────────┘
               │ 1. POST /v1/lookups (target, mode)
               ▼
┌─────────────────────────────────┐
│ Spoor Server (/v1)              │ ── 2. Returns { id, status: "pending" }
└──────────────┬──────────────────┘
               │ 3. GET /v1/lookups/{id}/events (SSE stream)
               │    Real progress: lookup.started, provider.started, scan.progress...
               ▼
┌─────────────────────────────────┐
│ Analysis Completed              │ ── 4. GET /v1/lookups/{id}/report (JSON)
│                                 │ ── 5. GET /v1/lookups/{id}/export/{format}
└─────────────────────────────────┘
```

---

## 2. API Endpoints

All application routes are prefixed with `/v1` (with the exception of `/healthz`).

| Method | Path | Purpose | Success Status |
| :--- | :--- | :--- | :--- |
| `GET` | `/healthz` | Service health & version | `200 OK` |
| `POST` | `/v1/lookups` | Initiate asynchronous lookup | `202 Accepted` |
| `GET` | `/v1/lookups/{id}` | Poll lookup execution status | `200 OK` |
| `GET` | `/v1/lookups/{id}/events` | Real-time SSE progress stream | `200 OK` (text/event-stream) |
| `GET` | `/v1/lookups/{id}/report` | Retrieve canonical JSON report | `200 OK` |
| `GET` | `/v1/lookups/{id}/export/{format}` | Download exported report | `200 OK` |

Supported export formats: `json`, `md`, `txt`, `svg`, `pdf`.

---

## 3. Request & Response Schemas

### 3.1 Initiate Lookup (`POST /v1/lookups`)

**Request Headers:**
```http
Content-Type: application/json
```

**Request Body:**
```json
{
  "target": "Voldigoade",
  "mode": "fast"
}
```

* `target` (string, required): GitHub username (`torvalds`), profile URL (`https://github.com/torvalds`), repository shorthand (`torvalds/linux`), or repository URL (`https://github.com/torvalds/linux`).
* `mode` (string, optional, default `"fast"`): `"fast"` (optimized for interactive latency, bounded commit sample, cache-prioritized) or `"deep"` (broader commit analysis, GitLeak enrichment).

**Response Body (`202 Accepted`):**
```json
{
  "id": "e9b27b3b-871f-44e2-9f6e-693d258bdf05",
  "target": "Voldigoade",
  "mode": "fast",
  "status": "pending",
  "created_at": "2026-10-14T18:34:11.234567Z"
}
```

---

### 3.2 Server-Sent Events Stream (`GET /v1/lookups/{id}/events`)

Connect using standard browser `EventSource`:
```javascript
const es = new EventSource('/v1/lookups/e9b27b3b-871f-44e2-9f6e-693d258bdf05/events');
```

**Event Types Dispatched:**

1. `lookup.started`
   ```json
   {"target": "Voldigoade", "mode": "fast"}
   ```
2. `target.resolved`
   ```json
   {"target": "Voldigoade", "target_type": "user"}
   ```
3. `provider.started`
   ```json
   {"provider": "GitHub REST", "operation": "Fetching user profile"}
   ```
4. `provider.completed`
   ```json
   {"provider": "GitHub REST", "details": "Profile resolved for 'Voldigoade'"}
   ```
5. `provider.warning`
   ```json
   {"provider": "GitLeak", "message": "Rate limit exceeded (50/hr)"}
   ```
6. `scan.progress`
   ```json
   {"current": 2, "total": 3, "message": "Inspecting commits in Voldigoade/portfolio"}
   ```
7. `analysis.completed`
   ```json
   {"duration_ms": 1420}
   ```
8. `lookup.failed`
   ```json
   {"error": "GitHub user not found: non_existent_user"}
   ```

---

### 3.3 Retrieve Report (`GET /v1/lookups/{id}/report`)

When status is `completed`, returns the complete canonical `Report` JSON:

```json
{
  "schema_version": "1.0",
  "generator": {
    "name": "Spoor",
    "version": "0.1.0",
    "author": "Voldigoade",
    "repository": "https://github.com/Voldigoade/spoor"
  },
  "target": {
    "type": "user",
    "username": "Voldigoade"
  },
  "mode": "fast",
  "generated_at": "2026-10-14T18:34:11.234567Z",
  "summary": {
    "display_name": "Voldigoade",
    "target_type": "GitHub User",
    "account_created": "2025-11-24T18:13:40Z",
    "total_public_repositories": 3,
    "total_stars_accumulated": 1,
    "total_forks_accumulated": 0,
    "primary_languages": [["TypeScript", 1], ["Python", 1]],
    "distinct_emails_observed": 4,
    "public_keys_found": 0,
    "fork_ratio": 0.0,
    "archived_ratio": 0.0
  },
  "profile": { ... },
  "repositories": [ ... ],
  "activity": { ... },
  "identities": [
    {
      "name": "Voldigoade",
      "email": "245930475+Voldigoade@users.noreply.github.com",
      "email_type": "github_no_reply",
      "first_seen": "2026-10-13T12:00:00Z",
      "last_seen": "2026-10-14T18:00:00Z",
      "commits_observed": 72,
      "repositories": ["Voldigoade/portfolio"],
      "sources": ["Public Git Commit Metadata"],
      "is_verified_signer": true
    }
  ],
  "keys": [],
  "inferences": [],
  "provider_status": {
    "GitHub Profile": { "status": "ok" },
    "GitLeak": { "status": "skipped", "message": "Skipped in FAST mode to preserve rate-limit quota" }
  },
  "coverage": {
    "commits_scanned": 49,
    "repositories_scanned": 3,
    "repositories_total": 3,
    "is_partial": false
  },
  "provenance": [ ... ],
  "warnings": [],
  "limitations": [ ... ]
}
```

---

## 4. Error Codes & Handling

Error responses use standard RFC-compatible JSON bodies:

```json
{
  "error": "Only github.com targets are permitted, received host: 'evil.com'",
  "code": "invalid_target"
}
```

| HTTP Status | Error Code | Description |
| :--- | :--- | :--- |
| `400 Bad Request` | `invalid_target` | Malformed target, unsupported scheme, or SSRF rejection |
| `404 Not Found` | `not_found` | Lookup ID does not exist |
| `409 Conflict` | `report_not_ready` | Report requested before analysis finished |
| `422 Unprocessable` | `lookup_failed` | Upstream target not found (e.g. 404 user on GitHub) |
| `429 Too Many Requests` | `rate_limit_exceeded` | Client exceeded 30 lookups/minute limit |
| `500 Internal Error` | `internal_error` | Server processing error |

---

## 5. Security & Abuse Controls

1. **SSRF Protection:** Target parsing rejects any non-GitHub domain or IP address. Requests to internal addresses (`127.0.0.1`, `169.254.169.254`, `localhost`) are refused at the parser level before any socket connection occurs.
2. **Ephemeral IP Rate Limiting:** Client IP addresses are salted and hashed into memory with short TTLs. No raw IP addresses are logged or stored.
3. **Execution Safety:** Spoor never runs code from inspected repositories (`npm`, `pip`, `cargo`, shell scripts).
4. **Token Isolation:** Server-side `GITHUB_TOKEN` is never echoed in API responses, logs, or exports.

---

## 6. TypeScript Reference Client

Use the zero-dependency client in `packages/spoor-client`:

```typescript
import { SpoorClient } from '@spoor/client';

const spoor = new SpoorClient({ baseUrl: 'https://api.spoor.example.com' });

// 1. Check service health
const health = await spoor.health();

// 2. Start lookup
const lookup = await spoor.startLookup('Voldigoade', 'fast');

// 3. Stream real progress to your terminal UI
const unsubscribe = spoor.streamEvents(
  lookup.id,
  (ev) => {
    switch (ev.event) {
      case 'lookup.started':
        terminal.writeln(`\x1b[90m[init]\x1b[0m ${ev.data.target}`);
        break;
      case 'provider.completed':
        terminal.writeln(`\x1b[32m[ok]\x1b[0m ${ev.data.provider}: ${ev.data.details}`);
        break;
      case 'analysis.completed':
        terminal.writeln(`\x1b[32m[done]\x1b[0m Finished in ${(ev.data.duration_ms / 1000).toFixed(2)}s`);
        loadReport(lookup.id);
        break;
    }
  },
  (err) => console.error('SSE connection error:', err)
);

// 4. Load completed report
async function loadReport(id: string) {
  unsubscribe();
  const report = await spoor.getReport(id);
  terminal.writeln(`Discovered ${report.identities.length} public identities.`);
}
```

---

## 7. Operational Topology & Production Hardening

### 7.1 Single-Instance State Constraint

Spoor v0.1 server maintains active lookup states, SSE broadcast channels, and ephemeral IP rate limits in process-local memory.

> [!IMPORTANT]
> **Operational Requirement:** Spoor v0.1 server must run as a single application replica (`replicas: 1`) unless a shared lookup/job state implementation is introduced. Deploying multiple server replicas behind a round-robin load balancer without sticky routing will cause disconnected SSE event streams and fragmented rate-limiting state.

### 7.2 Reverse Proxy & Client IP Verification

Direct clients connecting over public networks can trivially inject or manipulate `X-Forwarded-For` headers. To prevent IP rate-limiting bypass:

* **Default Mode (Untrusted):** When `SPOOR_TRUSTED_PROXIES` is unset, Spoor uses the direct socket peer IP exclusively. All incoming `X-Forwarded-For` headers are strictly ignored.
* **Trusted Reverse Proxy Mode:** When Spoor is deployed behind reverse proxies or CDNs (such as Nginx, Caddy, or Cloudflare), configure trusted proxy IPs or CIDR subnets via `SPOOR_TRUSTED_PROXIES` (e.g. `127.0.0.1, 10.0.0.0/8, 172.16.0.0/12`):
  * Spoor first verifies that the direct socket connection originates from a configured trusted proxy.
  * If trusted, Spoor inspects `X-Forwarded-For` from right to left (nearest proxy backwards), skipping intermediary trusted proxies and selecting the first untrusted upstream IP.
  * Any client-injected spoofed headers situated upstream of the trusted proxy are ignored.

### 7.3 Production CORS Configuration

Cross-Origin Resource Sharing (CORS) is strictly restricted in production:

* Set `SPOOR_ALLOWED_ORIGINS` to a comma-separated list of exact production origins (e.g. `https://voldigoade-portfolio.vercel.app`).
* Wildcard origins (`*`) are disallowed in production deployments.
* If `SPOOR_ALLOWED_ORIGINS` is unset, the server defaults to permitting only local development origins (`http://localhost`, `http://127.0.0.1`).

