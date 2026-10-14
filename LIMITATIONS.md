# Limitations & Operational Boundaries

Spoor is built on the philosophy of empirical correctness, explicit provenance, and strict distinction between facts and inferences. To use and interpret Spoor reports accurately, understand the following operational constraints.

---

## 1. Strictly Public Data Scope

* Spoor queries only publicly reachable GitHub API endpoints, Git public metadata, and public enrichment services.
* It does not access private repositories, private organizations, private gists, or private email configurations unless the account owner has explicitly made those attributes public.
* Deleted commits, force-pushed history, or purged accounts are generally not recoverable once pruned by GitHub's upstream garbage collection.

---

## 2. GitHub API Retention & Windowing

* **Public Events:** GitHub's public event stream (`/users/{username}/events/public`) only retains activity generated within the previous 30 days, capped at a maximum of 300 events across 3 pages. Reports explicitly note this window and never claim to represent complete lifetime activity history.
* **Commit Budgets:** Full commit history for multi-thousand-commit repositories is neither practical nor polite to scan over public APIs in interactive workflows. Spoor bounds commit inspection by default:
  * **FAST mode:** inspects up to 3 most recently pushed repositories, sampling up to 30 commits per repository.
  * **DEEP mode:** inspects up to 8 repositories, sampling up to 50 commits per repository.
  * Any report where fewer than 100% of repositories or commits were inspected is explicitly flagged with `coverage.is_partial = true`.

---

## 3. GitLeak Quota & Upstream Timeouts

* GitLeak operates with a documented rate limit of 50 requests per hour per client.
* GitLeak enforces an upstream timeout of approximately 8 seconds for prolific accounts.
* In **FAST mode**, GitLeak is skipped by default to conserve quota for deep inspection.
* If GitLeak returns `429 Too Many Requests`, `408 Request Timeout`, or `422 Unprocessable Entity`, Spoor records the condition in `provider_status` and continues assembling the remaining report from GitHub REST and GraphQL data. Upstream enrichment failure does not fail the report.

---

## 4. Inferences vs. Observed Facts

Spoor adheres to a strict classification model:

* **OBSERVED:** Returned directly by a primary authoritative source (e.g. GitHub user profile, raw Git commit author header).
* **DERIVED:** Calculated mathematically from observed data (e.g. language percentage byte sum, repository star totals).
* **INFERRED:** Algorithmic deductions (e.g. peak activity hour window, commit timestamp cluster).
* **PARTIAL:** Sourced with known coverage boundaries (e.g. pagination caps or sample limits).
* **UNKNOWN:** Unestablished facts.

### Timezone & Working Hours Inferences

* When commit timestamps cluster around specific UTC offsets, Spoor may record an inference regarding peak activity windows.
* **This is an inference, never a fact.** Commit timestamps can reflect automated CI bots, travel, irregular sleeping hours, or misconfigured local system clocks.
* Spoor never equates timezone inferences with physical location, residency, or nationality.

---

## 5. Cache Freshness & Invalidation

* To prevent rate-limit exhaustion, Spoor caches completed reports and raw provider responses in a local SQLite database (`spoor/cache.sqlite`).
* Default Time-To-Live (TTL):
  * **FAST reports:** 1 hour
  * **DEEP reports:** 6 hours
  * **GitLeak responses:** 24 hours
* Cached results are marked with `CACHED` in the provider status map. Users can bypass or reset cache state via:
  ```bash
  spoor cache clear
  ```
