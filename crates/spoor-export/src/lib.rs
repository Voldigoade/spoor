pub mod json;
pub mod markdown;
pub mod pdf;
pub mod svg;
pub mod text;

use serde::{Deserialize, Serialize};
use spoor_core::{Report, SpoorError};

pub use json::export_json;
pub use markdown::export_markdown;
pub use pdf::export_pdf;
pub use svg::export_svg;
pub use text::export_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Md,
    Txt,
    Svg,
    Pdf,
}

impl ExportFormat {
    pub fn from_str_loose(s: &str) -> Result<Self, SpoorError> {
        match s.trim().to_lowercase().as_str() {
            "json" => Ok(ExportFormat::Json),
            "md" | "markdown" => Ok(ExportFormat::Md),
            "txt" | "text" => Ok(ExportFormat::Txt),
            "svg" => Ok(ExportFormat::Svg),
            "pdf" => Ok(ExportFormat::Pdf),
            other => Err(SpoorError::Export(format!(
                "Unsupported export format '{other}'. Choose from: json, md, txt, svg, pdf"
            ))),
        }
    }

    pub fn file_extension(&self) -> &'static str {
        match self {
            ExportFormat::Json => "json",
            ExportFormat::Md => "md",
            ExportFormat::Txt => "txt",
            ExportFormat::Svg => "svg",
            ExportFormat::Pdf => "pdf",
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            ExportFormat::Json => "application/json",
            ExportFormat::Md => "text/markdown; charset=utf-8",
            ExportFormat::Txt => "text/plain; charset=utf-8",
            ExportFormat::Svg => "image/svg+xml",
            ExportFormat::Pdf => "application/pdf",
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.file_extension())
    }
}

pub fn export_bytes(report: &Report, format: ExportFormat) -> Result<Vec<u8>, SpoorError> {
    match format {
        ExportFormat::Json => export_json(report).map(|s| s.into_bytes()),
        ExportFormat::Md => export_markdown(report).map(|s| s.into_bytes()),
        ExportFormat::Txt => export_text(report).map(|s| s.into_bytes()),
        ExportFormat::Svg => export_svg(report).map(|s| s.into_bytes()),
        ExportFormat::Pdf => export_pdf(report),
    }
}

pub fn export_string(report: &Report, format: ExportFormat) -> Result<String, SpoorError> {
    match format {
        ExportFormat::Json => export_json(report),
        ExportFormat::Md => export_markdown(report),
        ExportFormat::Txt => export_text(report),
        ExportFormat::Svg => export_svg(report),
        ExportFormat::Pdf => Err(SpoorError::Export(
            "PDF is a binary format and cannot be rendered as a UTF-8 string".into(),
        )),
    }
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
