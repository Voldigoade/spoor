use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Classification {
    /// Directly returned by an authoritative primary source (e.g. GitHub API response, raw Git commit).
    Observed,

    /// Calculated or aggregated strictly from observed facts (e.g. repo count, language byte sums).
    Derived,

    /// Statistical or heuristic deduction (e.g. commit timezone pattern, working hours).
    Inferred,

    /// Established partially due to pagination boundaries, request budgets, or upstream rate limits.
    Partial,

    /// Not established or ambiguous.
    Unknown,
}

impl Classification {
    pub fn as_str(&self) -> &'static str {
        match self {
            Classification::Observed => "OBSERVED",
            Classification::Derived => "DERIVED",
            Classification::Inferred => "INFERRED",
            Classification::Partial => "PARTIAL",
            Classification::Unknown => "UNKNOWN",
        }
    }
}

impl fmt::Display for Classification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
