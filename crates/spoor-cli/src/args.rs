use clap::{Parser, Subcommand, ValueEnum};

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "lower")]
pub enum CliMode {
    Fast,
    Deep,
}

impl std::fmt::Display for CliMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliMode::Fast => write!(f, "fast"),
            CliMode::Deep => write!(f, "deep"),
        }
    }
}

impl From<CliMode> for spoor_core::AnalysisMode {
    fn from(m: CliMode) -> Self {
        match m {
            CliMode::Fast => spoor_core::AnalysisMode::Fast,
            CliMode::Deep => spoor_core::AnalysisMode::Deep,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "lower")]
pub enum CliExportFormat {
    Text,
    Json,
    Markdown,
    Pdf,
    Svg,
}

impl std::fmt::Display for CliExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliExportFormat::Text => write!(f, "text"),
            CliExportFormat::Json => write!(f, "json"),
            CliExportFormat::Markdown => write!(f, "markdown"),
            CliExportFormat::Pdf => write!(f, "pdf"),
            CliExportFormat::Svg => write!(f, "svg"),
        }
    }
}

impl From<CliExportFormat> for spoor_export::ExportFormat {
    fn from(f: CliExportFormat) -> Self {
        match f {
            CliExportFormat::Text => spoor_export::ExportFormat::Txt,
            CliExportFormat::Json => spoor_export::ExportFormat::Json,
            CliExportFormat::Markdown => spoor_export::ExportFormat::Md,
            CliExportFormat::Pdf => spoor_export::ExportFormat::Pdf,
            CliExportFormat::Svg => spoor_export::ExportFormat::Svg,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "spoor",
    version,
    author = "Voldigoade",
    about = "Terminal-first GitHub public-footprint inspection tool",
    long_about = "Spoor inspects publicly accessible GitHub metadata, commit headers, public keys, and repository history to construct sourced, structured reports with complete provenance.",
    after_help = "EXAMPLES:\n    spoor torvalds\n    spoor inspect Voldigoade --mode deep --format markdown -o report.md\n    spoor inspect rust-lang/rust\n    spoor serve --bind 127.0.0.1 --port 8080"
)]
pub struct Cli {
    #[arg(long, global = true, help = "Disable ANSI color formatting")]
    pub no_color: bool,

    /// Target to inspect directly (e.g. `spoor torvalds`)
    #[arg(value_name = "TARGET")]
    pub target: Option<String>,

    /// Analysis mode: fast (bounded API scan) or deep (commit discovery, bare git clone history)
    #[arg(long, short = 'm', value_name = "MODE", value_enum)]
    pub mode: Option<CliMode>,

    /// Enable DEEP mode (alias for --mode deep)
    #[arg(long, short = 'd')]
    pub deep: bool,

    /// Enable third-party enrichment (e.g. GitLeak)
    #[arg(long)]
    pub enrich: bool,

    /// Export format (text, json, markdown, pdf, svg)
    #[arg(long, short = 'f', value_name = "FORMAT", value_enum)]
    pub format: Option<CliExportFormat>,

    /// Output destination file path
    #[arg(long, short = 'o', value_name = "PATH")]
    pub output: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inspect a GitHub target (username, profile URL, repo shorthand, or repo URL)
    Inspect {
        /// Target identifier or URL (e.g. torvalds, rust-lang/rust, https://github.com/torvalds)
        target: String,

        /// Analysis mode: fast or deep
        #[arg(long, short = 'm', value_name = "MODE", value_enum)]
        mode: Option<CliMode>,

        /// Enable DEEP mode (alias for --mode deep)
        #[arg(long, short = 'd')]
        deep: bool,

        /// Enable third-party enrichment (e.g. GitLeak)
        #[arg(long)]
        enrich: bool,

        /// Export format (text, json, markdown, pdf, svg)
        #[arg(long, short = 'f', value_name = "FORMAT", value_enum)]
        format: Option<CliExportFormat>,

        /// Destination file path
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<String>,
    },

    /// Inspect a GitHub user profile
    User {
        /// GitHub username or profile URL
        username: String,

        /// Analysis mode: fast or deep
        #[arg(long, short = 'm', value_name = "MODE", value_enum)]
        mode: Option<CliMode>,

        /// Enable DEEP mode
        #[arg(long, short = 'd')]
        deep: bool,

        /// Enable third-party enrichment (e.g. GitLeak)
        #[arg(long)]
        enrich: bool,

        /// Export format (text, json, markdown, pdf, svg)
        #[arg(long, short = 'f', value_name = "FORMAT", value_enum)]
        format: Option<CliExportFormat>,

        /// Destination file path
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<String>,
    },

    /// Inspect a GitHub repository
    Repo {
        /// Repository in owner/repo shorthand or full GitHub URL
        repo: String,

        /// Export format (text, json, markdown, pdf, svg)
        #[arg(long, short = 'f', value_name = "FORMAT", value_enum)]
        format: Option<CliExportFormat>,

        /// Destination file path
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<String>,
    },

    /// Convert a saved Spoor JSON report into another format
    Export {
        /// Path to input report JSON file
        file: String,

        /// Desired format: text, json, markdown, pdf, svg
        #[arg(long, short = 'f', value_name = "FORMAT", value_enum)]
        format: CliExportFormat,

        /// Output file path (defaults to auto-generated filename)
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<String>,
    },

    /// Show configured upstream providers and authentication status
    Providers,

    /// View cache statistics or clear local SQLite cache
    Cache {
        #[command(subcommand)]
        action: Option<CacheAction>,
    },

    /// Start Spoor HTTP API server with SSE progress streaming
    Serve {
        /// Listen address
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,

        /// Listen port
        #[arg(long, short = 'p', default_value = "8080")]
        port: u16,

        /// Comma-separated list of trusted proxy IPs or CIDRs (e.g. 127.0.0.1,10.0.0.0/8).
        /// If omitted, defaults to SPOOR_TRUSTED_PROXIES environment variable, or empty (untrusted).
        #[arg(long, env = "SPOOR_TRUSTED_PROXIES")]
        trusted_proxies: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum CacheAction {
    /// Show cache statistics
    Stats,
    /// Purge all records from local cache
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn test_help_output_contains_expected_flags_and_enums() {
        let help = Cli::command().render_help().to_string();
        assert!(help.contains("-m, --mode <MODE>"));
        assert!(help.contains("[possible values: fast, deep]"));
        assert!(help.contains("-f, --format <FORMAT>"));
        assert!(help.contains("[possible values: text, json, markdown, pdf, svg]"));
        assert!(help.contains("--enrich"));
        assert!(help.contains("-d, --deep"));
    }

    #[test]
    fn test_inspect_help_output_contains_expected_flags_and_enums() {
        let mut cmd = Cli::command();
        let inspect_cmd = cmd
            .find_subcommand_mut("inspect")
            .expect("inspect subcommand");
        let help = inspect_cmd.render_help().to_string();
        assert!(help.contains("-m, --mode <MODE>"));
        assert!(help.contains("[possible values: fast, deep]"));
        assert!(help.contains("-f, --format <FORMAT>"));
        assert!(help.contains("[possible values: text, json, markdown, pdf, svg]"));
        assert!(help.contains("--enrich"));
        assert!(help.contains("-d, --deep"));
    }

    #[test]
    fn test_export_help_output_contains_expected_flags_and_enums() {
        let mut cmd = Cli::command();
        let export_cmd = cmd
            .find_subcommand_mut("export")
            .expect("export subcommand");
        let help = export_cmd.render_help().to_string();
        assert!(help.contains("-f, --format <FORMAT>"));
        assert!(help.contains("[possible values: text, json, markdown, pdf, svg]"));
    }

    #[test]
    fn test_parse_valid_mode_and_enrich() {
        let cli = Cli::try_parse_from(["spoor", "torvalds", "--mode", "deep", "--enrich"])
            .expect("parse valid args");
        assert_eq!(cli.target, Some("torvalds".to_string()));
        assert_eq!(cli.mode, Some(CliMode::Deep));
        assert!(cli.enrich);
        assert!(!cli.deep);

        let cli2 =
            Cli::try_parse_from(["spoor", "torvalds", "-d"]).expect("parse valid short deep");
        assert!(cli2.deep);
        assert!(!cli2.enrich);
    }

    #[test]
    fn test_parse_invalid_mode_rejected() {
        let err = Cli::try_parse_from(["spoor", "torvalds", "--mode", "unsupported"])
            .expect_err("should reject unsupported mode");
        let err_msg = err.to_string();
        assert!(err_msg.contains("[possible values: fast, deep]"));
    }

    #[test]
    fn test_parse_valid_export_format() {
        let cli = Cli::try_parse_from(["spoor", "export", "report.json", "-f", "pdf"])
            .expect("parse export format");
        match cli.command {
            Some(Commands::Export { format, .. }) => {
                assert_eq!(format, CliExportFormat::Pdf);
            }
            _ => panic!("Expected export command"),
        }
    }
}
