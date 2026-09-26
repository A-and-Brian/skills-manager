//! The report structs printed by each command; their JSON is what `--json` callers parse.

use app_lib::core::scenario_service;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct RepoStatus {
    pub(crate) base_dir: String,
    pub(crate) skills_dir: String,
    pub(crate) db_path: String,
    pub(crate) metadata_dir: String,
    pub(crate) skill_count: usize,
    pub(crate) preset_count: usize,
    pub(crate) active_preset_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) path: String,
    pub(crate) enabled: bool,
    pub(crate) tags: Vec<String>,
    pub(crate) source_type: String,
    pub(crate) source_ref: Option<String>,
    pub(crate) preset_ids: Vec<String>,
    pub(crate) presets: Vec<String>,
    pub(crate) deployed_to: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentMutationReport {
    pub(crate) agent: String,
    pub(crate) enabled: bool,
    pub(crate) changed: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillAgentStatus {
    pub(crate) key: String,
    pub(crate) display_name: String,
    pub(crate) installed: bool,
    pub(crate) globally_enabled: bool,
    pub(crate) deployed: bool,
    pub(crate) target_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillStatusReport {
    #[serde(flatten)]
    pub(crate) skill: SkillSummary,
    pub(crate) agents: Vec<SkillAgentStatus>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillDeploymentReport {
    pub(crate) ok: bool,
    pub(crate) action: String,
    pub(crate) agents: Vec<String>,
    pub(crate) dry_run: bool,
    pub(crate) skill_count: usize,
    pub(crate) pair_count: usize,
    pub(crate) changed_pairs: usize,
    pub(crate) skills: Vec<String>,
    /// Paths left in place because they no longer match the deployment we
    /// recorded — someone else's content now lives there (#363).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) preserved: Vec<String>,
}

pub(crate) struct DeploymentVerification {
    pub(crate) succeeded: std::collections::HashSet<(String, String)>,
    pub(crate) failures: Vec<String>,
    pub(crate) preserved: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkillDetail {
    #[serde(flatten)]
    pub(crate) summary: SkillSummary,
    pub(crate) skill_file: String,
    pub(crate) files: Vec<String>,
    pub(crate) markdown: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresetInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) sort_order: i32,
    pub(crate) skill_count: usize,
    pub(crate) active: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresetAgentStatus {
    pub(crate) key: String,
    pub(crate) display_name: String,
    pub(crate) deployed: usize,
    pub(crate) total: usize,
    pub(crate) status: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresetStatusReport {
    pub(crate) preset: PresetInfo,
    pub(crate) agents: Vec<PresetAgentStatus>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresetDeploymentReport {
    pub(crate) ok: bool,
    pub(crate) action: String,
    pub(crate) preset_id: String,
    pub(crate) preset_name: String,
    pub(crate) agents: Vec<String>,
    pub(crate) dry_run: bool,
    pub(crate) skill_count: usize,
    pub(crate) pair_count: usize,
    pub(crate) changed_pairs: usize,
    /// See [`SkillDeploymentReport::preserved`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) preserved: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresetDeleteReport {
    pub(crate) ok: bool,
    pub(crate) preset_id: String,
    pub(crate) preset_name: String,
    pub(crate) dry_run: bool,
    pub(crate) deleted: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct InstallReport {
    pub(crate) ok: bool,
    pub(crate) skill_id: String,
    pub(crate) name: String,
    pub(crate) central_path: String,
    pub(crate) source_type: String,
    pub(crate) synced: bool,
    pub(crate) preset_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UpdateReport {
    pub(crate) skill_id: String,
    pub(crate) name: String,
    pub(crate) source_type: String,
    pub(crate) refreshed: bool,
    pub(crate) error: Option<String>,
    /// Present when the update was held back because it would have removed
    /// these paths (#256). Nothing changed, and the CLI offers no way to
    /// accept: approving means seeing the list, which needs a person, so it
    /// only exists in the app. A bare `refreshed: false` would read as
    /// "already up to date".
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub(crate) held_back_removals: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CheckReport {
    pub(crate) skill_id: String,
    pub(crate) name: String,
    pub(crate) source_type: String,
    pub(crate) update_status: String,
    pub(crate) last_check_error: Option<String>,
    pub(crate) skipped: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct RemoveReport {
    pub(crate) ok: bool,
    pub(crate) deleted: usize,
    pub(crate) failed: Vec<String>,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeprecatedEnableReport {
    pub(crate) skill_id: String,
    pub(crate) name: String,
    pub(crate) enabled: bool,
    pub(crate) changed: bool,
    pub(crate) deprecated: bool,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SyncReport {
    pub(crate) ok: bool,
    pub(crate) preset_id: String,
    pub(crate) preset_name: String,
    pub(crate) tool: Option<String>,
    pub(crate) dry_run: bool,
    pub(crate) targets: Vec<scenario_service::SyncPreviewTarget>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresetDeactivateReport {
    pub(crate) ok: bool,
    pub(crate) preset_id: String,
    pub(crate) preset_name: String,
    pub(crate) removed_target_count: usize,
    pub(crate) active_preset_id: Option<String>,
    pub(crate) active_preset_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchHit {
    pub(crate) install_ref: String,
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) skill_id: String,
    pub(crate) installs: u64,
    pub(crate) skills_sh_url: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdoptCandidate {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdoptReport {
    pub(crate) ok: bool,
    pub(crate) dry_run: bool,
    pub(crate) adopted: Vec<InstallReport>,
    pub(crate) candidates: Vec<AdoptCandidate>,
    pub(crate) skipped: Vec<AdoptCandidate>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TagReport {
    pub(crate) skill_id: String,
    pub(crate) name: String,
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct GlobalTagReport {
    pub(crate) ok: bool,
    pub(crate) tag: String,
    pub(crate) renamed_to: Option<String>,
    pub(crate) affected_skills: usize,
    pub(crate) dry_run: bool,
    pub(crate) deleted: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresetMembershipReport {
    pub(crate) preset_id: String,
    pub(crate) preset_name: String,
    pub(crate) added: Vec<String>,
    pub(crate) removed: Vec<String>,
    pub(crate) missing: Vec<String>,
}
