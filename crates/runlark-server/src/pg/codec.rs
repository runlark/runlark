use crate::error::ServerError;
use runlark_domain::{
    ArtifactDigest, ArtifactKind, ArtifactRef, Attempt, AttemptId, AttemptOutcome, BudgetPolicy,
    CommentId, Decision, DecisionId, DecisionKind, InvitationStatus, InviteId, LeaseEpoch,
    PolicyDigest, ProjectId, Revision, Run, RunId, RunOwnership, RunStatus, RunnerId,
    RunnerRegistration, RunnerStatus, SessionId, Task, TaskComment, TaskId, TaskKind, TaskStage,
    TeamProjectSpec, Timestamp, UserId, WorkflowStage, WorkspaceId, WorkspaceInvitation,
    WorkspaceMember, WorkspaceRole,
};
use tokio_postgres::Row;

pub(crate) fn task_kind_to_str(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::QuickFix => "quick_fix",
        TaskKind::Standard => "standard",
    }
}

pub(crate) fn str_to_task_kind(raw: &str) -> Result<TaskKind, ServerError> {
    match raw {
        "quick_fix" => Ok(TaskKind::QuickFix),
        "standard" => Ok(TaskKind::Standard),
        other => Err(ServerError::BadRequest(format!(
            "unknown task_kind: {other}"
        ))),
    }
}

pub(crate) fn task_stage_to_str(stage: TaskStage) -> &'static str {
    match stage {
        TaskStage::Inbox => "inbox",
        TaskStage::Ready => "ready",
        TaskStage::InProgress => "in_progress",
        TaskStage::Acceptance => "acceptance",
        TaskStage::Done => "done",
    }
}

pub(crate) fn str_to_task_stage(raw: &str) -> Result<TaskStage, ServerError> {
    match raw {
        "inbox" => Ok(TaskStage::Inbox),
        "ready" => Ok(TaskStage::Ready),
        "in_progress" => Ok(TaskStage::InProgress),
        "acceptance" => Ok(TaskStage::Acceptance),
        "done" => Ok(TaskStage::Done),
        other => Err(ServerError::BadRequest(format!(
            "unknown task_stage: {other}"
        ))),
    }
}

pub(crate) fn workflow_stage_to_str(stage: WorkflowStage) -> &'static str {
    match stage {
        WorkflowStage::Clarification => "clarification",
        WorkflowStage::Plan => "plan",
        WorkflowStage::Implementation => "implementation",
        WorkflowStage::Verification => "verification",
        WorkflowStage::HumanAcceptance => "human_acceptance",
    }
}

pub(crate) fn str_to_workflow_stage(raw: &str) -> Result<WorkflowStage, ServerError> {
    match raw {
        "clarification" => Ok(WorkflowStage::Clarification),
        "plan" => Ok(WorkflowStage::Plan),
        "implementation" => Ok(WorkflowStage::Implementation),
        "verification" => Ok(WorkflowStage::Verification),
        "human_acceptance" => Ok(WorkflowStage::HumanAcceptance),
        other => Err(ServerError::BadRequest(format!(
            "unknown workflow_stage: {other}"
        ))),
    }
}

pub(crate) fn run_status_to_str(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::WaitingForApproval => "waiting_for_approval",
        RunStatus::CancelRequested => "cancel_requested",
        RunStatus::Stopped => "stopped",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Interrupted => "interrupted",
    }
}

pub(crate) fn str_to_run_status(raw: &str) -> Result<RunStatus, ServerError> {
    match raw {
        "queued" => Ok(RunStatus::Queued),
        "running" => Ok(RunStatus::Running),
        "waiting_for_approval" => Ok(RunStatus::WaitingForApproval),
        "cancel_requested" => Ok(RunStatus::CancelRequested),
        "stopped" => Ok(RunStatus::Stopped),
        "completed" => Ok(RunStatus::Completed),
        "failed" => Ok(RunStatus::Failed),
        "interrupted" => Ok(RunStatus::Interrupted),
        other => Err(ServerError::BadRequest(format!(
            "unknown run_status: {other}"
        ))),
    }
}

pub(crate) fn decision_kind_to_str(kind: DecisionKind) -> &'static str {
    match kind {
        DecisionKind::Accept => "accept",
        DecisionKind::RequestChanges => "request_changes",
        DecisionKind::ApprovePlan => "approve_plan",
    }
}

pub(crate) fn str_to_decision_kind(raw: &str) -> Result<DecisionKind, ServerError> {
    match raw {
        "accept" => Ok(DecisionKind::Accept),
        "request_changes" => Ok(DecisionKind::RequestChanges),
        "approve_plan" => Ok(DecisionKind::ApprovePlan),
        other => Err(ServerError::BadRequest(format!(
            "unknown decision_kind: {other}"
        ))),
    }
}

pub(crate) fn artifact_kind_to_str(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Diff => "diff",
        ArtifactKind::VerificationLog => "verification_log",
        ArtifactKind::WasmValidation => "wasm_validation",
        ArtifactKind::Plan => "plan",
        ArtifactKind::EvidenceBundle => "evidence_bundle",
    }
}

pub(crate) fn str_to_artifact_kind(raw: &str) -> Result<ArtifactKind, ServerError> {
    match raw {
        "diff" => Ok(ArtifactKind::Diff),
        "verification_log" => Ok(ArtifactKind::VerificationLog),
        "wasm_validation" => Ok(ArtifactKind::WasmValidation),
        "plan" => Ok(ArtifactKind::Plan),
        "evidence_bundle" => Ok(ArtifactKind::EvidenceBundle),
        other => Err(ServerError::BadRequest(format!(
            "unknown artifact_kind: {other}"
        ))),
    }
}

pub(crate) fn str_to_invitation_status(raw: &str) -> Result<InvitationStatus, ServerError> {
    match raw {
        "pending" => Ok(InvitationStatus::Pending),
        "accepted" => Ok(InvitationStatus::Accepted),
        "revoked" => Ok(InvitationStatus::Revoked),
        "expired" => Ok(InvitationStatus::Expired),
        other => Err(ServerError::BadRequest(format!(
            "unknown invitation_status: {other}"
        ))),
    }
}

pub(crate) fn row_to_member(row: &Row) -> Result<WorkspaceMember, ServerError> {
    let role_str: String = row.get("role");
    let role = WorkspaceRole::from_str_name(&role_str)
        .ok_or_else(|| ServerError::BadRequest(format!("unknown workspace role: {role_str}")))?;
    Ok(WorkspaceMember {
        workspace_id: WorkspaceId::from(row.get::<_, String>("workspace_id")),
        user_id: UserId::from(row.get::<_, String>("user_id")),
        display_name: row.get("display_name"),
        role,
        joined_at: Timestamp(ServerError::from_pg_i64(row.get("joined_at"), "joined_at")?),
    })
}

pub(crate) fn row_to_invitation(row: &Row) -> Result<WorkspaceInvitation, ServerError> {
    let role_str: String = row.get("role");
    let role = WorkspaceRole::from_str_name(&role_str)
        .ok_or_else(|| ServerError::BadRequest(format!("unknown invitation role: {role_str}")))?;
    let status_str: String = row.get("status");
    let accepted_at: Option<i64> = row.get("accepted_at");
    let accepted_by: Option<String> = row.get("accepted_by");
    Ok(WorkspaceInvitation {
        id: InviteId::from(row.get::<_, String>("invite_id")),
        workspace_id: WorkspaceId::from(row.get::<_, String>("workspace_id")),
        invitee_hint: row.get("invitee_hint"),
        role,
        status: str_to_invitation_status(&status_str)?,
        invited_by: UserId::from(row.get::<_, String>("invited_by")),
        expires_at: Timestamp(ServerError::from_pg_i64(
            row.get("expires_at"),
            "expires_at",
        )?),
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
        accepted_at: accepted_at
            .map(|v| ServerError::from_pg_i64(v, "accepted_at").map(Timestamp))
            .transpose()?,
        accepted_by: accepted_by.map(UserId::from),
    })
}

pub(crate) fn row_to_runner(row: &Row) -> Result<RunnerRegistration, ServerError> {
    let status_str: String = row.get("status");
    let status = RunnerStatus::from_str_name(&status_str)
        .ok_or_else(|| ServerError::BadRequest(format!("unknown runner status: {status_str}")))?;
    Ok(RunnerRegistration {
        id: RunnerId::from(row.get::<_, String>("runner_id")),
        workspace_id: WorkspaceId::from(row.get::<_, String>("workspace_id")),
        owner_user_id: UserId::from(row.get::<_, String>("owner_user_id")),
        name: row.get("name"),
        status,
        last_heartbeat_at: Timestamp(ServerError::from_pg_i64(
            row.get("last_heartbeat_at"),
            "last_heartbeat_at",
        )?),
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
    })
}

pub(crate) fn row_to_project(row: &Row) -> Result<TeamProjectSpec, ServerError> {
    let budget_json: String = row.get("budget_policy_json");
    let budget_policy: BudgetPolicy = serde_json::from_str(&budget_json)?;
    Ok(TeamProjectSpec {
        project_id: ProjectId::from(row.get::<_, String>("project_id")),
        workspace_id: WorkspaceId::from(row.get::<_, String>("workspace_id")),
        name: row.get("name"),
        default_branch: row.get("default_branch"),
        max_concurrent_runs: ServerError::from_pg_i32(
            row.get("max_concurrent_runs"),
            "max_concurrent_runs",
        )?,
        budget_policy,
        policy_digest: PolicyDigest::from(row.get::<_, String>("policy_digest")),
        ownership_epoch: ServerError::from_pg_i64(row.get("ownership_epoch"), "ownership_epoch")?,
        revision: Revision(ServerError::from_pg_i64(row.get("revision"), "revision")?),
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
        updated_at: Timestamp(ServerError::from_pg_i64(
            row.get("updated_at"),
            "updated_at",
        )?),
    })
}

pub(crate) fn row_to_task(row: &Row) -> Result<Task, ServerError> {
    let kind_str: String = row.get("kind");
    let stage_str: String = row.get("stage");
    let wf_str: String = row.get("workflow_stage");
    let budget_json: Option<String> = row.get("budget_override_json");
    let budget_override = budget_json
        .as_deref()
        .map(serde_json::from_str::<BudgetPolicy>)
        .transpose()?;

    Ok(Task {
        id: TaskId::from(row.get::<_, String>("task_id")),
        project_id: ProjectId::from(row.get::<_, String>("project_id")),
        title: row.get("title"),
        description: row.get("description"),
        kind: str_to_task_kind(&kind_str)?,
        stage: str_to_task_stage(&stage_str)?,
        workflow_stage: str_to_workflow_stage(&wf_str)?,
        plan_text: row.get("plan_text"),
        plan_version: ServerError::from_pg_i32(row.get("plan_version"), "plan_version")?,
        base_ref: row.get("base_ref"),
        head_ref: row.get("head_ref"),
        pr_url: row.get("pr_url"),
        active_session_id: row
            .get::<_, Option<String>>("active_session_id")
            .map(SessionId::from),
        active_run_id: row
            .get::<_, Option<String>>("active_run_id")
            .map(RunId::from),
        latest_run_id: row
            .get::<_, Option<String>>("latest_run_id")
            .map(RunId::from),
        budget_override,
        revision: Revision(ServerError::from_pg_i64(row.get("revision"), "revision")?),
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
        updated_at: Timestamp(ServerError::from_pg_i64(
            row.get("updated_at"),
            "updated_at",
        )?),
    })
}

pub(crate) fn row_to_run(row: &Row) -> Result<(Run, RunOwnership), ServerError> {
    let status_str: String = row.get("status");
    let wf_str: String = row.get("workflow_stage");
    let kind_str: String = row.get("task_kind");
    let wf_digest: Option<String> = row.get("workflow_digest");
    let budget_json: String = row.get("budget_json");
    let cost_json: String = row.get("cost_summary_json");
    let assigned_runner: Option<String> = row.get("assigned_runner_id");

    let run = Run {
        id: RunId::from(row.get::<_, String>("run_id")),
        session_id: SessionId::from(row.get::<_, String>("session_id")),
        status: str_to_run_status(&status_str)?,
        worktree_path: String::new(),
        revision: Revision(ServerError::from_pg_i64(row.get("revision"), "revision")?),
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
        updated_at: Timestamp(ServerError::from_pg_i64(
            row.get("updated_at"),
            "updated_at",
        )?),
        task_id: Some(TaskId::from(row.get::<_, String>("task_id"))),
        project_id: Some(ProjectId::from(row.get::<_, String>("project_id"))),
        workflow_stage: str_to_workflow_stage(&wf_str)?,
        task_kind: str_to_task_kind(&kind_str)?,
        workflow_digest: wf_digest.map(ArtifactDigest::parse).transpose()?,
        base_sha: row.get("base_sha"),
        head_sha: row.get("head_sha"),
        plan_version: ServerError::from_pg_i32(row.get("plan_version"), "plan_version")?,
        policy_digest: row
            .get::<_, Option<String>>("policy_digest")
            .map(PolicyDigest::from),
        budget_policy: serde_json::from_str(&budget_json)?,
        cost_summary: serde_json::from_str(&cost_json)?,
        repair_attempts: ServerError::from_pg_i32(row.get("repair_attempts"), "repair_attempts")?,
        pr_url: row.get("pr_url"),
    };

    let ownership = RunOwnership {
        assigned_runner_id: assigned_runner.map(RunnerId::from),
        lease_epoch: LeaseEpoch(ServerError::from_pg_i64(
            row.get("lease_epoch"),
            "lease_epoch",
        )?),
        runner_unreachable: row.get("runner_unreachable"),
    };

    Ok((run, ownership))
}

pub(crate) fn row_to_attempt(row: &Row) -> Result<Attempt, ServerError> {
    let wf_str: String = row.get("workflow_stage");
    let outcome_json: String = row.get("outcome_json");
    let ended_at: Option<i64> = row.get("ended_at");
    let cost_micro_usd: Option<i64> = row.get("cost_micro_usd");

    Ok(Attempt {
        id: AttemptId::from(row.get::<_, String>("attempt_id")),
        run_id: RunId::from(row.get::<_, String>("run_id")),
        attempt_no: ServerError::from_pg_i32(row.get("attempt_no"), "attempt_no")?,
        workflow_stage: str_to_workflow_stage(&wf_str)?,
        started_at: Timestamp(ServerError::from_pg_i64(
            row.get("started_at"),
            "started_at",
        )?),
        ended_at: ended_at
            .map(|v| ServerError::from_pg_i64(v, "ended_at").map(Timestamp))
            .transpose()?,
        outcome: serde_json::from_str::<AttemptOutcome>(&outcome_json)?,
        cost_micro_usd: cost_micro_usd
            .map(|v| ServerError::from_pg_i64(v, "cost_micro_usd"))
            .transpose()?,
        unknown_cost_reason: row.get("unknown_cost_reason"),
        repair_reason: row.get("repair_reason"),
    })
}

pub(crate) fn row_to_decision(row: &Row) -> Result<Decision, ServerError> {
    let kind_str: String = row.get("kind");
    Ok(Decision {
        id: DecisionId::from(row.get::<_, String>("decision_id")),
        task_id: TaskId::from(row.get::<_, String>("task_id")),
        run_id: RunId::from(row.get::<_, String>("run_id")),
        kind: str_to_decision_kind(&kind_str)?,
        actor: row.get("actor"),
        plan_version: ServerError::from_pg_i32(row.get("plan_version"), "plan_version")?,
        head_sha: row.get("head_sha"),
        base_sha: row.get("base_sha"),
        policy_digest: PolicyDigest::from(row.get::<_, String>("policy_digest")),
        evidence_digest: ArtifactDigest::parse(row.get::<_, String>("evidence_digest"))?,
        comment: row.get("comment"),
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
    })
}

pub(crate) fn row_to_comment(row: &Row) -> Result<TaskComment, ServerError> {
    Ok(TaskComment {
        id: CommentId::from(row.get::<_, String>("comment_id")),
        task_id: TaskId::from(row.get::<_, String>("task_id")),
        run_id: row.get::<_, Option<String>>("run_id").map(RunId::from),
        author_user_id: UserId::from(row.get::<_, String>("author_user_id")),
        author_display_name: row.get("author_display_name"),
        body: row.get("body"),
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
    })
}

pub(crate) fn row_to_artifact_ref(row: &Row) -> Result<ArtifactRef, ServerError> {
    let kind_str: String = row.get("kind");
    Ok(ArtifactRef {
        digest_sha256: ArtifactDigest::parse(row.get::<_, String>("digest_sha256"))?,
        kind: str_to_artifact_kind(&kind_str)?,
        media_type: row.get("media_type"),
        size_bytes: ServerError::from_pg_i64(row.get("size_bytes"), "size_bytes")?,
        created_at: Timestamp(ServerError::from_pg_i64(
            row.get("created_at"),
            "created_at",
        )?),
    })
}
