//! Conservative shared classification for untrusted maintainer messages.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintainerIntent {
    Acknowledgement,
    FactualQuestion,
    ChangeRequest,
    Clarification,
    StopRequest,
    OptOutRequest,
    SecurityReport,
    AbuseOrUnrelated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementDisposition {
    NoResponse,
    AwaitOperator,
    ProposeChange,
    PauseRepository,
    Quarantine,
}

impl MaintainerIntent {
    pub const fn disposition(self) -> EngagementDisposition {
        match self {
            Self::Acknowledgement => EngagementDisposition::NoResponse,
            Self::ChangeRequest => EngagementDisposition::ProposeChange,
            Self::StopRequest | Self::OptOutRequest | Self::SecurityReport => {
                EngagementDisposition::PauseRepository
            }
            Self::AbuseOrUnrelated => EngagementDisposition::Quarantine,
            Self::FactualQuestion | Self::Clarification | Self::Unknown => {
                EngagementDisposition::AwaitOperator
            }
        }
    }
}

pub fn trusted_author_association(value: Option<&str>) -> bool {
    matches!(value, Some("OWNER" | "MEMBER" | "COLLABORATOR"))
}

pub fn classify_maintainer_message(body: &str, review_state: Option<&str>) -> MaintainerIntent {
    let normalized = body.trim().to_ascii_lowercase();
    if contains_any(
        &normalized,
        &[
            "security vulnerability",
            "security issue",
            "responsible disclosure",
            "private disclosure",
            "cve-",
            "proof of concept",
            "proof-of-concept",
            "working exploit",
            "actively exploited",
            "can be exploited",
            "exploitable",
            "remote code execution",
            "privilege escalation",
            "credential exposure",
            "credentials can be exposed",
        ],
    ) {
        return MaintainerIntent::SecurityReport;
    }
    if contains_any(
        &normalized,
        &[
            "opt out",
            "opt-out",
            "do not contribute",
            "don't contribute",
            "never open another",
            "block this bot",
        ],
    ) {
        return MaintainerIntent::OptOutRequest;
    }
    if contains_any(
        &normalized,
        &[
            "please stop",
            "stop working",
            "stop commenting",
            "stop updating",
            "close this",
            "close the pr",
            "close the pull request",
        ],
    ) {
        return MaintainerIntent::StopRequest;
    }
    if review_state.is_some_and(|state| state.eq_ignore_ascii_case("changes_requested"))
        || contains_any(
            &normalized,
            &[
                "please change",
                "please fix",
                "please update",
                "please add",
                "please remove",
                "needs changes",
                "requested changes",
                "can you change",
                "can you fix",
                "could you change",
                "could you fix",
            ],
        )
    {
        return MaintainerIntent::ChangeRequest;
    }
    if normalized.ends_with('?') || normalized.contains("?\n") {
        return MaintainerIntent::FactualQuestion;
    }
    if contains_any(
        &normalized,
        &[
            "please clarify",
            "not sure what",
            "what do you mean",
            "unclear",
        ],
    ) {
        return MaintainerIntent::Clarification;
    }
    if is_acknowledgement(&normalized) {
        return MaintainerIntent::Acknowledgement;
    }
    MaintainerIntent::Unknown
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn is_acknowledgement(value: &str) -> bool {
    let trimmed = value.trim_matches(|character: char| {
        character.is_whitespace() || character.is_ascii_punctuation()
    });
    matches!(
        trimmed,
        "thanks"
            | "thank you"
            | "looks good"
            | "lgtm"
            | "great"
            | "nice"
            | "awesome"
            | "perfect"
            | "approved"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        classify_maintainer_message, trusted_author_association, EngagementDisposition,
        MaintainerIntent,
    };

    #[test]
    fn repository_authority_is_explicit() {
        for association in ["OWNER", "MEMBER", "COLLABORATOR"] {
            assert!(trusted_author_association(Some(association)));
        }
        for association in ["CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR", "NONE"] {
            assert!(!trusted_author_association(Some(association)));
        }
        assert!(!trusted_author_association(None));
    }

    #[test]
    fn halt_language_precedes_patch_language() {
        let intent = classify_maintainer_message(
            "Please stop updating this and close the PR instead of fixing it.",
            Some("changes_requested"),
        );
        assert_eq!(intent, MaintainerIntent::StopRequest);
        assert_eq!(intent.disposition(), EngagementDisposition::PauseRepository);
    }

    #[test]
    fn acknowledgements_never_become_patch_requests() {
        assert_eq!(
            classify_maintainer_message("LGTM!", None),
            MaintainerIntent::Acknowledgement
        );
        assert_eq!(
            classify_maintainer_message("Thanks", None).disposition(),
            EngagementDisposition::NoResponse
        );
    }

    #[test]
    fn formal_and_ambiguous_feedback_stay_distinct() {
        assert_eq!(
            classify_maintainer_message("This needs another pass", Some("changes_requested")),
            MaintainerIntent::ChangeRequest
        );
        assert_eq!(
            classify_maintainer_message("Why was this dependency selected?", None),
            MaintainerIntent::FactualQuestion
        );
        assert_eq!(
            classify_maintainer_message("I have some thoughts", None),
            MaintainerIntent::Unknown
        );
    }

    #[test]
    fn incidental_exploit_word_does_not_pause_the_repository() {
        assert_eq!(
            classify_maintainer_message(
                "Please exploit the existing hook instead of adding another abstraction.",
                None,
            ),
            MaintainerIntent::Unknown
        );
    }

    #[test]
    fn credible_security_language_still_pauses_the_repository() {
        for message in [
            "This is a security vulnerability; please contact us privately.",
            "I have a working exploit for remote code execution.",
            "User credentials can be exposed through this endpoint.",
            "This is CVE-2026-12345.",
        ] {
            let intent = classify_maintainer_message(message, None);
            assert_eq!(intent, MaintainerIntent::SecurityReport, "{message}");
            assert_eq!(
                intent.disposition(),
                EngagementDisposition::PauseRepository,
                "{message}"
            );
        }
    }
}
