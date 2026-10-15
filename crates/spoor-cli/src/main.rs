use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use spoor_core::{
    AnalysisMode, CacheStore, Report, SpoorError, SpoorEvent, SqliteCacheStore, Target,
};
use spoor_export::{export_bytes, ExportFormat};
use spoor_providers::{AnalysisEngine, GitHubProvider, GitLeakClient, HttpClient};
use tokio::sync::mpsc;

mod args;
mod format;
mod render;
mod repl;

use args::{CacheAction, Cli, Commands};
use format::{bold, green, init_color, red, yellow};
use render::{render_event, render_terminal_report};
use repl::run_repl;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_color(cli.no_color);

    // Initialize tracing only if RUST_LOG is explicitly configured
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt::init();
    }

    let cache_path = SqliteCacheStore::default_path();
    let cache: Arc<dyn CacheStore> = match SqliteCacheStore::new(&cache_path) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            eprintln!(
                "{}: {}",
                yellow("[warn] Cache initialization failed, running in memory-only"),
                e
            );
            Arc::new(spoor_core::NoopCacheStore)
        }
    };

    let http_client = HttpClient::new();
    let github_provider = GitHubProvider::from_env(http_client.clone());
    let gitleak_client = GitLeakClient::new(http_client, None);

    let engine = Arc::new(AnalysisEngine::new(
        github_provider,
        gitleak_client,
        cache.clone(),
    ));

    match cli.command {
        None => {
            run_repl(engine, cache).await;
        }
        Some(Commands::Inspect {
            target,
            deep,
            format,
            output,
        }) => {
            let mode = if deep {
                AnalysisMode::Deep
            } else {
                AnalysisMode::Fast
            };
            if let Err(e) = run_inspect(engine, &target, mode, format, output).await {
                eprintln!("{}: {}", red("[error]"), e);
                std::process::exit(1);
            }
        }
        Some(Commands::User {
            username,
            deep,
            format,
            output,
        }) => {
            let mode = if deep {
                AnalysisMode::Deep
            } else {
                AnalysisMode::Fast
            };
            if let Err(e) = run_inspect(engine, &username, mode, format, output).await {
                eprintln!("{}: {}", red("[error]"), e);
                std::process::exit(1);
            }
        }
        Some(Commands::Repo {
            repo,
            format,
            output,
        }) => {
            if let Err(e) = run_inspect(engine, &repo, AnalysisMode::Fast, format, output).await {
                eprintln!("{}: {}", red("[error]"), e);
                std::process::exit(1);
            }
        }
        Some(Commands::Export {
            file,
            format,
            output,
        }) => {
            if let Err(e) = run_export_file(&file, &format, output) {
                eprintln!("{}: {}", red("[error]"), e);
                std::process::exit(1);
            }
        }
        Some(Commands::Providers) => {
            run_providers_status();
        }
        Some(Commands::Cache { action }) => match action.unwrap_or(CacheAction::Stats) {
            CacheAction::Stats => match cache.stats().await {
                Ok(stats) => {
                    println!("{}", bold("Spoor Local Cache:"));
                    println!("  Path:            {}", stats.database_path);
                    println!("  Active entries:  {}", stats.active_entries);
                    println!("  Expired entries: {}", stats.expired_entries);
                    println!("  Total entries:   {}", stats.total_entries);
                    println!("  Payload bytes:   {} bytes", stats.total_bytes);
                }
                Err(e) => {
                    eprintln!("{}: {}", red("[error]"), e);
                    std::process::exit(1);
                }
            },
            CacheAction::Clear => match cache.clear().await {
                Ok(count) => {
                    println!("{} Cleared {} cached records.", green("[ok]"), count);
                }
                Err(e) => {
                    eprintln!("{}: {}", red("[error]"), e);
                    std::process::exit(1);
                }
            },
        },
        Some(Commands::Serve {
            bind,
            port,
            trusted_proxies,
        }) => {
            let addr_str = format!("{bind}:{port}");
            let addr: SocketAddr = match addr_str.parse() {
                Ok(a) => a,
                Err(e) => {
                    eprintln!(
                        "{}: Invalid socket address '{addr_str}': {e}",
                        red("[error]")
                    );
                    std::process::exit(1);
                }
            };

            let proxy_config = if let Some(raw) = trusted_proxies {
                match spoor_server::TrustedProxyConfig::parse(&raw) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        eprintln!("{}: {}", red("[error]"), e);
                        std::process::exit(1);
                    }
                }
            } else {
                std::env::var("SPOOR_TRUSTED_PROXIES")
                    .ok()
                    .and_then(|raw| spoor_server::TrustedProxyConfig::parse(&raw).ok())
                    .unwrap_or_default()
            };

            let state = spoor_server::AppState::with_proxy_config(engine, proxy_config);
            println!(
                "{}",
                bold(format!("Starting Spoor HTTP service on http://{addr}"))
            );
            println!("  * Health check:    http://{addr}/healthz");
            println!("  * Lookups API:     http://{addr}/v1/lookups");
            println!("  * Progress stream: http://{addr}/v1/lookups/:id/events");
            if state.proxy_config.is_enabled() {
                println!("  * Trusted Proxies: Configured (forwarding headers honored for verified proxies)");
            } else {
                println!("  * Trusted Proxies: Disabled (direct peer socket IP used exclusively)");
            }
            println!("Press Ctrl+C to terminate gracefully.\n");

            if let Err(e) = spoor_server::run_server(addr, state).await {
                eprintln!("{}: {}", red("[error]"), e);
                std::process::exit(1);
            }
        }
    }
}

async fn run_inspect(
    engine: Arc<AnalysisEngine>,
    raw_target: &str,
    mode: AnalysisMode,
    format_arg: Option<String>,
    output_arg: Option<String>,
) -> Result<(), SpoorError> {
    let target = Target::parse(raw_target)?;

    let is_export_to_stdout = format_arg.is_some() && output_arg.is_none();

    let (tx, mut rx) = mpsc::channel::<SpoorEvent>(64);
    let event_printer = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if !is_export_to_stdout {
                render_event(&ev);
            }
        }
    });

    let report = engine.run_analysis(&target, mode, Some(tx)).await?;
    let _ = event_printer.await;

    if let Some(fmt_str) = format_arg {
        let fmt = ExportFormat::from_str_loose(&fmt_str)?;
        let bytes = export_bytes(&report, fmt)?;

        if let Some(dest_path) = output_arg {
            std::fs::write(&dest_path, bytes).map_err(|e| {
                SpoorError::Export(format!("Failed to write export to '{dest_path}': {e}"))
            })?;
            println!("{} Export saved to {}", green("[ok]"), bold(dest_path));
        } else {
            match String::from_utf8(bytes) {
                Ok(s) => print!("{s}"),
                Err(_) => {
                    return Err(SpoorError::Export(
                        "Binary export format (such as PDF) requires --output <file>".into(),
                    ));
                }
            }
        }
    } else {
        render_terminal_report(&report);
    }

    Ok(())
}

fn run_export_file(
    file_path: &str,
    format_str: &str,
    output_path: Option<String>,
) -> Result<(), SpoorError> {
    let content = std::fs::read_to_string(file_path).map_err(|e| {
        SpoorError::Export(format!("Failed to read report file '{file_path}': {e}"))
    })?;

    let report: Report = serde_json::from_str(&content)
        .map_err(|e| SpoorError::Export(format!("Invalid report JSON in '{file_path}': {e}")))?;

    let fmt = ExportFormat::from_str_loose(format_str)?;
    let bytes = export_bytes(&report, fmt)?;

    let dest = output_path.unwrap_or_else(|| {
        format!(
            "{}.{}",
            report
                .target
                .safe_filename_stem(report.mode, &report.generated_at),
            fmt.file_extension()
        )
    });

    std::fs::write(&dest, bytes)
        .map_err(|e| SpoorError::Export(format!("Failed to write export to '{dest}': {e}")))?;

    println!("{} Export written to {}", green("[ok]"), bold(dest));
    Ok(())
}

fn run_providers_status() {
    let token_status = if std::env::var("GITHUB_TOKEN").is_ok() {
        green("Configured (5,000 req/hr)")
    } else {
        yellow("Not configured (Anonymous REST degradation: 60 req/hr)")
    };

    println!("{}", bold("Spoor Configured Providers:"));
    println!("  * GitHub REST API:    Active");
    println!("  * GitHub GraphQL:     Active");
    println!("  * GitHub Auth Token:  {token_status}");
    println!("  * GitLeak API:        Active (50 req/hr quota, DEEP mode only)");
    println!("  * Public Keys:        Active (GPG & SSH)\n");
}
