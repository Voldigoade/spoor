use spoor_core::{Report, SpoorError};

pub fn export_json(report: &Report) -> Result<String, SpoorError> {
    serde_json::to_string_pretty(report)
        .map_err(|e| SpoorError::Export(format!("JSON export serialization failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoor_core::{AnalysisMode, Target};

    #[test]
    fn exports_valid_json() {
        let report = Report::new(
            Target::User {
                username: "torvalds".into(),
            },
            AnalysisMode::Fast,
        );
        let json = export_json(&report).unwrap();
        assert!(json.contains(r#""schema_version": "1.0""#));
        assert!(json.contains(r#""author": "Voldigoade""#));
        assert!(json.contains(r#""username": "torvalds""#));
    }
}
