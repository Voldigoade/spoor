# HTTP operations

These defaults match v0.1.3. Keep one application replica: active jobs, SSE channels, rate limits and the GitLeak budget live in process memory. A restart loses active lookup IDs. SQLite cache persistence is separate from lookup retention.

| Variable | Default | Purpose |
| --- | --- | --- |
| `GITHUB_TOKEN` | unset | Optional server-side GitHub authentication |
| `SPOOR_CACHE_PATH` | OS cache directory, `spoor/cache.sqlite` | SQLite cache file |
| `SPOOR_ALLOWED_ORIGINS` | local development origins | Comma-separated exact browser origins |
| `SPOOR_TRUSTED_PROXIES` | none | Trusted proxy IPs or CIDRs |
| `SPOOR_HTTP_DEEP_ENABLED` | `false` | Allow HTTP DEEP; otherwise returns 403 |
| `SPOOR_RATE_LIMIT_PER_IP` | `10` | Lookup creations per client window |
| `SPOOR_RATE_LIMIT_WINDOW_SECS` | `3600` | Client window length |
| `SPOOR_GLOBAL_RATE_LIMIT` | `100` | Lookup creations per instance window |
| `SPOOR_GLOBAL_RATE_LIMIT_WINDOW_SECS` | `3600` | Instance window length |
| `SPOOR_MAX_CONCURRENT_LOOKUPS` | `4` | Concurrent background analyses |
| `SPOOR_LOOKUP_RETENTION_SECS` | `3600` | Retention from creation for completed/failed records |
| `SPOOR_GITLEAK_BUDGET` | `40` | Uncached outbound GitLeak requests per window |
| `SPOOR_GITLEAK_BUDGET_WINDOW_SECS` | `3600` | GitLeak budget window length |
| `RUST_LOG` | unset | Enable tracing with the selected filter |

Only lookup creation consumes the per-IP and global limits. A 429 response includes Retry-After. Status, SSE, reports and exports do not consume lookup quota. Queued analyses wait on the concurrency semaphore. Pending/running records are not pruned. Cleanup runs every minute and before creating a lookup; its age calculation uses `created_at`, not completion time.

FAST report cache TTL is one hour; DEEP is six hours. Cache keys distinguish mode and enrichment. GitLeak responses have a 24-hour TTL, and hits do not consume upstream budget. Exhaustion leaves a report with GitHub data and a provider warning. These caches can contain public identity evidence; secure the cache volume accordingly.

## Proxy and CORS

With no trusted proxies, rate limiting uses the direct socket peer and ignores X-Forwarded-For. With a trusted peer, Spoor walks the forwarded chain from right to left and selects the first untrusted address. Configure the actual ingress ranges, not arbitrary client networks.

Set exact HTTP(S) origins for production. The development fallback accepts origin strings beginning with `http://localhost` or `http://127.0.0.1`; it is not a production allowlist. The configured origin list does not enable wildcard matching. CORS does not authenticate non-browser callers.

The limiter briefly processes raw addresses to hash them with a startup salt. Its entries store hashes in memory. This does not control ingress, platform or explicitly enabled HTTP tracing logs.

## Deployment

Build the supplied Dockerfile, expose port 8080 and check `GET /healthz` for status and version. Mount the SQLite cache if results should survive restarts. Keep provider tokens in the hosting secret store, outside browser code and Git. Use HTTPS at ingress and keep HTTP DEEP disabled unless its extra cost is intended.
