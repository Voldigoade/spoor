use clap::{Parser, Subcommand};

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
    #[arg(long, short = 'm', value_name = "MODE")]
    pub mode: Option<String>,

    /// Enable DEEP mode (alias for --mode deep)
    #[arg(long, short = 'd')]
    pub deep: bool,

    /// Enable third-party enrichment (e.g. GitLeak)
    #[arg(long)]
    pub enrich: bool,

    /// Export format (text, json, markdown, pdf, svg)
    #[arg(long, short = 'f', value_name = "FORMAT")]
    pub format: Option<String>,

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
        #[arg(long, short = 'm', value_name = "MODE")]
        mode: Option<String>,

        /// Enable DEEP mode (alias for --mode deep)
        #[arg(long, short = 'd')]
        deep: bool,

        /// Enable third-party enrichment (e.g. GitLeak)
        #[arg(long)]
        enrich: bool,

        /// Export format (text, json, markdown, pdf, svg)
        #[arg(long, short = 'f', value_name = "FORMAT")]
        format: Option<String>,

        /// Destination file path
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<String>,
    },

    /// Inspect a GitHub user profile
    User {
        /// GitHub username or profile URL
        username: String,

        /// Analysis mode: fast or deep
        #[arg(long, short = 'm', value_name = "MODE")]
        mode: Option<String>,

        /// Enable DEEP mode
        #[arg(long, short = 'd')]
        deep: bool,

        /// Enable third-party enrichment (e.g. GitLeak)
        #[arg(long)]
        enrich: bool,

        /// Export format (text, json, markdown, pdf, svg)
        #[arg(long, short = 'f', value_name = "FORMAT")]
        format: Option<String>,

        /// Destination file path
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<String>,
    },

    /// Inspect a GitHub repository
    Repo {
        /// Repository in owner/repo shorthand or full GitHub URL
        repo: String,

        /// Export format (text, json, markdown, pdf, svg)
        #[arg(long, short = 'f', value_name = "FORMAT")]
        format: Option<String>,

        /// Destination file path
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<String>,
    },

    /// Convert a saved Spoor JSON report into another format
    Export {
        /// Path to input report JSON file
        file: String,

        /// Desired format: text, json, markdown, pdf, svg
        #[arg(long, short = 'f', value_name = "FORMAT")]
        format: String,

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
