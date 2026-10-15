use spoor_core::{Report, SpoorError};
use std::fmt::Write;

pub fn export_text(report: &Report) -> Result<String, SpoorError> {
    let mut out = String::with_capacity(8192);

    let divider = "=".repeat(80);
    let sub_divider = "-".repeat(80);

    writeln!(out, "{divider}").unwrap();
    writeln!(out, "SPOOR - GITHUB PUBLIC-FOOTPRINT REPORT").unwrap();
    writeln!(out, "by Voldigoade").unwrap();
    writeln!(out, "{divider}").unwrap();

    writeln!(out, "TARGET:          {}", report.target.display_name()).unwrap();
    writeln!(out, "CANONICAL URL:   {}", report.target.canonical_url()).unwrap();
    writeln!(out, "MODE:            {}", report.mode).unwrap();
    writeln!(
        out,
        "GENERATED AT:    {}",
        report.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
    )
    .unwrap();
    writeln!(out, "SCHEMA VERSION:  {}", report.schema_version).unwrap();

    writeln!(out, "\n{sub_divider}").unwrap();
    writeln!(out, "SUMMARY").unwrap();
    writeln!(out, "{sub_divider}").unwrap();
    writeln!(out, "Display Name:         {}", report.summary.display_name).unwrap();
    writeln!(out, "Target Type:          {}", report.summary.target_type).unwrap();
    if let Some(created) = report.summary.account_created {
        writeln!(
            out,
            "Account Created:      {}",
            created.format("%Y-%m-%d %H:%M:%S UTC")
        )
        .unwrap();
    }
    writeln!(
        out,
        "Public Repositories:  {}",
        report.summary.total_public_repositories
    )
    .unwrap();
    writeln!(
        out,
        "Total Stars:          {}",
        report.summary.total_stars_accumulated
    )
    .unwrap();
    writeln!(
        out,
        "Total Forks:          {}",
        report.summary.total_forks_accumulated
    )
    .unwrap();
    writeln!(
        out,
        "Distinct Emails:      {}",
        report.summary.distinct_emails_observed
    )
    .unwrap();
    writeln!(
        out,
        "Public Keys:          {}",
        report.summary.public_keys_found
    )
    .unwrap();
    writeln!(
        out,
        "Fork Ratio:           {:.1}%",
        report.summary.fork_ratio * 100.0
    )
    .unwrap();
    writeln!(
        out,
        "Archived Ratio:       {:.1}%",
        report.summary.archived_ratio * 100.0
    )
    .unwrap();

    if !report.summary.primary_languages.is_empty() {
        let langs_str = report
            .summary
            .primary_languages
            .iter()
            .take(6)
            .map(|(l, c)| format!("{l} ({c})"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "Primary Languages:    {langs_str}").unwrap();
    }

    if let Some(prof) = &report.profile {
        writeln!(out, "\n{sub_divider}").unwrap();
        writeln!(out, "PUBLIC PROFILE").unwrap();
        writeln!(out, "{sub_divider}").unwrap();
        writeln!(out, "Login:         {}", prof.login).unwrap();
        writeln!(out, "Numeric ID:    {}", prof.id).unwrap();
        if let Some(name) = &prof.name {
            writeln!(out, "Name:          {}", name).unwrap();
        }
        if let Some(company) = &prof.company {
            writeln!(out, "Company:       {}", company).unwrap();
        }
        if let Some(loc) = &prof.location {
            writeln!(out, "Location:      {}", loc).unwrap();
        }
        if let Some(email) = &prof.public_email {
            writeln!(out, "Public Email:  {}", email).unwrap();
        }
        if let Some(website) = &prof.blog_url {
            if !website.is_empty() {
                writeln!(out, "Website:       {}", website).unwrap();
            }
        }
        writeln!(out, "Followers:     {}", prof.followers).unwrap();
        writeln!(out, "Following:     {}", prof.following).unwrap();
        writeln!(out, "Account Age:   {} days", prof.account_age_days).unwrap();
    }

    writeln!(out, "\n{sub_divider}").unwrap();
    writeln!(out, "PUBLIC IDENTITIES OBSERVED").unwrap();
    writeln!(out, "{sub_divider}").unwrap();
    if report.identities.is_empty() {
        writeln!(out, "No identities observed.").unwrap();
    } else {
        for (i, id) in report.identities.iter().enumerate() {
            writeln!(out, "[{}] Email:        {}", i + 1, id.email).unwrap();
            if let Some(name) = &id.name {
                writeln!(out, "    Name:         {}", name).unwrap();
            }
            writeln!(out, "    Type:         {}", id.email_type.as_str()).unwrap();
            writeln!(out, "    Commits:      {}", id.commits_observed).unwrap();
            writeln!(
                out,
                "    Verified:     {}",
                if id.is_verified_signer { "Yes" } else { "No" }
            )
            .unwrap();
            if let Some(first) = id.first_seen {
                writeln!(out, "    First Seen:   {}", first.format("%Y-%m-%d")).unwrap();
            }
            if let Some(last) = id.last_seen {
                writeln!(out, "    Last Seen:    {}", last.format("%Y-%m-%d")).unwrap();
            }
            if !id.repositories.is_empty() {
                writeln!(out, "    Repos:        {}", id.repositories.join(", ")).unwrap();
            }
            writeln!(out, "    Sources:      {}", id.sources.join(", ")).unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "{sub_divider}").unwrap();
    writeln!(out, "PUBLIC REPOSITORIES (Sample)").unwrap();
    writeln!(out, "{sub_divider}").unwrap();
    for r in report.repositories.iter().take(15) {
        let state = if r.is_archived {
            "ARCHIVED"
        } else if r.is_fork {
            "FORK"
        } else {
            "ACTIVE"
        };
        writeln!(
            out,
            "  * {:<30} stars: {:<6} forks: {:<6} lang: {:<12} [{}]",
            r.name,
            r.stars,
            r.forks,
            r.primary_language.as_deref().unwrap_or("—"),
            state
        )
        .unwrap();
    }
    if report.repositories.len() > 15 {
        writeln!(
            out,
            "  ... and {} more repositories",
            report.repositories.len() - 15
        )
        .unwrap();
    }

    if !report.keys.is_empty() {
        writeln!(out, "\n{sub_divider}").unwrap();
        writeln!(out, "PUBLIC KEYS").unwrap();
        writeln!(out, "{sub_divider}").unwrap();
        for k in &report.keys {
            writeln!(
                out,
                "  [{}] ID: {:<16} Preview: {}",
                k.key_type,
                k.key_id.as_deref().unwrap_or("—"),
                k.raw_preview
            )
            .unwrap();
        }
    }

    if !report.inferences.is_empty() {
        writeln!(out, "\n{sub_divider}").unwrap();
        writeln!(out, "INFERENCES").unwrap();
        writeln!(out, "{sub_divider}").unwrap();
        for inf in &report.inferences {
            writeln!(out, "* Topic:       {}", inf.topic).unwrap();
            writeln!(out, "  Inference:   {}", inf.inference).unwrap();
            writeln!(out, "  Class:       {}", inf.classification).unwrap();
            writeln!(out, "  Confidence:  {}", inf.confidence).unwrap();
            writeln!(out, "  Limitation:  {}", inf.limitation).unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "{sub_divider}").unwrap();
    writeln!(out, "COVERAGE & PROVIDER STATUS").unwrap();
    writeln!(out, "{sub_divider}").unwrap();
    writeln!(
        out,
        "Coverage: {} (Commits: {}, Repos: {}/{})",
        if report.coverage.is_partial {
            "PARTIAL"
        } else {
            "COMPLETE"
        },
        report.coverage.commits_scanned,
        report.coverage.repositories_scanned,
        report.coverage.repositories_total
    )
    .unwrap();
    for (provider, st) in &report.provider_status {
        writeln!(
            out,
            "  {:<24} {:<12} items: {:<6} {}",
            provider,
            st.status.to_string(),
            st.items_retrieved
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            st.message.as_deref().unwrap_or("")
        )
        .unwrap();
    }

    writeln!(out, "\n{sub_divider}").unwrap();
    writeln!(out, "LIMITATIONS").unwrap();
    writeln!(out, "{sub_divider}").unwrap();
    for lim in &report.limitations {
        writeln!(out, "* {lim}").unwrap();
    }

    writeln!(out, "\n{divider}").unwrap();
    writeln!(out, "Spoor by Voldigoade").unwrap();
    writeln!(out, "{divider}").unwrap();

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoor_core::{AnalysisMode, Target};

    #[test]
    fn exports_plain_text() {
        let report = Report::new(
            Target::User {
                username: "torvalds".into(),
            },
            AnalysisMode::Fast,
        );
        let txt = export_text(&report).unwrap();
        assert!(txt.contains("SPOOR - GITHUB PUBLIC-FOOTPRINT REPORT"));
        assert!(txt.contains("by Voldigoade"));
        assert!(!txt.contains("\x1b[")); // No ANSI codes!
    }
}
