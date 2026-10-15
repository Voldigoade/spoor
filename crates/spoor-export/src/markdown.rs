use spoor_core::{Report, SpoorError};
use std::fmt::Write;

pub fn export_markdown(report: &Report) -> Result<String, SpoorError> {
    let mut out = String::with_capacity(8192);

    writeln!(out, "# Spoor").unwrap();
    writeln!(out, "GitHub Public-Footprint Inspection Report\n").unwrap();

    writeln!(out, "| Property | Value |").unwrap();
    writeln!(out, "| :--- | :--- |").unwrap();
    writeln!(out, "| **Target** | `{}` |", report.target.display_name()).unwrap();
    writeln!(
        out,
        "| **Canonical URL** | {} |",
        report.target.canonical_url()
    )
    .unwrap();
    writeln!(out, "| **Mode** | {} |", report.mode).unwrap();
    writeln!(
        out,
        "| **Generated** | {} |",
        report.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
    )
    .unwrap();
    writeln!(out, "| **Schema Version** | {} |\n", report.schema_version).unwrap();

    writeln!(out, "## Summary\n").unwrap();
    writeln!(out, "- **Display Name:** {}", report.summary.display_name).unwrap();
    writeln!(out, "- **Target Type:** {}", report.summary.target_type).unwrap();
    if let Some(created) = report.summary.account_created {
        writeln!(
            out,
            "- **Account Created:** {}",
            created.format("%Y-%m-%d %H:%M:%S UTC")
        )
        .unwrap();
    }
    writeln!(
        out,
        "- **Public Repositories:** {}",
        report.summary.total_public_repositories
    )
    .unwrap();
    writeln!(
        out,
        "- **Total Stars Accumulated:** {}",
        report.summary.total_stars_accumulated
    )
    .unwrap();
    writeln!(
        out,
        "- **Total Forks Accumulated:** {}",
        report.summary.total_forks_accumulated
    )
    .unwrap();
    writeln!(
        out,
        "- **Distinct Emails Observed:** {}",
        report.summary.distinct_emails_observed
    )
    .unwrap();
    writeln!(
        out,
        "- **Public Keys Found:** {}",
        report.summary.public_keys_found
    )
    .unwrap();
    writeln!(
        out,
        "- **Fork Ratio:** {:.1}%",
        report.summary.fork_ratio * 100.0
    )
    .unwrap();
    writeln!(
        out,
        "- **Archived Ratio:** {:.1}%\n",
        report.summary.archived_ratio * 100.0
    )
    .unwrap();

    if !report.summary.primary_languages.is_empty() {
        writeln!(out, "### Primary Languages\n").unwrap();
        writeln!(out, "| Language | Repositories / Weight |").unwrap();
        writeln!(out, "| :--- | :--- |").unwrap();
        for (lang, count) in report.summary.primary_languages.iter().take(8) {
            writeln!(out, "| {} | {} |", lang, count).unwrap();
        }
        writeln!(out).unwrap();
    }

    if let Some(prof) = &report.profile {
        writeln!(out, "## Public Profile\n").unwrap();
        writeln!(out, "| Field | Information |").unwrap();
        writeln!(out, "| :--- | :--- |").unwrap();
        writeln!(out, "| Login | `{}` |", prof.login).unwrap();
        writeln!(out, "| Numeric ID | `{}` |", prof.id).unwrap();
        if let Some(name) = &prof.name {
            writeln!(out, "| Name | {} |", name).unwrap();
        }
        if let Some(bio) = &prof.bio {
            writeln!(out, "| Bio | {} |", bio.replace('\n', " ")).unwrap();
        }
        if let Some(company) = &prof.company {
            writeln!(out, "| Company | {} |", company).unwrap();
        }
        if let Some(location) = &prof.location {
            writeln!(out, "| Location | {} |", location).unwrap();
        }
        if let Some(email) = &prof.public_email {
            writeln!(out, "| Public Profile Email | `{}` |", email).unwrap();
        }
        if let Some(blog) = &prof.blog_url {
            if !blog.is_empty() {
                writeln!(out, "| Website | {} |", blog).unwrap();
            }
        }
        if let Some(tw) = &prof.twitter_username {
            writeln!(out, "| Social Handle | @{} |", tw).unwrap();
        }
        writeln!(out, "| Followers | {} |", prof.followers).unwrap();
        writeln!(out, "| Following | {} |", prof.following).unwrap();
        writeln!(out, "| Public Repos | {} |", prof.public_repos).unwrap();
        writeln!(out, "| Public Gists | {} |", prof.public_gists).unwrap();
        writeln!(out, "| Account Age | {} days |\n", prof.account_age_days).unwrap();
    }

    writeln!(out, "## Public Identities\n").unwrap();
    if report.identities.is_empty() {
        writeln!(
            out,
            "*No public commit author or profile email identities observed.*\n"
        )
        .unwrap();
    } else {
        writeln!(out, "| Name | Observed Email | Classification | Commits | Repositories | Verified Signer | First Seen | Last Seen |").unwrap();
        writeln!(
            out,
            "| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |"
        )
        .unwrap();
        for id in &report.identities {
            let name_str = id.name.as_deref().unwrap_or("—");
            let verified_str = if id.is_verified_signer { "Yes" } else { "No" };
            let first_str = id
                .first_seen
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "—".into());
            let last_str = id
                .last_seen
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "—".into());
            let repos_str = if id.repositories.is_empty() {
                "—".to_string()
            } else {
                id.repositories.join(", ")
            };
            writeln!(
                out,
                "| {} | `{}` | {} | {} | {} | {} | {} | {} |",
                name_str,
                id.email,
                id.email_type.as_str(),
                id.commits_observed,
                repos_str,
                verified_str,
                first_str,
                last_str
            )
            .unwrap();
        }
        writeln!(out, "\n> [!NOTE]\n> Email addresses listed above were observed in public Git commit headers or public profile attributes. They do not assert private contact status.\n").unwrap();
    }

    writeln!(out, "## Repositories\n").unwrap();
    if report.repositories.is_empty() {
        writeln!(out, "*No repositories listed.*\n").unwrap();
    } else {
        writeln!(
            out,
            "| Repository | Language | Stars | Forks | State | Default Branch |"
        )
        .unwrap();
        writeln!(out, "| :--- | :--- | :--- | :--- | :--- | :--- |").unwrap();
        for r in report.repositories.iter().take(25) {
            let state = if r.is_archived {
                "Archived"
            } else if r.is_fork {
                "Fork"
            } else {
                "Active"
            };
            writeln!(
                out,
                "| [{}]({}) | {} | {} | {} | {} | `{}` |",
                r.name,
                r.html_url,
                r.primary_language.as_deref().unwrap_or("—"),
                r.stars,
                r.forks,
                state,
                r.default_branch
            )
            .unwrap();
        }
        if report.repositories.len() > 25 {
            writeln!(
                out,
                "\n*(Displaying top 25 of {} repositories)*\n",
                report.repositories.len()
            )
            .unwrap();
        } else {
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "## Public Keys & Signing Material\n").unwrap();
    if report.keys.is_empty() {
        writeln!(
            out,
            "*No public GPG or SSH keys configured on this profile.*\n"
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "| Type | Key Identifier | Preview | Associated Emails | Source |"
        )
        .unwrap();
        writeln!(out, "| :--- | :--- | :--- | :--- | :--- |").unwrap();
        for k in &report.keys {
            let emails = if k.emails.is_empty() {
                "—".to_string()
            } else {
                k.emails.join(", ")
            };
            writeln!(
                out,
                "| {} | `{}` | `{}` | {} | [Source]({}) |",
                k.key_type,
                k.key_id.as_deref().unwrap_or("—"),
                k.raw_preview,
                emails,
                k.source
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }

    writeln!(out, "## Public Activity\n").unwrap();
    writeln!(out, "> {}\n", report.activity.window_note).unwrap();
    writeln!(
        out,
        "- **Total Recent Events:** {}",
        report.activity.total_events_observed
    )
    .unwrap();
    writeln!(out, "- **Push Events:** {}", report.activity.push_events).unwrap();
    writeln!(
        out,
        "- **Pull Request Events:** {}",
        report.activity.pull_request_events
    )
    .unwrap();
    writeln!(out, "- **Issue Events:** {}", report.activity.issue_events).unwrap();
    writeln!(
        out,
        "- **Release Events:** {}",
        report.activity.release_events
    )
    .unwrap();
    writeln!(out, "- **Fork Events:** {}\n", report.activity.fork_events).unwrap();

    writeln!(out, "## Inferences\n").unwrap();
    if report.inferences.is_empty() {
        writeln!(
            out,
            "*No algorithmic inferences generated for this target.*\n"
        )
        .unwrap();
    } else {
        for inf in &report.inferences {
            writeln!(out, "### {}", inf.topic).unwrap();
            writeln!(out, "- **Inference:** {}", inf.inference).unwrap();
            writeln!(out, "- **Classification:** `{}`", inf.classification).unwrap();
            writeln!(out, "- **Confidence:** {}", inf.confidence).unwrap();
            writeln!(out, "- **Empirical Basis:** {}", inf.basis).unwrap();
            writeln!(out, "- **Limitation:** *{}*\n", inf.limitation).unwrap();
        }
    }

    writeln!(out, "## Coverage Accounting\n").unwrap();
    writeln!(out, "| Metric | Value |").unwrap();
    writeln!(out, "| :--- | :--- |").unwrap();
    writeln!(
        out,
        "| Commits Scanned | {} |",
        report.coverage.commits_scanned
    )
    .unwrap();
    writeln!(
        out,
        "| Repositories Scanned | {} |",
        report.coverage.repositories_scanned
    )
    .unwrap();
    writeln!(
        out,
        "| Repositories Total | {} |",
        report.coverage.repositories_total
    )
    .unwrap();
    writeln!(
        out,
        "| Coverage State | {} |",
        if report.coverage.is_partial {
            "Partial (budget limited)"
        } else {
            "Complete"
        }
    )
    .unwrap();
    if let Some(notes) = &report.coverage.notes {
        writeln!(out, "| Notes | {} |", notes).unwrap();
    }
    writeln!(out).unwrap();

    writeln!(out, "## Provider Status\n").unwrap();
    writeln!(out, "| Provider | Status | Items | Detail |").unwrap();
    writeln!(out, "| :--- | :--- | :--- | :--- |").unwrap();
    for (name, status) in &report.provider_status {
        writeln!(
            out,
            "| {} | `{}` | {} | {} |",
            name,
            status.status,
            status
                .items_retrieved
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            status.message.as_deref().unwrap_or("—")
        )
        .unwrap();
    }
    writeln!(out).unwrap();

    writeln!(out, "## Inherent Limitations\n").unwrap();
    for lim in &report.limitations {
        writeln!(out, "- {}", lim).unwrap();
    }
    writeln!(out).unwrap();

    writeln!(out, "---\n*Spoor by Voldigoade*").unwrap();

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoor_core::{AnalysisMode, Target};

    #[test]
    fn exports_clean_markdown() {
        let report = Report::new(
            Target::User {
                username: "torvalds".into(),
            },
            AnalysisMode::Fast,
        );
        let md = export_markdown(&report).unwrap();
        assert!(md.contains("# Spoor"));
        assert!(md.contains("Spoor by Voldigoade"));
        assert!(md.contains("torvalds"));
    }
}
