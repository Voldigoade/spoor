use spoor_core::RepoHygiene;

/// Inspects a collection of repository file paths to accurately determine
/// hygiene, governance, and CI configurations.
pub fn detect_hygiene_from_paths<I, S>(paths: I) -> RepoHygiene
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut hygiene = RepoHygiene::default();

    for raw in paths {
        let p = raw.as_ref().trim();
        let lower = p.to_ascii_lowercase();

        // 1. README detection
        if lower == "readme"
            || lower.starts_with("readme.")
            || lower.starts_with(".github/readme.")
            || lower.starts_with("docs/readme.")
        {
            hygiene.has_readme = true;
        }

        // 2. LICENSE detection
        if lower == "license"
            || lower.starts_with("license.")
            || lower == "copying"
            || lower.starts_with("copying.")
            || lower == "unlicense"
            || lower.starts_with("unlicense.")
            || lower.starts_with(".github/license.")
        {
            hygiene.has_license = true;
        }

        // 3. SECURITY policy detection
        if lower == "security.md"
            || lower == ".github/security.md"
            || lower == "docs/security.md"
            || lower.starts_with("security.")
            || lower.starts_with(".github/security.")
            || lower.starts_with("docs/security.")
        {
            hygiene.has_security_policy = true;
        }

        // 4. CONTRIBUTING guidelines detection
        if lower == "contributing"
            || lower.starts_with("contributing.")
            || lower.starts_with(".github/contributing.")
            || lower.starts_with("docs/contributing.")
        {
            hygiene.has_contributing = true;
        }

        // 5. CODEOWNERS detection
        if lower == "codeowners" || lower == ".github/codeowners" || lower == "docs/codeowners" {
            hygiene.has_codeowners = true;
        }

        // 6. GitHub Actions CI detection
        if lower.starts_with(".github/workflows/")
            && (lower.ends_with(".yml") || lower.ends_with(".yaml"))
        {
            hygiene.has_actions_ci = true;
        }

        // 7. Dependabot configuration detection
        if lower == ".github/dependabot.yml" || lower == ".github/dependabot.yaml" {
            hygiene.has_dependabot = true;
        }
    }

    hygiene
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_standard_root_files() {
        let paths = vec![
            "README.md",
            "LICENSE",
            "SECURITY.md",
            "CONTRIBUTING.md",
            "CODEOWNERS",
            ".github/workflows/ci.yml",
            ".github/dependabot.yml",
        ];

        let h = detect_hygiene_from_paths(&paths);
        assert!(h.has_readme);
        assert!(h.has_license);
        assert!(h.has_security_policy);
        assert!(h.has_contributing);
        assert!(h.has_codeowners);
        assert!(h.has_actions_ci);
        assert!(h.has_dependabot);
    }

    #[test]
    fn detects_github_folder_variants() {
        let paths = vec![
            ".github/README.md",
            ".github/SECURITY.md",
            ".github/CONTRIBUTING.md",
            ".github/CODEOWNERS",
            ".github/workflows/deploy.yaml",
        ];

        let h = detect_hygiene_from_paths(&paths);
        assert!(h.has_readme);
        assert!(h.has_security_policy);
        assert!(h.has_contributing);
        assert!(h.has_codeowners);
        assert!(h.has_actions_ci);
        assert!(!h.has_license);
        assert!(!h.has_dependabot);
    }

    #[test]
    fn detects_docs_folder_variants() {
        let paths = vec![
            "docs/README.md",
            "docs/SECURITY.md",
            "docs/CONTRIBUTING.md",
            "docs/CODEOWNERS",
        ];

        let h = detect_hygiene_from_paths(&paths);
        assert!(h.has_readme);
        assert!(h.has_security_policy);
        assert!(h.has_contributing);
        assert!(h.has_codeowners);
    }

    #[test]
    fn handles_empty_or_non_matching_paths() {
        let paths = vec!["src/main.rs", "Cargo.toml", "package.json"];
        let h = detect_hygiene_from_paths(&paths);
        assert!(!h.has_readme);
        assert!(!h.has_license);
        assert!(!h.has_security_policy);
        assert!(!h.has_contributing);
        assert!(!h.has_codeowners);
        assert!(!h.has_actions_ci);
        assert!(!h.has_dependabot);
    }
}
