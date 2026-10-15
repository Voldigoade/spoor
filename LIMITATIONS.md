# Limitations

Spoor reads public GitHub metadata and optional public enrichment. It cannot retrieve private repositories or restore history removed by upstream providers.

## Scan coverage

User FAST scans up to 3 repositories and 30 commits each; DEEP scans up to 8 repositories and 50 commits each. Non-forks are preferred. The report records sampled repositories, commits, provider status and partial coverage. These bounds are not a complete history scan, including in DEEP mode.

Public activity comes from GitHub's bounded event window: events from the previous 30 days, with at most 300 returned. Spoor requests up to three pages of 100, follows the API's next-page link and stops when there is no next page or a page is short. Events older than 30 days are filtered out. A first-page failure marks Public Events unavailable, or rate limited for HTTP 403/429; a later-page failure preserves collected events and marks coverage partial with a warning. Zero returned events does not establish inactivity.

## Enrichment and caches

FAST skips GitLeak unless enrichment is requested. Enriched FAST and DEEP can return cached enrichment or record rate-limit, timeout and availability warnings. Repository targets skip GitLeak. The local budget defaults to 40 uncached requests/hour; upstream limits can be stricter. Budget failure does not invalidate the GitHub portion.

SQLite caches FAST reports for one hour, DEEP reports for six hours and GitLeak responses for 24 hours. Provider status marks reused data as cached. `spoor cache clear` removes the local cache. HTTP lookup retention is separate; see [operations](docs/OPERATIONS.md).

## Interpreting evidence

Observed fields come from public sources. Derived values are calculated from those fields. Inferences, such as activity-hour patterns, remain deductions with a stated basis and limitation. Partial means coverage is bounded; unknown means a fact was not established.

Commit author and committer fields can contain historical, shared, automated or misleading identities. An observed address is not proof of ownership or a current contact address. GitHub system identities are excluded from the distinct human-address summary, but remain in the canonical evidence. A verified commit signature does not make every field a verified claim.

Timestamp patterns can reflect bots, travel or clock errors. They do not establish residence, nationality or working hours.
