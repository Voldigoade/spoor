use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "spoor",
    version,
    author = "Voldigoade",
    about = "Terminal-first GitHub public-footprint inspection tool",
    long_about = "Spoor inspects publicly accessible GitHub metadata, commit headers, and public keys to construct sourced, structured reports with complete provenance."
)]
pub struct Cli {
    #[arg(long, global = true, help = "Disable ANSI color formatting")]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inspect a GitHub target (username, profile URL, repo shorthand, or repo URL)
    Inspect {
        /// Target identifier or URL
        target: String,

        /// Enable DEEP mode (wider commit sampling and GitLeak enrichment)
        #[arg(long, short = 'd')]
        deep: bool,

        /// Export format (json, md, txt, svg, pdf)
        #[arg(long, short = 'f')]
        format: Option<String>,

        /// Destination file path
        #[arg(long, short = 'o')]
        output: Option<String>,
    },

    /// Inspect a GitHub user profile
    User {
        /// GitHub username or profile URL
        username: String,

        /// Enable DEEP mode
        #[arg(long, short = 'd')]
        deep: bool,

        /// Export format (json, md, txt, svg, pdf)
        #[arg(long, short = 'f')]
        format: Option<String>,

        /// Destination file path
        #[arg(long, short = 'o')]
        output: Option<String>,
    },

    /// Inspect a GitHub repository
    Repo {
        /// Repository in owner/repo shorthand or full GitHub URL
        repo: String,

        /// Export format (json, md, txt, svg, pdf)
        #[arg(long, short = 'f')]
        format: Option<String>,

        /// Destination file path
        #[arg(long, short = 'o')]
        output: Option<String>,
    },

    /// Convert a saved Spoor JSON report into another format
    Export {
        /// Path to input report JSON file
        file: String,

        /// Desired format: json, md, txt, svg, pdf
        #[arg(long, short = 'f')]
        format: String,

        /// Output file path (defaults to auto-generated filename)
        #[arg(long, short = 'o')]
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
