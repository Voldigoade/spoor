use std::io::{self, Write};
use std::sync::Arc;

use spoor_core::{AnalysisMode, CacheStore, Report, SpoorEvent, Target};
use spoor_export::{export_bytes, ExportFormat};
use spoor_providers::AnalysisEngine;
use tokio::sync::mpsc;

use crate::format::{accent, bold, dim, green, red, yellow};
use crate::render::{render_event, render_terminal_report};

pub async fn run_repl(engine: Arc<AnalysisEngine>, cache: Arc<dyn CacheStore>) {
    println!("{}", bold(format!("spoor {}", spoor_core::version())));
    println!("{}", dim("public GitHub footprint explorer"));
    println!("{}", dim("by Voldigoade"));
    println!(
        "{}",
        dim("Type 'help' for commands, 'exit' or 'quit' to exit.\n")
    );

    let mut last_report: Option<Report> = None;

    loop {
        print!("{} ", accent("spoor>"));
        let _ = io::stdout().flush();

        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("{}: {}", red("Input error"), e);
                break;
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();

        match cmd.as_str() {
            "exit" | "quit" | "q" => {
                println!("{}", dim("Exiting Spoor."));
                break;
            }
            "help" => {
                print_repl_help();
            }
            "about" => {
                println!("{}", bold("Spoor"));
                println!("Terminal-first GitHub public-footprint inspection tool.");
                println!("Public attribution: Spoor by Voldigoade");
                println!("Repository: https://github.com/Voldigoade/spoor\n");
            }
            "clear" | "cls" => {
                // Portable ANSI clear screen
                print!("\x1b[2J\x1b[1;1H");
                let _ = io::stdout().flush();
            }
            "providers" => {
                let token_status = if std::env::var("GITHUB_TOKEN").is_ok() {
                    green("Configured (5,000 req/hr)")
                } else {
                    yellow("Not configured (Anonymous, 60 req/hr)")
                };
                println!("{}", bold("Upstream Providers:"));
                println!("  * GitHub REST API:    Active");
                println!("  * GitHub GraphQL:     Active");
                println!("  * GitHub Auth Token:  {token_status}");
                println!("  * GitLeak API:        Active (50 req/hr quota, DEEP mode only)\n");
            }
            "cache" => {
                let sub = parts.get(1).map(|s| s.to_lowercase());
                match sub.as_deref() {
                    Some("clear") => match cache.clear().await {
                        Ok(n) => println!("{} Cleared {} cache records.\n", green("[ok]"), n),
                        Err(e) => eprintln!("{}: {}\n", red("[err]"), e),
                    },
                    _ => match cache.stats().await {
                        Ok(stats) => {
                            println!("{}", bold("Cache Statistics:"));
                            println!("  Active entries:  {}", stats.active_entries);
                            println!("  Expired entries: {}", stats.expired_entries);
                            println!("  Total entries:   {}", stats.total_entries);
                            println!("  Payload bytes:   {} bytes", stats.total_bytes);
                            println!("  Database path:   {}\n", stats.database_path);
                        }
                        Err(e) => eprintln!("{}: {}\n", red("[err]"), e),
                    },
                }
            }
            "inspect" | "user" | "repo" => {
                if parts.len() < 2 {
                    println!(
                        "{}: Target required. Usage: inspect <username|repo|url> [--deep]\n",
                        yellow("Usage")
                    );
                    continue;
                }
                let target_arg = parts[1];
                let deep = parts.iter().any(|&p| p == "--deep" || p == "-d");
                let mode = if deep {
                    AnalysisMode::Deep
                } else {
                    AnalysisMode::Fast
                };

                match Target::parse(target_arg) {
                    Ok(target) => {
                        println!();
                        let (tx, mut rx) = mpsc::channel::<SpoorEvent>(64);
                        let event_printer = tokio::spawn(async move {
                            while let Some(ev) = rx.recv().await {
                                render_event(&ev);
                            }
                        });

                        match engine.run_analysis(&target, mode, Some(tx)).await {
                            Ok(report) => {
                                let _ = event_printer.await;
                                render_terminal_report(&report);
                                println!();
                                last_report = Some(report);
                            }
                            Err(e) => {
                                let _ = event_printer.await;
                                eprintln!("\n{}: {}\n", red("Analysis error"), e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("{}: {}\n", red("Invalid target"), e);
                    }
                }
            }
            "export" => {
                if last_report.is_none() {
                    println!("{}: No report has been generated yet in this session. Run 'inspect <target>' first.\n", yellow("Notice"));
                    continue;
                }
                if parts.len() < 2 {
                    println!(
                        "{}: Format required. Usage: export <json|md|txt|svg|pdf> [filename]\n",
                        yellow("Usage")
                    );
                    continue;
                }
                let fmt_str = parts[1];
                match ExportFormat::from_str_loose(fmt_str) {
                    Ok(fmt) => {
                        let report = last_report.as_ref().unwrap();
                        let default_filename = format!(
                            "{}.{}",
                            report
                                .target
                                .safe_filename_stem(report.mode, &report.generated_at),
                            fmt.file_extension()
                        );
                        let filename = parts.get(2).copied().unwrap_or(&default_filename);

                        match export_bytes(report, fmt) {
                            Ok(bytes) => match std::fs::write(filename, bytes) {
                                Ok(_) => println!(
                                    "{} Exported report to {}\n",
                                    green("[ok]"),
                                    bold(filename)
                                ),
                                Err(e) => eprintln!("{}: {}\n", red("Write error"), e),
                            },
                            Err(e) => eprintln!("{}: {}\n", red("Export error"), e),
                        }
                    }
                    Err(e) => eprintln!("{}: {}\n", red("Format error"), e),
                }
            }
            // Strict defense against shell execution
            "rm" | "curl" | "wget" | "bash" | "sh" | "zsh" | "powershell" | "cmd" | "ssh"
            | "cat" | "ls" | "dir" => {
                println!(
                    "{}: Spoor is a dedicated inspection tool, not a system shell. System commands are disabled.\n",
                    yellow("Prohibited")
                );
            }
            other => {
                println!(
                    "{}: Unknown command '{}'. Type 'help' for available commands.\n",
                    yellow("Notice"),
                    other
                );
            }
        }
    }
}

fn print_repl_help() {
    println!("{}", bold("Available Commands:"));
    println!("  inspect <target> [--deep]   Inspect user, profile URL, repo shorthand, or URL");
    println!("  user <username> [--deep]    Inspect a GitHub user profile");
    println!("  repo <owner/repo>           Inspect a GitHub repository");
    println!(
        "  export <format> [filename]  Export last generated report (json, md, txt, svg, pdf)"
    );
    println!("  providers                   Display provider status and token rate limits");
    println!("  cache [stats|clear]         Inspect or clear local SQLite cache");
    println!("  clear                       Clear terminal screen");
    println!("  about                       Show project information");
    println!("  help                        Display this command manual");
    println!("  quit, exit                  Exit interactive mode\n");
}
