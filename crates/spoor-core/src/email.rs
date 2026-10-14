use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmailType {
    /// Email directly exposed on user's public GitHub profile.
    PublicProfile,

    /// Modern privacy noreply email: `<id>+<username>@users.noreply.github.com`.
    GitHubNoReply,

    /// Legacy privacy noreply email: `<username>@users.noreply.github.com`.
    LegacyGitHubNoReply,

    /// Custom corporate, academic, or personal domain email observed in commit headers.
    DomainEmail,

    /// Malformed or unclassifiable email.
    Unknown,
}

impl EmailType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmailType::PublicProfile => "Public Profile Email",
            EmailType::GitHubNoReply => "GitHub Privacy NoReply",
            EmailType::LegacyGitHubNoReply => "GitHub Legacy NoReply",
            EmailType::DomainEmail => "Domain Email (Commit Metadata)",
            EmailType::Unknown => "Unknown / Unclassified",
        }
    }
}

pub fn classify_email(email: &str, profile_public_email: Option<&str>) -> EmailType {
    let trimmed = email.trim().to_lowercase();
    if trimmed.is_empty() || !trimmed.contains('@') {
        return EmailType::Unknown;
    }

    if let Some(pub_email) = profile_public_email {
        if trimmed == pub_email.trim().to_lowercase() {
            return EmailType::PublicProfile;
        }
    }

    if trimmed.ends_with("@users.noreply.github.com") {
        let local_part = trimmed.split('@').next().unwrap_or("");
        if local_part.contains('+') {
            let mut parts = local_part.splitn(2, '+');
            let id_part = parts.next().unwrap_or("");
            if !id_part.is_empty() && id_part.chars().all(|c| c.is_ascii_digit()) {
                return EmailType::GitHubNoReply;
            }
        }
        return EmailType::LegacyGitHubNoReply;
    }

    if trimmed.ends_with("@noreply.github.com") {
        return EmailType::LegacyGitHubNoReply;
    }

    if let Some((_user, domain)) = trimmed.split_once('@') {
        if domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') {
            return EmailType::DomainEmail;
        }
    }

    EmailType::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_modern_noreply() {
        assert_eq!(
            classify_email("245930475+Voldigoade@users.noreply.github.com", None),
            EmailType::GitHubNoReply
        );
    }

    #[test]
    fn classifies_legacy_noreply() {
        assert_eq!(
            classify_email("torvalds@users.noreply.github.com", None),
            EmailType::LegacyGitHubNoReply
        );
        assert_eq!(
            classify_email("torvalds@noreply.github.com", None),
            EmailType::LegacyGitHubNoReply
        );
    }

    #[test]
    fn classifies_public_profile_email() {
        assert_eq!(
            classify_email("linus@example.com", Some("linus@example.com")),
            EmailType::PublicProfile
        );
    }

    #[test]
    fn classifies_domain_email() {
        assert_eq!(
            classify_email("torvalds@linux-foundation.org", None),
            EmailType::DomainEmail
        );
    }

    #[test]
    fn classifies_malformed_email() {
        assert_eq!(classify_email("not-an-email", None), EmailType::Unknown);
        assert_eq!(classify_email("", None), EmailType::Unknown);
    }
}
