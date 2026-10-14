# Security Policy

## Reporting Security Vulnerabilities

Spoor is a public footprint research tool and security auditing instrument designed for responsible public metadata inspection.

If you believe you have found a security vulnerability in Spoor—such as a potential Server-Side Request Forgery (SSRF) bypass, denial of service vector, or credential handling issue—please report it responsibly through GitHub Private Vulnerability Reporting:

1. Navigate to the repository at `https://github.com/Voldigoade/spoor`.
2. Click on the **Security** tab.
3. Select **Report a vulnerability** to open an advisory draft.

Do not file public GitHub issues for undisclosed security vulnerabilities.

## Security Guarantees & Boundaries

Spoor implements strict architectural safeguards:

* **Strict SSRF Immunity:** The target parser validates hostnames prior to issuing any HTTP request. All targets must resolve strictly to `github.com` or `www.github.com`. Requests to private subnets, loopback addresses (`127.0.0.1`), link-local metadata endpoints (`169.254.169.254`), or non-GitHub hosts are rejected at parse time.
* **No Remote Code Execution:** Spoor never runs package managers (`npm`, `cargo`, `pip`, etc.) or executes scripts from target repositories. It operates as an HTTP API client and metadata inspector only.
* **Ephemeral Abuse Protection:** The hosted HTTP server uses an in-memory salted hash of incoming IP addresses with sliding time windows to enforce rate limits. Raw IP addresses are never persisted to disk or emitted in logs.
* **Token Isolation:** Server authentication tokens (such as `GITHUB_TOKEN`) remain in server process memory and are never exposed via API endpoints, exports, or error traces.
