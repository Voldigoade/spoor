use spoor_core::{Report, SpoorEvent};

use crate::format::{accent, bold, dim, green, red, yellow};

pub fn render_event(event: &SpoorEvent) {
    match event {
        SpoorEvent::LookupStarted { target, mode } => {
            println!(
                "{} {} {} {}",
                dim("[init]   "),
                bold(target),
                dim("in"),
                accent(mode)
            );
        }
        SpoorEvent::TargetResolved { target_type, .. } => {
            println!("{} target type: {}", dim("[resolve]"), accent(target_type));
        }
        SpoorEvent::ProviderStarted {
            provider,
            operation,
        } => {
            println!(
                "{} {}: {}",
                dim("[query]  "),
                bold(provider),
                dim(operation)
            );
        }
        SpoorEvent::ProviderCompleted { provider, details } => {
            println!(
                "{} {}: {}",
                green("[ok]     "),
                bold(provider),
                dim(details)
            );
        }
        SpoorEvent::ProviderWarning { provider, message } => {
            println!(
                "{} {}: {}",
                yellow("[warn]   "),
                bold(provider),
                yellow(message)
            );
        }
        SpoorEvent::ScanProgress {
            current,
            total,
            message,
        } => {
            println!(
                "{} [{}/{}] {}",
                dim("[scan]   "),
                current,
                total,
                dim(message)
            );
        }
        SpoorEvent::AnalysisCompleted { duration_ms } => {
            let secs = *duration_ms as f64 / 1000.0;
            println!("{} completed in {:.2}s\n", green("[done]   "), secs);
        }
        SpoorEvent::LookupFailed { error } => {
            println!("{} {}", red("[fail]   "), bold(error));
        }
    }
}

pub fn render_terminal_report(report: &Report) {
    println!(
        "{}",
        bold(format!("TARGET: {}", report.target.display_name()))
    );
    println!(
        "{}",
        dim(format!("Canonical URL: {}", report.target.canonical_url()))
    );
    println!(
        "{} · Generated: {}",
        accent(format!("Mode: {}", report.mode)),
        dim(report
            .generated_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string())
    );
    println!();

    println!(
        "{}",
        bold("── SUMMARY ─────────────────────────────────────────────────────────────")
    );
    println!(
        "  {:<24} {}",
        dim("Display Name:"),
        report.summary.display_name
    );
    println!(
        "  {:<24} {}",
        dim("Target Type:"),
        report.summary.target_type
    );
    if let Some(created) = report.summary.account_created {
        println!(
            "  {:<24} {}",
            dim("Account Created:"),
            created.format("%Y-%m-%d %H:%M:%S UTC")
        );
    }
    println!(
        "  {:<24} {}",
        dim("Repositories:"),
        report.summary.total_public_repositories
    );
    println!(
        "  {:<24} ★ {}",
        dim("Total Stars:"),
        report.summary.total_stars_accumulated
    );
    println!(
        "  {:<24} ⑂ {}",
        dim("Total Forks:"),
        report.summary.total_forks_accumulated
    );
    println!(
        "  {:<24} {}",
        dim("Distinct Emails:"),
        report.summary.distinct_emails_observed
    );
    println!(
        "  {:<24} {}",
        dim("Public Keys:"),
        report.summary.public_keys_found
    );
    if !report.summary.primary_languages.is_empty() {
        let langs_str = report
            .summary
            .primary_languages
            .iter()
            .take(5)
            .map(|(l, c)| format!("{l} ({c})"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {:<24} {}", dim("Top Languages:"), langs_str);
    }
    println!();

    if let Some(prof) = &report.profile {
        println!(
            "{}",
            bold("── PUBLIC PROFILE ──────────────────────────────────────────────────────")
        );
        println!(
            "  {:<24} {} (id: {})",
            dim("Identity:"),
            prof.login,
            prof.id
        );
        if let Some(name) = &prof.name {
            println!("  {:<24} {}", dim("Display Name:"), name);
        }
        if let Some(bio) = &prof.bio {
            println!("  {:<24} {}", dim("Bio:"), bio.replace('\n', " "));
        }
        if let Some(company) = &prof.company {
            println!("  {:<24} {}", dim("Company:"), company);
        }
        if let Some(loc) = &prof.location {
            println!("  {:<24} {}", dim("Location:"), loc);
        }
        if let Some(email) = &prof.public_email {
            println!("  {:<24} {}", dim("Public Email:"), accent(email));
        }
        if let Some(web) = &prof.blog_url {
            if !web.is_empty() {
                println!("  {:<24} {}", dim("Website:"), web);
            }
        }
        println!(
            "  {:<24} {} followers · {} following",
            dim("Network:"),
            prof.followers,
            prof.following
        );
        println!();
    }

    println!(
        "{}",
        bold("── PUBLIC IDENTITIES (COMMIT HEADERS & PROFILE) ─────────────────────────")
    );
    if report.identities.is_empty() {
        println!("  {}", dim("No public email identities observed."));
    } else {
        for (i, id) in report.identities.iter().enumerate().take(10) {
            let name_str = id.name.as_deref().unwrap_or("—");
            let verified_badge = if id.is_verified_signer {
                green(" [VERIFIED GPG SIGNER]")
            } else {
                String::new()
            };
            let type_badge = if id.email_type.is_platform_system() {
                dim("[PLATFORM / BOT]")
            } else {
                dim(format!("({})", id.email_type.as_str()))
            };
            println!(
                "  [{}] {} {}{}",
                i + 1,
                bold(&id.email),
                type_badge,
                verified_badge
            );
            let repos_info = if !id.repositories.is_empty() {
                format!(" · Repos: {}", id.repositories.join(", "))
            } else {
                String::new()
            };
            println!(
                "      Committer: {} · Commits: {} · Sources: {}{}",
                name_str,
                id.commits_observed,
                dim(id.sources.join(", ")),
                dim(repos_info)
            );
        }
        if report.identities.len() > 10 {
            println!(
                "  {}",
                dim(format!(
                    "... and {} additional identities",
                    report.identities.len() - 10
                ))
            );
        }
    }
    println!();

    if !report.related_commit_actors.is_empty() {
        println!(
            "{}",
            bold("── RELATED COMMIT ACTORS (REPOSITORY CONTRIBUTORS & CO-AUTHORS) ────────")
        );
        for (i, actor) in report.related_commit_actors.iter().enumerate().take(10) {
            let name_str = actor.name.as_deref().unwrap_or("—");
            println!(
                "  [{}] {} {}",
                i + 1,
                dim(&actor.email),
                dim(format!("({})", actor.relationship))
            );
            let repos_info = if !actor.repositories.is_empty() {
                format!(" · Repos: {}", actor.repositories.join(", "))
            } else {
                String::new()
            };
            println!(
                "      Actor: {} · Commits: {} · Sources: {}{}",
                name_str,
                actor.commits_observed,
                dim(actor.sources.join(", ")),
                dim(repos_info)
            );
        }
        if report.related_commit_actors.len() > 10 {
            println!(
                "  {}",
                dim(format!(
                    "... and {} additional related commit actors",
                    report.related_commit_actors.len() - 10
                ))
            );
        }
        println!();
    }

    println!(
        "{}",
        bold("── REPOSITORIES (SAMPLE) ────────────────────────────────────────────────")
    );
    for r in report.repositories.iter().take(8) {
        let flag = if r.is_archived {
            yellow("[archived]")
        } else if r.is_fork {
            dim("[fork]")
        } else {
            String::new()
        };
        println!(
            "  * {:<32} ★ {:<5} ⑂ {:<4} {:<12} {}",
            accent(&r.name),
            r.stars,
            r.forks,
            dim(r.primary_language.as_deref().unwrap_or("—")),
            flag
        );
    }
    if report.repositories.len() > 8 {
        println!(
            "  {}",
            dim(format!(
                "... and {} more repositories",
                report.repositories.len() - 8
            ))
        );
    }
    println!();

    if !report.keys.is_empty() {
        println!(
            "{}",
            bold("── PUBLIC KEYS ─────────────────────────────────────────────────────────")
        );
        for k in &report.keys {
            println!(
                "  [{}] {:<16} {}",
                accent(k.key_type.to_string()),
                dim(k.key_id.as_deref().unwrap_or("—")),
                k.raw_preview
            );
        }
        println!();
    }

    if !report.inferences.is_empty() {
        println!(
            "{}",
            bold("── INFERENCES [ALGORITHMIC DEDUCTIONS] ─────────────────────────────────")
        );
        for inf in &report.inferences {
            println!("  * {}: {}", bold(&inf.topic), inf.inference);
            println!(
                "    {}",
                dim(format!(
                    "Confidence: {} · Basis: {}",
                    inf.confidence, inf.basis
                ))
            );
            println!("    {}", yellow(format!("Limitation: {}", inf.limitation)));
        }
        println!();
    }

    println!(
        "{}",
        bold("── COVERAGE & STATUS ───────────────────────────────────────────────────")
    );
    let cov_state = if report.coverage.is_partial {
        yellow("PARTIAL")
    } else {
        green("COMPLETE")
    };
    if report.coverage.owned_repositories_total > 0
        || report.coverage.external_repositories_discovered > 0
    {
        println!(
            "  Coverage: {} ({} commits, Owned: {}/{}, External: {}/{})",
            cov_state,
            report.coverage.commits_scanned,
            report.coverage.owned_repositories_scanned,
            report.coverage.owned_repositories_total,
            report.coverage.external_repositories_scanned,
            report.coverage.external_repositories_discovered
        );
    } else {
        println!(
            "  Coverage: {} ({} commits, {}/{} repos)",
            cov_state,
            report.coverage.commits_scanned,
            report.coverage.repositories_scanned,
            report.coverage.repositories_total
        );
    }
    if let Some(notes) = &report.coverage.notes {
        println!("  {}", dim(format!("Notes: {}", notes)));
    }

    for (p, st) in &report.provider_status {
        let st_badge = match st.status {
            spoor_core::ProviderHealth::Ok => green("OK"),
            spoor_core::ProviderHealth::Cached => accent("CACHED"),
            spoor_core::ProviderHealth::Partial => yellow("PARTIAL"),
            spoor_core::ProviderHealth::RateLimited => red("RATE_LIMITED"),
            spoor_core::ProviderHealth::Unavailable => red("UNAVAILABLE"),
            spoor_core::ProviderHealth::Skipped => dim("SKIPPED"),
        };
        println!(
            "  {:<24} {:<16} {}",
            p,
            st_badge,
            dim(st.message.as_deref().unwrap_or(""))
        );
    }

    println!();
    println!(
        "{}",
        dim("────────────────────────────────────────────────────────────────────────")
    );
    println!(
        "{}",
        dim("Spoor by Voldigoade · Public metadata exploration")
    );
}
