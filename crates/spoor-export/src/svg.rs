use spoor_core::{ProviderHealth, Report, SpoorError};

pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let mut res: String = s.chars().take(max_len.saturating_sub(1)).collect();
        res.push('\u{2026}');
        res
    }
}

fn short_provider_name(name: &str) -> &str {
    if name.contains("REST") || name.contains("Repositories") {
        "REST"
    } else if name.contains("GraphQL") || name.contains("Contributions") {
        "GRAPHQL"
    } else if name.contains("Events") {
        "EVENTS"
    } else if name.contains("Search") {
        "SEARCH"
    } else if name.contains("Git History") || name.contains("Git") {
        "GIT"
    } else if name.contains("GitLeak") {
        "GITLEAK"
    } else if name.contains("Keys") {
        "KEYS"
    } else {
        name
    }
}

pub fn export_svg(report: &Report) -> Result<String, SpoorError> {
    let target_display = escape_xml(&truncate_str(&report.target.display_name(), 38));
    let target_type = escape_xml(&report.summary.target_type);
    let mode = escape_xml(&report.mode.to_string().to_uppercase());
    let generated = escape_xml(
        &report
            .generated_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
    );
    let canonical_url = escape_xml(&truncate_str(&report.target.canonical_url(), 60));
    let display_name = escape_xml(&truncate_str(&report.summary.display_name, 35));

    let total_stars = report.summary.total_stars_accumulated;
    let total_forks = report.summary.total_forks_accumulated;
    let repos_count = report.summary.total_public_repositories;
    let distinct_emails = report.summary.distinct_emails_observed;
    let keys_count = report.summary.public_keys_found;

    let mut id_rows = String::new();
    let mut y = 285;
    for id in report.identities.iter().take(3) {
        let name = escape_xml(&truncate_str(id.name.as_deref().unwrap_or("-"), 20));
        let email = escape_xml(&truncate_str(&id.email, 32));
        let email_type = escape_xml(id.email_type.as_str());
        let verified = if id.is_verified_signer { "YES" } else { "NO" };
        id_rows.push_str(&format!(
            r##"<g transform="translate(40, {y})">
                <rect width="425" height="46" fill="#ffffff" stroke="#e2e8f0" stroke-width="1" />
                <text class="mono" x="12" y="18" fill="#0f172a" font-size="11" font-weight="600">{email}</text>
                <text class="mono" x="12" y="34" fill="#64748b" font-size="10">{name} | {email_type} | VERIFIED: {verified}</text>
                <text class="mono" x="413" y="26" fill="#0284c7" font-size="11" font-weight="600" text-anchor="end">{commits} cmt</text>
            </g>"##,
            commits = id.commits_observed
        ));
        y += 52;
    }
    if report.identities.is_empty() {
        id_rows.push_str(
            r##"<g transform="translate(40, 285)">
                <rect width="425" height="46" fill="#ffffff" stroke="#e2e8f0" stroke-width="1" stroke-dasharray="4" />
                <text class="mono" x="16" y="28" fill="#94a3b8" font-size="11">NO PUBLIC COMMITS OR PROFILES RECORDED</text>
            </g>"##
        );
    }

    let mut repo_rows = String::new();
    let mut ry = 285;
    for r in report.repositories.iter().take(3) {
        let rname = escape_xml(&truncate_str(&r.name, 26));
        let lang = escape_xml(&truncate_str(
            r.primary_language.as_deref().unwrap_or("-"),
            14,
        ));
        let state = if r.is_archived {
            "ARCHIVED"
        } else if r.is_fork {
            "FORK"
        } else {
            "ACTIVE"
        };
        repo_rows.push_str(&format!(
            r##"<g transform="translate(495, {ry})">
                <rect width="425" height="46" fill="#ffffff" stroke="#e2e8f0" stroke-width="1" />
                <text class="mono" x="12" y="18" fill="#0f172a" font-size="11" font-weight="600">{rname}</text>
                <text class="mono" x="12" y="34" fill="#64748b" font-size="10">LANG: {lang} | STATE: {state}</text>
                <text class="mono" x="413" y="26" fill="#0f172a" font-size="11" text-anchor="end">{stars} * / {forks} f</text>
            </g>"##,
            stars = r.stars,
            forks = r.forks,
        ));
        ry += 52;
    }
    if report.repositories.is_empty() {
        repo_rows.push_str(
            r##"<g transform="translate(495, 285)">
                <rect width="425" height="46" fill="#ffffff" stroke="#e2e8f0" stroke-width="1" stroke-dasharray="4" />
                <text class="mono" x="16" y="28" fill="#94a3b8" font-size="11">NO PUBLIC REPOSITORIES CATALOGED</text>
            </g>"##
        );
    }

    let mut status_badges = String::new();
    let mut sx = 40;
    for (name, status) in report.provider_status.iter().take(5) {
        let short_name = escape_xml(short_provider_name(name));
        let p_status = escape_xml(&status.status.to_string());
        let (stroke_color, text_color) = match status.status {
            ProviderHealth::Ok => ("#10b981", "#059669"),
            ProviderHealth::Partial | ProviderHealth::RateLimited => ("#f59e0b", "#d97706"),
            ProviderHealth::Skipped => ("#94a3b8", "#64748b"),
            _ => ("#ef4444", "#dc2626"),
        };
        status_badges.push_str(&format!(
            r##"<g transform="translate({sx}, 472)">
                <rect width="166" height="26" fill="#ffffff" stroke="#e2e8f0" stroke-width="1" />
                <rect x="6" y="6" width="4" height="14" fill="{stroke_color}" />
                <text class="mono" x="16" y="17" fill="#0f172a" font-size="9" font-weight="600">{short_name}</text>
                <text class="mono" x="156" y="17" fill="{text_color}" font-size="9" font-weight="600" text-anchor="end">{p_status}</text>
            </g>"##
        ));
        sx += 178;
    }

    let commits_scanned = report.coverage.commits_scanned;
    let repos_scanned = report.coverage.repositories_scanned;
    let git_inspected = if report.coverage.git_history_inspected {
        "YES"
    } else {
        "NO"
    };

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 560" width="960" height="560">
    <style>
        .mono {{ font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace; }}
    </style>
    <!-- Background Canvas -->
    <rect width="960" height="560" fill="#f8fafc" />
    <rect x="20" y="20" width="920" height="520" fill="#ffffff" stroke="#cbd5e1" stroke-width="1" />

    <!-- Top Technical Header -->
    <g transform="translate(40, 48)">
        <text class="mono" x="0" y="0" fill="#0284c7" font-size="13" font-weight="700" letter-spacing="1.5">SPOOR AUDIT REPORT // v0.2.0</text>
        <text class="mono" x="320" y="0" fill="#94a3b8" font-size="11">PUBLIC FOOTPRINT TELEMETRY</text>
        <rect x="760" y="-12" width="120" height="20" fill="#f1f5f9" stroke="#cbd5e1" stroke-width="1" />
        <text class="mono" x="820" y="2" fill="#0f172a" font-size="10" font-weight="600" text-anchor="middle">MODE: {mode}</text>
    </g>
    <line x1="40" y1="62" x2="920" y2="62" stroke="#e2e8f0" stroke-width="1" />

    <!-- Target Metadata Strip -->
    <g transform="translate(40, 84)">
        <text class="mono" x="0" y="0" fill="#0f172a" font-size="16" font-weight="700">TARGET: {target_display} [{target_type}]</text>
        <text class="mono" x="0" y="18" fill="#64748b" font-size="10">NAME: {display_name} | URL: {canonical_url} | TIMESTAMP: {generated}</text>
    </g>

    <!-- Metrics Matrix Grid (5 blocks) -->
    <g transform="translate(40, 122)">
        <rect width="166" height="60" fill="#f8fafc" stroke="#e2e8f0" stroke-width="1" />
        <text class="mono" x="12" y="18" fill="#64748b" font-size="9" font-weight="600">REPOSITORIES</text>
        <text class="mono" x="12" y="44" fill="#0f172a" font-size="18" font-weight="700">{repos_count}</text>

        <rect x="178" width="166" height="60" fill="#f8fafc" stroke="#e2e8f0" stroke-width="1" />
        <text class="mono" x="190" y="18" fill="#64748b" font-size="9" font-weight="600">STARS ACCUMULATED</text>
        <text class="mono" x="190" y="44" fill="#0f172a" font-size="18" font-weight="700">{total_stars}</text>

        <rect x="356" width="166" height="60" fill="#f8fafc" stroke="#e2e8f0" stroke-width="1" />
        <text class="mono" x="368" y="18" fill="#64748b" font-size="9" font-weight="600">FORKS ACCUMULATED</text>
        <text class="mono" x="368" y="44" fill="#0f172a" font-size="18" font-weight="700">{total_forks}</text>

        <rect x="534" width="166" height="60" fill="#f8fafc" stroke="#e2e8f0" stroke-width="1" />
        <text class="mono" x="546" y="18" fill="#64748b" font-size="9" font-weight="600">OBSERVED IDENTITIES</text>
        <text class="mono" x="546" y="44" fill="#0f172a" font-size="18" font-weight="700">{distinct_emails}</text>

        <rect x="712" width="168" height="60" fill="#f8fafc" stroke="#e2e8f0" stroke-width="1" />
        <text class="mono" x="724" y="18" fill="#64748b" font-size="9" font-weight="600">PUBLIC KEYS</text>
        <text class="mono" x="724" y="44" fill="#0f172a" font-size="18" font-weight="700">{keys_count}</text>
    </g>

    <!-- Mid Section Titles -->
    <text class="mono" x="40" y="210" fill="#0284c7" font-size="10" font-weight="700">// 01. OBSERVABLE IDENTITIES (SAMPLE)</text>
    <text class="mono" x="495" y="210" fill="#0284c7" font-size="10" font-weight="700">// 02. REPOSITORIES &amp; COVERAGE RECORD</text>

    <!-- Coverage Sub-Bar in Right Column -->
    <g transform="translate(495, 222)">
        <rect width="425" height="34" fill="#f8fafc" stroke="#e2e8f0" stroke-width="1" />
        <text class="mono" x="12" y="14" fill="#64748b" font-size="9">COMMITS SCANNED: <tspan fill="#0f172a" font-weight="600">{commits_scanned}</tspan> | REPOS: <tspan fill="#0f172a" font-weight="600">{repos_scanned}</tspan></text>
        <text class="mono" x="12" y="26" fill="#64748b" font-size="9">GIT INSPECTION: <tspan fill="#0f172a" font-weight="600">{git_inspected}</tspan> | PROVENANCE: <tspan fill="#059669" font-weight="600">VERIFIED METADATA</tspan></text>
    </g>

    <!-- Table Rows -->
    {id_rows}
    {repo_rows}

    <!-- Telemetry Title -->
    <text class="mono" x="40" y="460" fill="#0284c7" font-size="10" font-weight="700">// 03. UPSTREAM PROVIDER TELEMETRY</text>
    {status_badges}

    <!-- Footer -->
    <line x1="40" y1="510" x2="920" y2="510" stroke="#e2e8f0" stroke-width="1" />
    <text class="mono" x="40" y="525" fill="#94a3b8" font-size="9">PROVENANCE: Public Git commit headers &amp; GitHub public API endpoints // SOURCED FACTS ONLY</text>
    <text class="mono" x="920" y="525" fill="#94a3b8" font-size="9" text-anchor="end">ENGINE: Spoor by Voldigoade</text>
</svg>"##
    );

    Ok(svg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoor_core::{AnalysisMode, Target};

    #[test]
    fn exports_valid_svg_and_escapes() {
        let mut report = Report::new(
            Target::User {
                username: "torvalds".into(),
            },
            AnalysisMode::Fast,
        );
        report.summary.display_name = "Linus & Friends <linux>".into();
        let svg = export_svg(&report).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("Spoor by Voldigoade"));
        assert!(svg.contains("Linus &amp; Friends &lt;linux&gt;"));
        assert!(!svg.contains("<linux>"));
    }

    #[test]
    fn handles_long_strings_without_corrupting_svg() {
        let mut report = Report::new(
            Target::User {
                username: "a-very-long-github-username-that-would-otherwise-overflow".into(),
            },
            AnalysisMode::Fast,
        );
        report.summary.display_name =
            "A Extremely Long Display Name Designed To Test Boundary Checks Inside SVG Exporter"
                .into();
        report.identities.push(spoor_core::IdentityRecord {
            name: Some("Long Contributor Name With Many Characters".into()),
            email: "extremely.long.email.address.associated.with.corporate.entity@subdomain.example.com".into(),
            email_type: spoor_core::EmailType::DomainEmail,
            first_seen: None,
            last_seen: None,
            commits_observed: 42,
            repositories: vec!["long-repo-name".into()],
            sources: vec!["Git".into()],
            is_verified_signer: true,
        });

        let svg = export_svg(&report).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }
}
