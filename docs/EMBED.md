# Embedding Spoor

The v0.2.0 service exposes asynchronous lookups under `/v1`. Reports use schema `1.2`. The [TypeScript reference client](../packages/spoor-client/src/index.ts) wraps these endpoints; it does not handle retries, polling recovery or UI state for you.

## Requests

| Method | Path | Response |
| --- | --- | --- |
| GET | `/healthz` | 200, status and version |
| POST | `/v1/lookups` | 202, lookup metadata |
| GET | `/v1/lookups/{id}` | 200, current metadata |
| GET | `/v1/lookups/{id}/events` | 200, `text/event-stream` |
| GET | `/v1/lookups/{id}/report` | 200, canonical report |
| GET | `/v1/lookups/{id}/export/{format}` | 200, file bytes |

```http
POST /v1/lookups
Content-Type: application/json

{"target":"Voldigoade","mode":"fast","enrich":true}
```

`target` accepts a username, `owner/repo` or a GitHub URL. `mode` defaults to `fast`. Omitted `enrich` is false for FAST and true for DEEP; an explicit value overrides that default. Enriched FAST keeps the FAST commit budget. DEEP requires `SPOOR_HTTP_DEEP_ENABLED=true`. Repository targets skip GitLeak regardless of enrichment.

The creation response includes `id`, `target`, `mode`, `enrich`, `status` and `created_at`. Status is `pending`, `running`, `completed` or `failed`; failed metadata includes `error`. Store the returned ID rather than reconstructing it from the target.

## Progress and completion

SSE uses named events with a JSON envelope. For example:

```text
event: provider.completed
data: {"event":"provider_completed","data":{"provider":"GitHub REST","details":"Profile resolved"}}
```

| Event name | Data fields |
| --- | --- |
| `lookup.started` | target, mode |
| `target.resolved` | target, target_type |
| `provider.started` | provider, operation |
| `provider.completed` | provider, details |
| `provider.warning` | provider, message |
| `scan.progress` | current, total, message |
| `analysis.completed` | duration_ms |
| `lookup.failed` | error |

The JSON event discriminator uses underscores; the SSE event name uses dots. The reference client unwraps `data` for consumers.

Events are broadcast without replay. A cached lookup may complete before subscription, and `analysis.completed` can arrive before the server stores the report. Poll `GET /v1/lookups/{id}` immediately and periodically, including after a disconnected stream. Retrieve the report when metadata says `completed`. Report retrieval before completion returns 202 with code `in_progress`; export retrieval returns 409 with `report_not_ready`.

Close EventSource and stop timers after completion, failure, cancellation or unmount. Ignore old callbacks when another lookup starts. Cancellation in the browser stops observation, not server analysis. Use bounded retries for transient status/report failures; automatically retrying POST may create duplicate work and consume quota.

```ts
const lookup = await client.startLookup('Voldigoade', 'fast', true);
const unsubscribe = client.streamEvents(lookup.id, renderProgress, recoverWithPolling);
// Poll metadata until completed/failed, then close the stream and fetch the report.
```

`renderProgress` and `recoverWithPolling` above are application callbacks. The reference client is source code, not a published package or a complete lookup lifecycle manager.

## Reports and files

The canonical [Rust report](../crates/spoor-core/src/report.rs) defines schema `1.2`. Its top-level fields are `schema_version`, `generator`, `target`, `mode`, `generated_at`, `summary`, optional `profile`, `repositories`, `activity`, `identities`, `keys`, `inferences`, `provider_status`, `coverage`, `provenance`, `warnings` and `limitations`. Generator version is `0.2.0` in this release.

Optional fields may be absent or null, depending on their serializer. Preserve partial coverage, provider warnings and evidence sources. Identity classification serializes as snake_case Rust variant names, for example `git_hub_no_reply` and `git_hub_system`. Summary address counts exclude system identities. Historical addresses are evidence, not current contact details. Activity is a bounded public window, not lifetime history. GitHub exposes up to 300 public events from the previous 30 days, with at most 100 per page. Spoor follows up to three pages and filters events older than 30 days. The report's `activity.window_note` describes this bound and any collection warning. The `provider_status["Public Events"]` entry is `ok` when the requested window was collected, `partial` when a later page or the three-page cap prevented a full response, and `unavailable` when the first page failed or `rate_limited` when GitHub returns HTTP 403/429. On later-page failure, returned events are partial data and the warning is preserved; a first-page failure returns no events and records the provider error. Zero events do not prove inactivity.

Export formats are `json`, `md`, `txt`, `svg` and `pdf`. Download actual response bytes. Responses include Content-Type and Content-Disposition; browser CORS may not expose the filename header, so use a safe fallback filename. A failed export should remain an error, not a fabricated client-side substitute.

## Errors

Bodies contain `error` and `code`. Rate-limit bodies also include retry information; honor Retry-After without automatically creating a new lookup.

| HTTP | Code | Meaning |
| --- | --- | --- |
| 400 | `invalid_target` | Unsupported or malformed target |
| 403 | `mode_disabled` | HTTP DEEP disabled |
| 404 | `not_found` | Lookup ID missing or expired |
| 202 | `in_progress` | Report not ready |
| 409 | `report_not_ready` | Export not ready |
| 422 | `lookup_failed` | Analysis failed |
| 429 | `rate_limited` | Per-IP or global creation limit |
| 500 | `internal_error` | Server error |

## Browser and deployment boundary

Browser code needs the public API origin, not GitHub tokens or provider credentials. Configure exact allowed origins on the service. CORS does not protect against non-browser clients. No provider calls or analysis engine need to be duplicated in the frontend.

Run a single server replica and configure trusted ingress ranges. Lookup retention, quotas, cache persistence and logging boundaries are documented in [operations](OPERATIONS.md).
