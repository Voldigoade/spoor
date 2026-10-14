use spoor_core::{Report, SpoorError};

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

pub fn export_svg(report: &Report) -> Result<String, SpoorError> {
    let target_display = escape_xml(&report.target.display_name());
    let target_type = escape_xml(&report.summary.target_type);
    let mode = escape_xml(&report.mode.to_string().to_uppercase());
    let generated = escape_xml(
        &report
            .generated_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
    );
    let display_name = escape_xml(&report.summary.display_name);

    let total_stars = report.summary.total_stars_accumulated;
    let total_forks = report.summary.total_forks_accumulated;
    let repos_count = report.summary.total_public_repositories;
    let distinct_emails = report.summary.distinct_emails_observed;
    let keys_count = report.summary.public_keys_found;

    // Identities preview (up to 3)
    let mut id_rows = String::new();
    let mut y = 295;
    for id in report.identities.iter().take(3) {
        let name = escape_xml(id.name.as_deref().unwrap_or("—"));
        let email = escape_xml(&id.email);
        let email_type = escape_xml(id.email_type.as_str());
        id_rows.push_str(&format!(
            r##"<g transform="translate(40, {y})">
                <rect width="420" height="48" rx="6" fill="#161b22" stroke="#30363d" />
                <text x="16" y="20" fill="#f0f6fc" font-size="13" font-weight="600">{email}</text>
                <text x="16" y="37" fill="#8b949e" font-size="11">{name} · {email_type}</text>
                <text x="404" y="28" fill="#58a6ff" font-size="12" font-weight="600" text-anchor="end">{commits} cmt</text>
            </g>"##,
            commits = id.commits_observed
        ));
        y += 58;
    }
    if report.identities.is_empty() {
        id_rows.push_str(r##"<text x="40" y="310" fill="#8b949e" font-size="13">No public commit or profile emails observed</text>"##);
    }

    // Top repos preview (up to 3)
    let mut repo_rows = String::new();
    let mut ry = 295;
    for r in report.repositories.iter().take(3) {
        let rname = escape_xml(&r.name);
        let lang = escape_xml(r.primary_language.as_deref().unwrap_or("—"));
        repo_rows.push_str(&format!(
            r##"<g transform="translate(500, {ry})">
                <rect width="420" height="48" rx="6" fill="#161b22" stroke="#30363d" />
                <text x="16" y="20" fill="#58a6ff" font-size="13" font-weight="600">{rname}</text>
                <text x="16" y="37" fill="#8b949e" font-size="11">Language: {lang}</text>
                <text x="404" y="28" fill="#f0f6fc" font-size="12" text-anchor="end">★ {stars}</text>
            </g>"##,
            stars = r.stars
        ));
        ry += 58;
    }
    if report.repositories.is_empty() {
        repo_rows.push_str(r##"<text x="500" y="310" fill="#8b949e" font-size="13">No public repositories cataloged</text>"##);
    }

    // Provider status badges
    let mut status_badges = String::new();
    let mut sx = 500;
    for (name, status) in report.provider_status.iter().take(4) {
        let p_name = escape_xml(name);
        let p_status = escape_xml(&status.status.to_string());
        status_badges.push_str(&format!(
            r##"<g transform="translate({sx}, 490)">
                <rect width="98" height="24" rx="4" fill="#21262d" />
                <text x="49" y="16" fill="#c9d1d9" font-size="10" font-weight="500" text-anchor="middle">{p_name}: {p_status}</text>
            </g>"##
        ));
        sx += 106;
    }

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 560" width="960" height="560">
    <style>
        .mono {{ font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace; }}
        .sans {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; }}
    </style>
    <rect width="960" height="560" fill="#0d1117" rx="12" />
    <rect x="1" y="1" width="958" height="558" fill="none" stroke="#30363d" stroke-width="1" rx="11" />

    <!-- Top Bar -->
    <g transform="translate(40, 40)">
        <text class="sans" x="0" y="24" fill="#58a6ff" font-size="22" font-weight="700" letter-spacing="1">SPOOR</text>
        <text class="sans" x="90" y="24" fill="#8b949e" font-size="13">by Voldigoade</text>
        <rect x="790" y="4" width="90" height="26" rx="4" fill="#1f6feb" fill-opacity="0.2" stroke="#388bfd" stroke-width="1" />
        <text class="mono" x="835" y="21" fill="#58a6ff" font-size="11" font-weight="600" text-anchor="middle">{mode}</text>
    </g>

    <!-- Target Title Header -->
    <g transform="translate(40, 90)">
        <text class="sans" x="0" y="26" fill="#f0f6fc" font-size="24" font-weight="600">{target_display}</text>
        <text class="sans" x="0" y="46" fill="#8b949e" font-size="13">{display_name} · {target_type} · Generated: {generated}</text>
    </g>

    <!-- Metrics Grid -->
    <g transform="translate(40, 160)">
        <rect width="168" height="74" rx="8" fill="#161b22" stroke="#30363d" />
        <text class="sans" x="16" y="26" fill="#8b949e" font-size="11" font-weight="500">PUBLIC REPOSITORIES</text>
        <text class="mono" x="16" y="55" fill="#f0f6fc" font-size="20" font-weight="600">{repos_count}</text>

        <rect x="184" width="168" height="74" rx="8" fill="#161b22" stroke="#30363d" />
        <text class="sans" x="200" y="26" fill="#8b949e" font-size="11" font-weight="500">TOTAL STARS</text>
        <text class="mono" x="200" y="55" fill="#f0f6fc" font-size="20" font-weight="600">{total_stars}</text>

        <rect x="368" width="168" height="74" rx="8" fill="#161b22" stroke="#30363d" />
        <text class="sans" x="384" y="26" fill="#8b949e" font-size="11" font-weight="500">TOTAL FORKS</text>
        <text class="mono" x="384" y="55" fill="#f0f6fc" font-size="20" font-weight="600">{total_forks}</text>

        <rect x="552" width="168" height="74" rx="8" fill="#161b22" stroke="#30363d" />
        <text class="sans" x="568" y="26" fill="#8b949e" font-size="11" font-weight="500">OBSERVED IDENTITIES</text>
        <text class="mono" x="568" y="55" fill="#f0f6fc" font-size="20" font-weight="600">{distinct_emails}</text>

        <rect x="736" width="144" height="74" rx="8" fill="#161b22" stroke="#30363d" />
        <text class="sans" x="752" y="26" fill="#8b949e" font-size="11" font-weight="500">PUBLIC KEYS</text>
        <text class="mono" x="752" y="55" fill="#f0f6fc" font-size="20" font-weight="600">{keys_count}</text>
    </g>

    <!-- Column Headers -->
    <text class="sans" x="40" y="275" fill="#8b949e" font-size="12" font-weight="600">PUBLIC IDENTITIES (SAMPLED)</text>
    <text class="sans" x="500" y="275" fill="#8b949e" font-size="12" font-weight="600">REPOSITORIES</text>

    <!-- Rows -->
    {id_rows}
    {repo_rows}

    <!-- Provider Statuses -->
    <text class="sans" x="500" y="480" fill="#8b949e" font-size="11" font-weight="600">PROVIDER RECONNAISSANCE STATUS</text>
    {status_badges}

    <!-- Footer -->
    <line x1="40" y1="525" x2="920" y2="525" stroke="#21262d" />
    <text class="sans" x="40" y="545" fill="#6e7681" font-size="11">Spoor public GitHub footprint exploration · Sourced &amp; provenance-backed facts</text>
    <text class="sans" x="920" y="545" fill="#6e7681" font-size="11" text-anchor="end">Spoor by Voldigoade</text>
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
}
