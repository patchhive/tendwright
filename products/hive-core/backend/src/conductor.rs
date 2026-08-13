use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use uuid::Uuid;

pub use patchhive_product_core::hivecore_kernel::AutonomyLevel as MandateAutonomy;
use patchhive_product_core::hivecore_kernel::{
    AdmissionDecision, AdmissionEvidence, AutonomyDecision,
};

use crate::{
    models::{now_rfc3339, DispatchActionResponse},
    pipeline::dispatch::dispatch_with_approval,
    state::AppState,
};

/// The stable identity of one piece of maintenance work.
///
/// Product, action, mandate, and discovery source deliberately do not participate
/// in this identity. Two products finding the same work must converge on one row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkIdentity {
    pub kind: String,
    pub repository: String,
    pub subject_ref: String,
}

impl WorkIdentity {
    fn normalized(self) -> Result<Self, String> {
        let kind = required("kind", self.kind, 80)?.to_ascii_lowercase();
        if !kind
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            return Err("kind must contain only letters, numbers, hyphens, or underscores".into());
        }
        let repository = required("repository", self.repository, 240)?.to_ascii_lowercase();
        let subject_ref = required("subject_ref", self.subject_ref, 500)?;
        let parts = repository.split('/').collect::<Vec<_>>();
        if parts.len() != 2
            || parts.iter().any(|part| part.is_empty())
            || repository.chars().any(char::is_whitespace)
        {
            return Err("repository must be a GitHub owner/repository name".into());
        }
        Ok(Self {
            kind,
            repository,
            subject_ref,
        })
    }

    pub fn fingerprint(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("work identity serialization cannot fail");
        hex::encode(Sha256::digest(bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum WorkOrigin {
    Operator,
    ProductRun {
        product_slug: String,
        run_id: String,
    },
    SuiteRun {
        run_id: String,
    },
    ConductorTick {
        tick_id: String,
    },
}

impl WorkOrigin {
    fn normalized(self) -> Result<Self, String> {
        match self {
            Self::Operator => Ok(Self::Operator),
            Self::ProductRun {
                product_slug,
                run_id,
            } => Ok(Self::ProductRun {
                product_slug: required("origin product_slug", product_slug, 100)?,
                run_id: required("origin run_id", run_id, 200)?,
            }),
            Self::SuiteRun { run_id } => Ok(Self::SuiteRun {
                run_id: required("origin run_id", run_id, 200)?,
            }),
            Self::ConductorTick { tick_id } => Ok(Self::ConductorTick {
                tick_id: required("origin tick_id", tick_id, 200)?,
            }),
        }
    }
}

/// The dispatch HiveCore is proposing, not permission or an instruction to run it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedDispatch {
    pub product_slug: String,
    pub action_id: String,
    pub input: Value,
}

impl ProposedDispatch {
    fn normalized(self) -> Result<Self, String> {
        if !self.input.is_object() {
            return Err("proposed dispatch input must be a JSON object".into());
        }
        Ok(Self {
            product_slug: required("product_slug", self.product_slug, 100)?,
            action_id: required("action_id", self.action_id, 100)?,
            input: self.input,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposeWorkRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mandate_id: Option<String>,
    pub identity: WorkIdentity,
    pub proposed_dispatch: ProposedDispatch,
    pub origin: WorkOrigin,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkProposal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mandate_id: Option<String>,
    pub identity: WorkIdentity,
    pub proposed_dispatch: ProposedDispatch,
    pub origin: WorkOrigin,
    pub rationale: String,
}

impl WorkProposal {
    pub fn from_request(request: ProposeWorkRequest) -> Result<Self, String> {
        let mandate_id = request
            .mandate_id
            .map(|value| required("mandate_id", value, 200))
            .transpose()?;
        Ok(Self {
            mandate_id,
            identity: request.identity.normalized()?,
            proposed_dispatch: request.proposed_dispatch.normalized()?,
            origin: request.origin.normalized()?,
            rationale: required("rationale", request.rationale, 2_000)?,
        })
    }
}

/// Durable, restart-safe state for one concrete repository work item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkLifecycle {
    Discovered {
        discovered_at: String,
    },
    Dispatching {
        claim_id: String,
        started_at: String,
        lease_until: String,
    },
    AwaitingApproval {
        approval_id: String,
        requested_at: String,
    },
    Gated {
        gate_product: String,
        gate_run_id: String,
        recommendation: String,
        gated_at: String,
    },
    Dispatched {
        action_event_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        receiving_run_id: Option<String>,
        dispatched_at: String,
    },
    Shipped {
        pr_url: String,
        shipped_at: String,
    },
    Completed {
        outcome: String,
        completed_at: String,
    },
    Blocked {
        reason: String,
        blocked_at: String,
        retryable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        next_attempt_at: Option<String>,
    },
    Failed {
        reason: String,
        failed_at: String,
        retryable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        next_attempt_at: Option<String>,
    },
    Abandoned {
        reason: String,
        abandoned_at: String,
    },
    Unknown {
        raw_state: String,
        raw_evidence: Value,
    },
}

impl WorkLifecycle {
    pub const fn kind(&self) -> &str {
        match self {
            Self::Discovered { .. } => "discovered",
            Self::Dispatching { .. } => "dispatching",
            Self::AwaitingApproval { .. } => "awaiting_approval",
            Self::Gated { .. } => "gated",
            Self::Dispatched { .. } => "dispatched",
            Self::Shipped { .. } => "shipped",
            Self::Completed { .. } => "completed",
            Self::Blocked { .. } => "blocked",
            Self::Failed { .. } => "failed",
            Self::Abandoned { .. } => "abandoned",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub fn from_storage(raw_state: String, raw_evidence: Value) -> Self {
        let parsed = serde_json::from_value::<Self>(raw_evidence.clone());
        match parsed {
            Ok(value) if value.kind() == raw_state => value,
            _ => Self::Unknown {
                raw_state,
                raw_evidence,
            },
        }
    }

    pub fn active_claim(&self) -> Option<&str> {
        match self {
            Self::Dispatching { claim_id, .. } => Some(claim_id),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. }
                | Self::Abandoned { .. }
                | Self::Blocked {
                    retryable: false,
                    ..
                }
                | Self::Failed {
                    retryable: false,
                    ..
                }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkItem {
    pub id: String,
    pub fingerprint: String,
    pub proposal: WorkProposal,
    pub lifecycle: WorkLifecycle,
    pub attempts: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkClaim {
    pub claim_id: String,
    pub item: WorkItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkHandoffEdge {
    pub from_product: String,
    pub to_product: String,
    pub work_items: u32,
    pub active_work_items: u32,
    pub last_observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SuiteLedgerEvent {
    pub id: String,
    pub entity_kind: String,
    pub entity_id: String,
    pub event_kind: String,
    pub evidence: Value,
    pub created_at: String,
}

impl WorkItem {
    pub fn discovered(proposal: WorkProposal) -> Self {
        let now = now_rfc3339();
        Self {
            id: format!("work_{}", Uuid::now_v7()),
            fingerprint: proposal.identity.fingerprint(),
            proposal,
            lifecycle: WorkLifecycle::Discovered {
                discovered_at: now.clone(),
            },
            attempts: 0,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ProposeWorkOutcome {
    Created { item: WorkItem },
    Deduplicated { item: WorkItem, observed_at: String },
}

/// Stable evidence identity supplied by the product that observed a finding.
/// A retry of the same product/run/finding tuple is one receipt, while another
/// product or run may independently rediscover the same work identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingSource {
    pub product_slug: String,
    pub run_id: String,
    pub finding_id: String,
}

impl FindingSource {
    fn normalized(self) -> Result<Self, String> {
        Ok(Self {
            product_slug: required("source product_slug", self.product_slug, 100)?
                .to_ascii_lowercase(),
            run_id: required("source run_id", self.run_id, 200)?,
            finding_id: required("source finding_id", self.finding_id, 500)?,
        })
    }

    pub fn fingerprint(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("finding source serialization cannot fail");
        hex::encode(Sha256::digest(bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductFinding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mandate_id: Option<String>,
    pub source: FindingSource,
    pub identity: WorkIdentity,
    pub proposed_dispatch: ProposedDispatch,
    pub rationale: String,
    pub evidence: Value,
}

impl ProductFinding {
    pub fn validated(self) -> Result<Self, String> {
        if !self.evidence.is_object() {
            return Err("finding evidence must be a JSON object".into());
        }
        Ok(Self {
            mandate_id: self
                .mandate_id
                .map(|value| required("mandate_id", value, 200))
                .transpose()?,
            source: self.source.normalized()?,
            identity: self.identity.normalized()?,
            proposed_dispatch: self.proposed_dispatch.normalized()?,
            rationale: required("rationale", self.rationale, 2_000)?,
            evidence: self.evidence,
        })
    }

    pub fn proposal(&self) -> WorkProposal {
        WorkProposal {
            mandate_id: self.mandate_id.clone(),
            identity: self.identity.clone(),
            proposed_dispatch: self.proposed_dispatch.clone(),
            origin: WorkOrigin::ProductRun {
                product_slug: self.source.product_slug.clone(),
                run_id: self.source.run_id.clone(),
            },
            rationale: self.rationale.clone(),
        }
    }

    pub fn fingerprint(&self) -> String {
        let value = serde_json::to_value(self).expect("product finding serialization cannot fail");
        let canonical = patchhive_product_core::approvals::canonical_json(&value);
        let bytes = serde_json::to_vec(&canonical)
            .expect("canonical product finding serialization cannot fail");
        hex::encode(Sha256::digest(bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestFindingsRequest {
    pub findings: Vec<ProductFinding>,
}

impl IngestFindingsRequest {
    pub fn validated(self) -> Result<Vec<ProductFinding>, String> {
        if self.findings.is_empty() {
            return Err("findings must contain at least one item".into());
        }
        if self.findings.len() > 100 {
            return Err("findings must contain at most 100 items".into());
        }
        let findings = self
            .findings
            .into_iter()
            .map(ProductFinding::validated)
            .collect::<Result<Vec<_>, _>>()?;
        let mut sources = HashSet::with_capacity(findings.len());
        if findings
            .iter()
            .any(|finding| !sources.insert(finding.source.fingerprint()))
        {
            return Err("a finding source may appear only once in an ingestion batch".into());
        }
        Ok(findings)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingReceipt {
    pub finding: ProductFinding,
    pub work_item_id: String,
    pub work_fingerprint: String,
    pub finding_fingerprint: String,
    pub ingested_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum FindingIngestionDisposition {
    Created,
    Deduplicated,
    AlreadyIngested,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingIngestionResult {
    pub disposition: FindingIngestionDisposition,
    pub receipt: FindingReceipt,
    pub item: WorkItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestFindingsOutcome {
    pub results: Vec<FindingIngestionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MandateScope {
    pub search_query: String,
    pub topics: Vec<String>,
    pub languages: Vec<String>,
    pub min_stars: u32,
    pub max_repositories: u32,
    pub issues_per_repository: u32,
    pub stale_days: u32,
}

impl MandateScope {
    fn normalized(self) -> Result<Self, String> {
        let search_query = self.search_query.trim().to_owned();
        if search_query.starts_with("repo:") {
            return Err(
                "mandate scope is autonomous discovery; use product direct targeting for one repository"
                    .into(),
            );
        }
        let topics = normalized_terms("topics", self.topics)?;
        let languages = normalized_terms("languages", self.languages)?;
        if search_query.is_empty() && topics.is_empty() && languages.is_empty() {
            return Err("mandate scope requires a search query, topic, or language".into());
        }
        if self.max_repositories == 0 || self.max_repositories > 25 {
            return Err("max_repositories must be between 1 and 25".into());
        }
        if !(5..=100).contains(&self.issues_per_repository) {
            return Err("issues_per_repository must be between 5 and 100".into());
        }
        if self.stale_days == 0 || self.stale_days > 730 {
            return Err("stale_days must be between 1 and 730".into());
        }
        if self.min_stars > 1_000_000 {
            return Err("min_stars must not exceed 1000000".into());
        }
        Ok(Self {
            search_query,
            topics,
            languages,
            min_stars: self.min_stars,
            max_repositories: self.max_repositories,
            issues_per_repository: self.issues_per_repository,
            stale_days: self.stale_days,
        })
    }

    fn signal_hive_input(&self, max_repositories: u32) -> Value {
        serde_json::json!({
            "search_query": self.search_query,
            "topics": self.topics,
            "languages": self.languages,
            "min_stars": self.min_stars,
            "max_repos": max_repositories,
            "issues_per_repo": self.issues_per_repository,
            "stale_days": self.stale_days,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MandateLimits {
    pub pr_budget: u32,
    pub cost_budget_cents_per_day: u64,
    pub per_owner_open_prs: u32,
    pub cooldown_after_close_days: u32,
}

impl MandateLimits {
    fn validated(self) -> Result<Self, String> {
        if self.pr_budget > 100 {
            return Err("pr_budget must not exceed 100".into());
        }
        if self.cost_budget_cents_per_day > 1_000_000 {
            return Err("cost_budget_cents_per_day must not exceed 1000000".into());
        }
        if self.per_owner_open_prs == 0 || self.per_owner_open_prs > 20 {
            return Err("per_owner_open_prs must be between 1 and 20".into());
        }
        if self.cooldown_after_close_days > 365 {
            return Err("cooldown_after_close_days must not exceed 365".into());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveMandateRequest {
    pub name: String,
    pub objective: String,
    pub scope: MandateScope,
    pub requested_autonomy: MandateAutonomy,
    pub limits: MandateLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateMandateRequest {
    pub expected_revision: u64,
    pub mandate: SaveMandateRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MandateConfig {
    pub name: String,
    pub objective: String,
    pub scope: MandateScope,
    pub requested_autonomy: MandateAutonomy,
    pub limits: MandateLimits,
}

impl MandateConfig {
    pub fn from_request(request: SaveMandateRequest) -> Result<Self, String> {
        Self {
            name: request.name,
            objective: request.objective,
            scope: request.scope,
            requested_autonomy: request.requested_autonomy,
            limits: request.limits,
        }
        .validated()
    }

    pub fn validated(self) -> Result<Self, String> {
        Ok(Self {
            name: required("name", self.name, 120)?,
            objective: required("objective", self.objective, 2_000)?,
            scope: self.scope.normalized()?,
            requested_autonomy: self.requested_autonomy,
            limits: self.limits.validated()?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MandateLifecycle {
    Active {
        activated_at: String,
    },
    Paused {
        paused_at: String,
        reason: String,
    },
    Archived {
        archived_at: String,
        reason: String,
    },
    Unknown {
        raw_state: String,
        raw_evidence: Value,
    },
}

impl MandateLifecycle {
    pub const fn kind(&self) -> &str {
        match self {
            Self::Active { .. } => "active",
            Self::Paused { .. } => "paused",
            Self::Archived { .. } => "archived",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    pub fn from_storage(raw_state: String, raw_evidence: Value) -> Self {
        let parsed = serde_json::from_value::<Self>(raw_evidence.clone());
        match parsed {
            Ok(value) if value.kind() == raw_state => value,
            _ => Self::Unknown {
                raw_state,
                raw_evidence,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MandateRecord {
    pub id: String,
    pub config: MandateConfig,
    pub lifecycle: MandateLifecycle,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
}

impl MandateRecord {
    pub fn active(config: MandateConfig) -> Self {
        let now = now_rfc3339();
        Self {
            id: format!("mandate_{}", Uuid::now_v7()),
            config,
            lifecycle: MandateLifecycle::Active {
                activated_at: now.clone(),
            },
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MandateReasonRequest {
    pub reason: String,
}

impl MandateReasonRequest {
    pub fn validated(self) -> Result<String, String> {
        required("reason", self.reason, 1_000)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapacityLayer {
    pub limit: u32,
    pub used: u32,
    pub remaining: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapacityLimitingLayer {
    MandateBacklog,
    RepoReaper,
    Suite,
}

/// Exact PR-slot evidence used to size one discovery proposal. Planned units are
/// allocated across the current tick so several mandates cannot each claim the
/// same remaining suite capacity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryCapacity {
    pub suite: CapacityLayer,
    pub repo_reaper: CapacityLayer,
    pub mandate_limit: u32,
    pub concrete_backlog: u32,
    pub mandate_remaining: u32,
    pub allocated_earlier_in_tick: u32,
    pub admitted_repositories: u32,
}

impl DiscoveryCapacity {
    pub fn limiting_layers(&self) -> Vec<CapacityLimitingLayer> {
        let mut layers = Vec::new();
        if self.mandate_remaining == 0 {
            layers.push(CapacityLimitingLayer::MandateBacklog);
        }
        if self.repo_reaper.remaining <= self.allocated_earlier_in_tick {
            layers.push(CapacityLimitingLayer::RepoReaper);
        }
        if self.suite.remaining <= self.allocated_earlier_in_tick {
            layers.push(CapacityLimitingLayer::Suite);
        }
        layers
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ConductorDecision {
    Deferred {
        mandate_id: String,
        reason: String,
    },
    ObservedOnly {
        mandate_id: String,
        requested_autonomy: MandateAutonomy,
        reason: String,
    },
    CapacityDeferred {
        mandate_id: String,
        requested_autonomy: MandateAutonomy,
        capacity: DiscoveryCapacity,
        limiting_layers: Vec<CapacityLimitingLayer>,
        reason: String,
    },
    SmokeDeferred {
        mandate_id: String,
        requested_autonomy: MandateAutonomy,
        earned_autonomy: MandateAutonomy,
        reason: String,
    },
    ResourceDeferred {
        mandate_id: String,
        admission: AdmissionDecision,
        evidence: AdmissionEvidence,
        reason: String,
    },
    PlannedDiscovery {
        mandate_id: String,
        requested_autonomy: MandateAutonomy,
        effective_autonomy: MandateAutonomy,
        earned_autonomy: MandateAutonomy,
        admission: AdmissionDecision,
        admission_evidence: AdmissionEvidence,
        capacity: DiscoveryCapacity,
        proposed_dispatch: ProposedDispatch,
        rationale: String,
    },
}

impl ConductorDecision {
    pub fn observed_only(mandate: &MandateRecord) -> Self {
        if !mandate.lifecycle.is_active() {
            return Self::Deferred {
                mandate_id: mandate.id.clone(),
                reason: "Mandate lifecycle evidence is not an active state.".into(),
            };
        }
        Self::ObservedOnly {
            mandate_id: mandate.id.clone(),
            requested_autonomy: MandateAutonomy::Observe,
            reason: "Observe autonomy records intent without proposing a product action.".into(),
        }
    }

    pub fn with_capacity(
        mandate: &MandateRecord,
        capacity: DiscoveryCapacity,
        autonomy: AutonomyDecision,
        admission: AdmissionDecision,
        admission_evidence: AdmissionEvidence,
    ) -> Self {
        if !mandate.lifecycle.is_active() {
            return Self::Deferred {
                mandate_id: mandate.id.clone(),
                reason: "Mandate lifecycle evidence is not an active state.".into(),
            };
        }
        if capacity.admitted_repositories == 0 {
            let limiting_layers = capacity.limiting_layers();
            return Self::CapacityDeferred {
                mandate_id: mandate.id.clone(),
                requested_autonomy: mandate.config.requested_autonomy,
                capacity,
                limiting_layers,
                reason: "Discovery was deferred because concrete backlog and active PR reservations leave no downstream capacity."
                    .into(),
            };
        }
        if autonomy.effective == MandateAutonomy::Observe {
            return Self::SmokeDeferred {
                mandate_id: mandate.id.clone(),
                requested_autonomy: autonomy.requested,
                earned_autonomy: autonomy.earned,
                reason: autonomy.demotion_reason.unwrap_or_else(|| {
                    "Durable smoke evidence has not earned proposal authority.".into()
                }),
            };
        }
        let admitted_repositories = capacity.admitted_repositories;
        Self::PlannedDiscovery {
            mandate_id: mandate.id.clone(),
            requested_autonomy: mandate.config.requested_autonomy,
            effective_autonomy: autonomy.effective,
            earned_autonomy: autonomy.earned,
            admission,
            admission_evidence,
            capacity,
            proposed_dispatch: ProposedDispatch {
                product_slug: "signal-hive".into(),
                action_id: "scan".into(),
                input: mandate
                    .config
                    .scope
                    .signal_hive_input(admitted_repositories),
            },
            rationale: format!(
                "Ask SignalHive to discover evidence for mandate within {admitted_repositories} downstream-capacity unit(s): {}",
                mandate.config.objective,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConductorTickTrigger {
    Operator,
    Background,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConductorTickLifecycle {
    Running {
        started_at: String,
        lease_until: String,
    },
    Completed {
        started_at: String,
        finished_at: String,
        decisions: Vec<ConductorDecision>,
        remaining_active_mandates: u32,
    },
    Failed {
        started_at: String,
        failed_at: String,
        reason: String,
    },
    Unknown {
        raw_state: String,
        raw_evidence: Value,
    },
}

impl ConductorTickLifecycle {
    pub const fn kind(&self) -> &str {
        match self {
            Self::Running { .. } => "running",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub fn from_storage(raw_state: String, raw_evidence: Value) -> Self {
        let parsed = serde_json::from_value::<Self>(raw_evidence.clone());
        match parsed {
            Ok(value) if value.kind() == raw_state => value,
            _ => Self::Unknown {
                raw_state,
                raw_evidence,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConductorTickRecord {
    pub id: String,
    pub trigger: ConductorTickTrigger,
    pub lifecycle: ConductorTickLifecycle,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RunConductorTickOutcome {
    Settled {
        tick: ConductorTickRecord,
    },
    Busy {
        active_tick_id: String,
        lease_until: String,
    },
}

fn normalized_terms(field: &str, values: Vec<String>) -> Result<Vec<String>, String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    if values.len() > 25 {
        return Err(format!("{field} must contain at most 25 values"));
    }
    if values.iter().any(|value| value.len() > 80) {
        return Err(format!("each {field} value must be at most 80 characters"));
    }
    Ok(values)
}

static BACKGROUND_LOOP_STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

pub fn start_background_loop() {
    if BACKGROUND_LOOP_STARTED.set(()).is_err() {
        return;
    }
    tokio::spawn(async {
        let state = crate::state::AppState::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(
                crate::db::conductor_interval_seconds(),
            ))
            .await;
            match run_tick_and_dispatch(&state, ConductorTickTrigger::Background).await {
                Ok(RunConductorTickOutcome::Settled { tick }) => {
                    tracing::debug!(tick_id = %tick.id, "conductor tick settled");
                }
                Ok(RunConductorTickOutcome::Busy {
                    active_tick_id,
                    lease_until,
                }) => {
                    tracing::debug!(%active_tick_id, %lease_until, "conductor tick skipped because another writer holds the lease");
                }
                Err(error) => {
                    tracing::error!(%error, "background conductor tick failed");
                }
            }
        }
    });
}

pub async fn run_tick_and_dispatch(
    state: &AppState,
    trigger: ConductorTickTrigger,
) -> rusqlite::Result<RunConductorTickOutcome> {
    let admission = crate::pipeline::governance::discovery_admission_evidence(state).await;
    let outcome = crate::db::run_conductor_tick(trigger, admission)?;
    let RunConductorTickOutcome::Settled { tick } = &outcome else {
        return Ok(outcome);
    };
    let ConductorTickLifecycle::Completed { decisions, .. } = &tick.lifecycle else {
        return Ok(outcome);
    };
    for decision in decisions {
        let ConductorDecision::PlannedDiscovery {
            mandate_id,
            proposed_dispatch,
            ..
        } = decision
        else {
            continue;
        };
        let response = dispatch_with_approval(
            state,
            &proposed_dispatch.product_slug,
            &proposed_dispatch.action_id,
            proposed_dispatch.input.clone(),
            patchhive_product_core::approvals::ApprovalOrigin::SuiteRun {
                run_id: tick.id.clone(),
            },
            None,
        )
        .await;
        match response {
            Ok(DispatchActionResponse::Dispatched { event, .. })
                if event.status == "dispatched" =>
            {
                let scan = crate::work_engine::normalized_response(&event.response_json);
                let findings = signal_hive_findings(mandate_id, &scan);
                let ingestion = if findings.is_empty() {
                    None
                } else {
                    Some(crate::db::ingest_findings(findings).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?)
                };
                crate::db::record_suite_event(
                    "conductor_tick",
                    &tick.id,
                    "discovery_settled",
                    &serde_json::json!({
                        "mandate_id": mandate_id,
                        "action_event_id": event.id,
                        "ingestion": ingestion,
                    }),
                )?;
            }
            Ok(response) => {
                crate::db::record_suite_event(
                    "conductor_tick",
                    &tick.id,
                    "discovery_not_accepted",
                    &serde_json::to_value(response).unwrap_or(Value::Null),
                )?;
            }
            Err((status, body)) => {
                crate::db::record_suite_event(
                    "conductor_tick",
                    &tick.id,
                    "discovery_failed",
                    &serde_json::json!({"status": status.as_u16(), "body": body.0}),
                )?;
            }
        }
    }
    let _ = crate::work_engine::run_once(state, 3).await;
    Ok(outcome)
}

fn signal_hive_findings(mandate_id: &str, scan: &Value) -> Vec<ProductFinding> {
    let run_id = scan
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("unknown-signal-hive-run");
    scan.get("repos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|repo| {
            let repository = repo.get("full_name")?.as_str()?.to_ascii_lowercase();
            let issue = repo
                .get("issue_examples")
                .and_then(Value::as_array)
                .and_then(|issues| issues.first());
            let (kind, subject_ref, finding_id) = match issue {
                Some(issue) => {
                    let number = issue.get("number")?.as_u64()?;
                    (
                        "github_issue".to_string(),
                        format!("issue:{number}"),
                        format!("issue:{number}"),
                    )
                }
                None => (
                    "maintenance_pressure".to_string(),
                    "signal-hive:repository-pressure".to_string(),
                    "repository-pressure".to_string(),
                ),
            };
            Some(ProductFinding {
                mandate_id: Some(mandate_id.to_owned()),
                source: FindingSource {
                    product_slug: "signal-hive".into(),
                    run_id: run_id.to_owned(),
                    finding_id,
                },
                identity: WorkIdentity {
                    kind,
                    repository: repository.clone(),
                    subject_ref,
                },
                proposed_dispatch: ProposedDispatch {
                    product_slug: "repo-reaper".into(),
                    action_id: "run".into(),
                    input: serde_json::json!({
                        "target_selection_mode": "direct",
                        "target_repo": repository,
                        "max_repos": 1,
                        "max_issues": 1,
                    }),
                },
                rationale: format!(
                    "SignalHive identified {} as concrete maintenance work for this mandate.",
                    repo.get("full_name")
                        .and_then(Value::as_str)
                        .unwrap_or("the repository")
                ),
                evidence: repo.clone(),
            })
        })
        .collect()
}

fn required(field: &str, value: String, max_len: usize) -> Result<String, String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max_len {
        return Err(format!("{field} must be at most {max_len} characters"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(repository: &str, product: &str) -> ProposeWorkRequest {
        ProposeWorkRequest {
            mandate_id: None,
            identity: WorkIdentity {
                kind: " GitHub-Issue ".into(),
                repository: repository.into(),
                subject_ref: "issue:42".into(),
            },
            proposed_dispatch: ProposedDispatch {
                product_slug: product.into(),
                action_id: "analyze".into(),
                input: json!({"repository": repository}),
            },
            origin: WorkOrigin::Operator,
            rationale: "Worth assessing".into(),
        }
    }

    #[test]
    fn fingerprint_converges_across_case_and_proposed_products() {
        let first = WorkProposal::from_request(request("NousResearch/Hermes-Agent", "signal-hive"))
            .expect("valid proposal");
        let second =
            WorkProposal::from_request(request("nousresearch/hermes-agent", "repo-reaper"))
                .expect("valid proposal");
        assert_eq!(first.identity.fingerprint(), second.identity.fingerprint());
    }

    #[test]
    fn malformed_or_future_lifecycle_is_unknown() {
        let lifecycle = WorkLifecycle::from_storage(
            "ready".into(),
            json!({"state": "ready", "ready_at": "later"}),
        );
        assert!(matches!(lifecycle, WorkLifecycle::Unknown { .. }));
    }

    #[test]
    fn proposal_rejects_non_object_dispatch_input() {
        let mut value = request("owner/repo", "signal-hive");
        value.proposed_dispatch.input = json!(["not", "an", "object"]);
        assert_eq!(
            WorkProposal::from_request(value).expect_err("must reject array input"),
            "proposed dispatch input must be a JSON object"
        );
    }

    #[test]
    fn finding_batch_rejects_duplicate_sources() {
        let finding = ProductFinding {
            mandate_id: None,
            source: FindingSource {
                product_slug: "signal-hive".into(),
                run_id: "scan-1".into(),
                finding_id: "issue-42".into(),
            },
            identity: request("owner/repo", "repo-reaper").identity,
            proposed_dispatch: ProposedDispatch {
                product_slug: "repo-reaper".into(),
                action_id: "run".into(),
                input: json!({"target_repo": "owner/repo"}),
            },
            rationale: "Concrete issue".into(),
            evidence: json!({"issue_number": 42}),
        };
        let error = IngestFindingsRequest {
            findings: vec![finding.clone(), finding],
        }
        .validated()
        .expect_err("duplicate source should be rejected");
        assert_eq!(
            error,
            "a finding source may appear only once in an ingestion batch"
        );
    }

    #[test]
    fn finding_requires_structured_evidence() {
        let finding = ProductFinding {
            mandate_id: None,
            source: FindingSource {
                product_slug: "signal-hive".into(),
                run_id: "scan-1".into(),
                finding_id: "issue-42".into(),
            },
            identity: request("owner/repo", "repo-reaper").identity,
            proposed_dispatch: ProposedDispatch {
                product_slug: "repo-reaper".into(),
                action_id: "run".into(),
                input: json!({"target_repo": "owner/repo"}),
            },
            rationale: "Concrete issue".into(),
            evidence: json!("not structured"),
        };
        assert_eq!(
            IngestFindingsRequest {
                findings: vec![finding]
            }
            .validated()
            .expect_err("string evidence should be rejected"),
            "finding evidence must be a JSON object"
        );
    }

    #[test]
    fn finding_fingerprint_is_stable_across_json_object_order() {
        let base = ProductFinding {
            mandate_id: None,
            source: FindingSource {
                product_slug: "signal-hive".into(),
                run_id: "scan-1".into(),
                finding_id: "issue-42".into(),
            },
            identity: request("owner/repo", "repo-reaper").identity,
            proposed_dispatch: ProposedDispatch {
                product_slug: "repo-reaper".into(),
                action_id: "run".into(),
                input: json!({"target_repo": "owner/repo"}),
            },
            rationale: "Concrete issue".into(),
            evidence: serde_json::from_str(r#"{"a":1,"b":2}"#).expect("valid JSON"),
        };
        let mut reordered = base.clone();
        reordered.evidence =
            serde_json::from_str(r#"{"b":2,"a":1}"#).expect("valid reordered JSON");
        assert_eq!(base.fingerprint(), reordered.fingerprint());
    }
}
