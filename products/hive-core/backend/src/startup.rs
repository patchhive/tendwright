use once_cell::sync::OnceCell;
use patchhive_product_core::secrets::{validate_encryption_secret, TokenProtector};
use patchhive_product_core::sqlite::db_path_message;
use patchhive_product_core::startup::{StartupCheck, StartupCheckLevel};

use crate::models::{SuiteBootstrapAuthoritySource, SuiteBootstrapAuthorityState};

static STARTUP_CHECKS: OnceCell<Vec<StartupCheck>> = OnceCell::new();

pub fn set_startup_checks(checks: Vec<StartupCheck>) {
    let _ = STARTUP_CHECKS.set(checks);
}

pub fn startup_checks() -> Vec<StartupCheck> {
    STARTUP_CHECKS.get().cloned().unwrap_or_default()
}

pub async fn validate_config() -> Vec<StartupCheck> {
    let mut checks = Vec::new();

    checks.push(StartupCheck::info(db_path_message(
        "HiveCore",
        crate::db::db_path(),
    )));

    if crate::auth::auth_enabled() {
        checks.push(StartupCheck::info("API-key auth is enabled for HiveCore."));
    } else {
        checks.push(StartupCheck::warn(
            "API-key auth is not enabled yet. Generate a key before exposing HiveCore beyond local development.",
        ).with_identity("api_key_auth", "missing"));
    }

    checks.push(StartupCheck::info(
        "HiveCore ships with a built-in localhost product registry and can persist per-product URL overrides for subdomains or remote deployments.",
    ));

    let engagement_webhook_configured = [
        "HIVE_CORE_GITHUB_WEBHOOK_SECRET",
        "PATCHHIVE_GITHUB_WEBHOOK_SECRET",
    ]
    .into_iter()
    .any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    });
    if engagement_webhook_configured && crate::engagements::configured_bot_login().is_some() {
        checks.push(
            StartupCheck::ok(
                "Signed GitHub maintainer-engagement ingestion and bot self-filtering are configured.",
            )
                .with_identity("maintainer_engagement_webhook", "configured"),
        );
    } else if engagement_webhook_configured {
        checks.push(
            StartupCheck::error(
                "Maintainer-engagement ingestion requires PATCHHIVE_GITHUB_BOT_LOGIN (or legacy BOT_GITHUB_USER) so Tendwright cannot ingest its own messages.",
            )
            .with_identity("maintainer_engagement_webhook", "invalid"),
        );
    } else {
        checks.push(
            StartupCheck::warn(
                "Maintainer-engagement ingestion is inactive until HIVE_CORE_GITHUB_WEBHOOK_SECRET is configured.",
            )
            .with_identity("maintainer_engagement_webhook", "missing"),
        );
    }

    let override_count = crate::db::product_override_count();
    if override_count == 0 {
        checks.push(StartupCheck::info(
            "HiveCore is currently using its built-in default product URLs. Save suite settings to override them per environment.",
        ));
    } else {
        checks.push(StartupCheck::ok(format!(
            "HiveCore has {} persisted product override{} ready for launch links and health polling.",
            override_count,
            if override_count == 1 { "" } else { "s" }
        )));
    }

    let repository_policy_count = crate::db::repository_policies().len();
    checks.push(StartupCheck::ok(format!(
        "HiveCore repository safety is active with {repository_policy_count} structured polic{}; local exclusions and trusted-repository elevations are available to suite products.",
        if repository_policy_count == 1 { "y" } else { "ies" }
    )));
    match crate::db::suite_pr_limit() {
        Ok(limit) => checks.push(StartupCheck::ok(format!(
            "Atomic pull-request budgets are active with a suite-wide ceiling of {limit}. RepoReaper reserves capacity before PR creation and releases it when monitored PRs close or merge."
        ))),
        Err(error) => checks.push(StartupCheck::error(format!(
            "HiveCore could not read the suite pull-request budget: {error}"
        ))),
    }
    let opt_out_feed_configured = std::env::var("PATCHHIVE_OPT_OUT_FEED_URL")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    let opt_out_key_configured = std::env::var("PATCHHIVE_OPT_OUT_SYNC_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    match (opt_out_feed_configured, opt_out_key_configured) {
        (true, true) => checks.push(
            StartupCheck::ok(
                "Authenticated repository-owner opt-out synchronization is configured.",
            )
            .with_identity("public_opt_out_sync", "configured"),
        ),
        (true, false) => checks.push(
            StartupCheck::error(
                "PATCHHIVE_OPT_OUT_FEED_URL is configured without PATCHHIVE_OPT_OUT_SYNC_KEY; the canonical opt-out feed cannot be authenticated.",
            )
            .with_identity("public_opt_out_sync", "invalid"),
        ),
        (false, true) => checks.push(
            StartupCheck::warn(
                "PATCHHIVE_OPT_OUT_SYNC_KEY is configured without PATCHHIVE_OPT_OUT_FEED_URL; public opt-out synchronization is not active.",
            )
            .with_identity("public_opt_out_sync", "incomplete"),
        ),
        (false, false) => checks.push(
            StartupCheck::info(
                "Public repository-owner opt-out synchronization is not configured; HiveCore continues enforcing durable local repository policy.",
            )
            .with_identity("public_opt_out_sync", "not_configured"),
        ),
    }

    match std::env::var("HIVECORE_APPROVAL_TTL_HOURS") {
        Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u32>() {
            Ok(hours @ 1..=168) => checks.push(StartupCheck::ok(format!(
                "HiveCore exact-dispatch approvals expire after {hours} hour{} unless consumed first.",
                if hours == 1 { "" } else { "s" }
            ))),
            _ => checks.push(StartupCheck::error(
                "HIVECORE_APPROVAL_TTL_HOURS must be an integer from 1 through 168. HiveCore will use its conservative 24-hour default until this is corrected.",
            )),
        },
        _ => checks.push(StartupCheck::info(
            "HiveCore exact-dispatch approvals use the default 24-hour expiry.",
        )),
    }

    match crate::bootstrap_authority::current().state {
        SuiteBootstrapAuthorityState::Ready {
            source,
            established_at,
        } => {
            let source_label = match source {
                SuiteBootstrapAuthoritySource::Environment => "PATCHHIVE_SUITE_BOOTSTRAP_SECRET",
                SuiteBootstrapAuthoritySource::PersistedEncrypted => "encrypted SQLite authority",
            };
            let established = established_at
                .map(|value| format!(" Established at {value}."))
                .unwrap_or_default();
            checks.push(
                StartupCheck::ok(format!(
                    "Suite bootstrap authority is ready from {source_label}.{established}"
                ))
                .with_identity("suite_bootstrap_authority", "ready"),
            );
        }
        SuiteBootstrapAuthorityState::NotConfigured { reason } => checks.push(
            StartupCheck::warn(format!(
                "Suite bootstrap authority is not configured: {reason}"
            ))
            .with_identity("suite_bootstrap_authority", "not_configured"),
        ),
        SuiteBootstrapAuthorityState::Invalid { reason, .. } => checks.push(
            StartupCheck::error(format!("Suite bootstrap authority is invalid: {reason}"))
                .with_identity("suite_bootstrap_authority", "invalid"),
        ),
        SuiteBootstrapAuthorityState::Unknown { reason } => checks.push(
            StartupCheck::error(format!("Suite bootstrap authority is unknown: {reason}"))
                .with_identity("suite_bootstrap_authority", "unknown"),
        ),
    }

    let token_stats = crate::db::service_token_storage_stats();
    let protector = TokenProtector::from_env("HIVECORE_ENCRYPTION_KEY");
    if let Ok(secret) = std::env::var("HIVECORE_ENCRYPTION_KEY") {
        let secret = secret.trim();
        if !secret.is_empty() {
            match validate_encryption_secret(secret) {
                Ok(()) => checks.push(StartupCheck::ok(
                    "HIVECORE_ENCRYPTION_KEY is configured with sufficient machine-random key material.",
                )),
                Err(error) => checks.push(StartupCheck::error(format!(
                    "HIVECORE_ENCRYPTION_KEY is not safe encryption key material: {error}"
                ))),
            }
        }
    }
    if token_stats.total == 0 {
        checks.push(StartupCheck::info(
            "HiveCore has no saved downstream product service tokens yet.",
        ));
    } else if protector.configured() {
        if token_stats.plaintext == 0 {
            checks.push(StartupCheck::ok(format!(
                "HiveCore has {} saved product service token{} encrypted at rest.",
                token_stats.total,
                if token_stats.total == 1 { "" } else { "s" }
            )));
        } else {
            checks.push(StartupCheck::warn(format!(
                "HiveCore still has {} plaintext product service token{} in SQLite. Restart with HIVECORE_ENCRYPTION_KEY and let boot migration finish before trusting at-rest protection.",
                token_stats.plaintext,
                if token_stats.plaintext == 1 { "" } else { "s" }
            )));
        }
    } else {
        if token_stats.encrypted > 0 {
            checks.push(StartupCheck::warn(format!(
                "HIVECORE_ENCRYPTION_KEY is not set, but {} saved product service token{} are encrypted. HiveCore cannot read them until that key is restored.",
                token_stats.encrypted,
                if token_stats.encrypted == 1 { "" } else { "s" }
            )));
        }
        if token_stats.plaintext > 0 {
            checks.push(StartupCheck::warn(format!(
                "HIVECORE_ENCRYPTION_KEY is not set. HiveCore currently keeps {} product service token{} in plaintext SQLite storage.",
                token_stats.plaintext,
                if token_stats.plaintext == 1 { "" } else { "s" }
            )));
        }
    }

    checks.push(StartupCheck::info(
        "HiveCore provides visibility, saved defaults, live product health polling, repository policy, shared outbound PR capacity, and durable single-use dispatch approvals. Additional products should adopt the same typed policy client before gaining write actions.",
    ));

    checks
}

pub fn summarize_check_levels(checks: &[StartupCheck]) -> (u32, u32, u32) {
    let mut errors = 0;
    let mut warns = 0;
    let mut infos = 0;

    for check in checks {
        match check.level {
            StartupCheckLevel::Error => errors += 1,
            StartupCheckLevel::Warn => warns += 1,
            _ => infos += 1,
        }
    }

    (errors, warns, infos)
}
