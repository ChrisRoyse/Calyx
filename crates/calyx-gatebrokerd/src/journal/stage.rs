use serde::{Deserialize, Serialize};

use super::db::{DbRow, QueryExecutor, db_params};
use super::*;
use crate::protocol::{
    AbsolutePath, InvocationId, RequestId, RunId, StageId, StageLabel, UnitName,
};

#[derive(Debug, Clone)]
pub struct StageIntent {
    pub stage_id: StageId,
    pub request_id: RequestId,
    pub run_id: RunId,
    pub label: StageLabel,
    /// Planned service identity, persisted before any systemd side effect.
    pub unit: UnitName,
    /// Fixed dedicated containment boundary, persisted before launch.
    pub slice_unit: UnitName,
    /// Account name and numeric ID are both frozen before launch so account
    /// database reassignment cannot redirect restart recovery.
    pub worker_user: String,
    pub worker_uid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedCgroupIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug)]
pub struct StageRunningEvidence<'a> {
    pub stage_id: &'a StageId,
    pub expected_unit: &'a UnitName,
    pub expected_slice_unit: &'a UnitName,
    pub invocation_id: &'a InvocationId,
    pub control_group: &'a AbsolutePath,
    pub slice_control_group: &'a AbsolutePath,
    pub control_group_identity: RecordedCgroupIdentity,
    pub slice_control_group_identity: RecordedCgroupIdentity,
    pub main_pid: u32,
}

#[derive(Debug, Clone)]
pub struct StageRecord {
    pub intent: StageIntent,
    pub state: StageState,
    pub invocation_id: Option<InvocationId>,
    pub control_group: Option<AbsolutePath>,
    pub slice_control_group: Option<AbsolutePath>,
    pub control_group_identity: Option<RecordedCgroupIdentity>,
    pub slice_control_group_identity: Option<RecordedCgroupIdentity>,
    pub main_pid: Option<u32>,
    pub exit_status: Option<i32>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    Intent,
    Running,
    Succeeded,
    Failed,
}

impl StageState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, JournalError> {
        match value {
            "intent" => Ok(Self::Intent),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            _ => Err(JournalError::Corrupt(format!(
                "unknown stage state {value:?}"
            ))),
        }
    }
}

pub(super) fn stage_transition_allowed(from: StageState, to: StageState) -> bool {
    matches!(
        (from, to),
        (StageState::Intent, StageState::Running | StageState::Failed)
            | (
                StageState::Running,
                StageState::Succeeded | StageState::Failed
            )
    )
}

impl Journal {
    pub fn begin_stage(&mut self, intent: &StageIntent) -> Result<(), JournalError> {
        validate_worker_identity(&intent.worker_user, intent.worker_uid)?;
        let now = now_ms()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| sql("begin stage intent", source))?;
        require_active_run(&transaction, &intent.run_id, "stage intent")?;
        transaction
            .execute(
                "INSERT INTO stages(stage_id,request_id,run_id,label,state,unit,slice_unit,worker_user,worker_uid,created_ms,updated_ms) VALUES(?1,?2,?3,?4,'intent',?5,?6,?7,?8,?9,?9)",
                db_params![
                    intent.stage_id.as_str(),
                    intent.request_id.as_str(),
                    intent.run_id.as_str(),
                    intent.label.as_str(),
                    intent.unit.as_str(),
                    intent.slice_unit.as_str(),
                    intent.worker_user.as_str(),
                    intent.worker_uid,
                    now,
                ]?,
            )
            .map_err(|source| sql("insert stage intent", source))?;
        transaction
            .execute(
                "INSERT INTO stage_events(stage_id,to_state,at_ms) VALUES(?1,'intent',?2)",
                db_params![intent.stage_id.as_str(), now]?,
            )
            .map_err(|source| sql("insert stage intent event", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit stage intent", source))
    }

    pub fn mark_stage_running(
        &mut self,
        evidence: &StageRunningEvidence<'_>,
    ) -> Result<(), JournalError> {
        if evidence.main_pid == 0 || evidence.main_pid > i32::MAX as u32 {
            return Err(JournalError::InvalidMetadata(
                "stage main pid is outside the Linux pid range".into(),
            ));
        }
        let now = now_ms()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| sql("begin running stage transition", source))?;
        let changed = transaction
            .execute(
                "UPDATE stages SET state='running',invocation_id=?1,control_group=?2,slice_control_group=?3,control_group_device=?4,control_group_inode=?5,slice_control_group_device=?6,slice_control_group_inode=?7,main_pid=?8,updated_ms=?9 WHERE stage_id=?10 AND state='intent' AND unit=?11 AND slice_unit=?12",
                db_params![
                    evidence.invocation_id.as_str(),
                    evidence.control_group.as_str(),
                    evidence.slice_control_group.as_str(),
                    evidence.control_group_identity.device.to_string(),
                    evidence.control_group_identity.inode.to_string(),
                    evidence.slice_control_group_identity.device.to_string(),
                    evidence.slice_control_group_identity.inode.to_string(),
                    evidence.main_pid,
                    now,
                    evidence.stage_id.as_str(),
                    evidence.expected_unit.as_str(),
                    evidence.expected_slice_unit.as_str(),
                ]?,
            )
            .map_err(|source| sql("mark stage running", source))?;
        require_stage_change(changed, evidence.stage_id, "intent -> running")?;
        transaction
            .execute(
                "INSERT INTO stage_events(stage_id,from_state,to_state,at_ms) VALUES(?1,'intent','running',?2)",
                db_params![evidence.stage_id.as_str(), now]?,
            )
            .map_err(|source| sql("insert running stage event", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit running stage", source))
    }

    pub fn finish_stage(
        &mut self,
        stage_id: &StageId,
        exit_status: i32,
    ) -> Result<StageState, JournalError> {
        let next = if exit_status == 0 {
            StageState::Succeeded
        } else {
            StageState::Failed
        };
        let now = now_ms()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| sql("begin finished stage transition", source))?;
        let changed = transaction
            .execute(
                "UPDATE stages SET state=?1,exit_status=?2,updated_ms=?3 WHERE stage_id=?4 AND state='running'",
                db_params![next.as_db(), exit_status, now, stage_id.as_str()]?,
            )
            .map_err(|source| sql("finish stage", source))?;
        require_stage_change(changed, stage_id, "running -> terminal")?;
        transaction
            .execute(
                "INSERT INTO stage_events(stage_id,from_state,to_state,detail,at_ms) VALUES(?1,'running',?2,?3,?4)",
                db_params![stage_id.as_str(), next.as_db(), format!("exit_status={exit_status}"), now]?,
            )
            .map_err(|source| sql("insert finished stage event", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit finished stage", source))?;
        Ok(next)
    }

    pub fn fail_stage_intent(
        &mut self,
        stage_id: &StageId,
        exit_status: i32,
        detail: &str,
    ) -> Result<(), JournalError> {
        if exit_status == 0 {
            return Err(JournalError::InvalidMetadata(
                "failed stage intent requires a nonzero exit status".into(),
            ));
        }
        validate_optional_text("stage failure detail", Some(detail), 2_048)?;
        let now = now_ms()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| sql("begin failed stage-intent transition", source))?;
        let changed = transaction
            .execute(
                "UPDATE stages SET state='failed',exit_status=?1,updated_ms=?2 WHERE stage_id=?3 AND state='intent'",
                db_params![exit_status, now, stage_id.as_str()]?,
            )
            .map_err(|source| sql("fail stage intent", source))?;
        require_stage_change(changed, stage_id, "intent -> failed")?;
        transaction
            .execute(
                "INSERT INTO stage_events(stage_id,from_state,to_state,detail,at_ms) VALUES(?1,'intent','failed',?2,?3)",
                db_params![stage_id.as_str(), detail, now]?,
            )
            .map_err(|source| sql("insert failed stage-intent event", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit failed stage intent", source))
    }

    pub fn get_stage(&self, stage_id: &StageId) -> Result<Option<StageRecord>, JournalError> {
        self.connection
            .query_optional(
                "SELECT request_id,run_id,label,state,unit,slice_unit,worker_user,worker_uid,invocation_id,control_group,slice_control_group,control_group_device,control_group_inode,slice_control_group_device,slice_control_group_inode,main_pid,exit_status,created_ms,updated_ms FROM stages WHERE stage_id=?1",
                db_params![stage_id.as_str()]?,
            )
            .map_err(|source| sql("read stage", source))?
            .map(|row| stage_from_row(stage_id.clone(), &row))
            .transpose()
    }

    pub fn list_incomplete_stages(&self) -> Result<Vec<StageRecord>, JournalError> {
        let ids = self
            .connection
            .query(
                "SELECT stage_id FROM stages WHERE state IN ('intent','running') ORDER BY created_ms,stage_id",
                db_params!()?,
            )
            .map_err(|source| sql("query incomplete stages", source))?
            .into_iter()
            .map(|row| row.get::<String>(0).map_err(|source| sql("read incomplete stage id", source)))
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|value| {
                let id = StageId::new(value)
                    .map_err(|error| JournalError::Corrupt(error.to_string()))?;
                self.get_stage(&id)?.ok_or_else(|| {
                    JournalError::Corrupt(format!("incomplete stage {id} disappeared"))
                })
            })
            .collect()
    }

    pub fn list_terminal_stages(&self) -> Result<Vec<StageRecord>, JournalError> {
        let ids = self
            .connection
            .query(
                "SELECT stage_id FROM stages WHERE state IN ('succeeded','failed') ORDER BY created_ms,stage_id",
                db_params!()?,
            )
            .map_err(|source| sql("query terminal stages", source))?
            .into_iter()
            .map(|row| row.get::<String>(0).map_err(|source| sql("read terminal stage id", source)))
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|value| {
                let id = StageId::new(value).map_err(corrupt_protocol)?;
                self.get_stage(&id)?.ok_or_else(|| {
                    JournalError::Corrupt(format!("terminal stage {id} disappeared"))
                })
            })
            .collect()
    }

    pub fn get_stage_by_request(
        &self,
        request_id: &RequestId,
    ) -> Result<Option<StageRecord>, JournalError> {
        let value_row = self
            .connection
            .query_optional(
                "SELECT stage_id FROM stages WHERE request_id=?1",
                db_params![request_id.as_str()]?,
            )
            .map_err(|source| sql("find stage by request", source))?;
        let value = value_row
            .map(|row| {
                row.get::<String>(0)
                    .map_err(|source| sql("decode stage by request", source))
            })
            .transpose()?;
        value
            .map(|value| StageId::new(value).map_err(corrupt_protocol))
            .transpose()?
            .map(|stage_id| self.get_stage(&stage_id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn list_stages_for_run(&self, run_id: &RunId) -> Result<Vec<StageRecord>, JournalError> {
        let ids = self
            .connection
            .query(
                "SELECT stage_id FROM stages WHERE run_id=?1 ORDER BY created_ms,stage_id",
                db_params![run_id.as_str()]?,
            )
            .map_err(|source| sql("query run stages", source))?
            .into_iter()
            .map(|row| {
                row.get::<String>(0)
                    .map_err(|source| sql("read run stage id", source))
            })
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|value| {
                let id = StageId::new(value).map_err(corrupt_protocol)?;
                self.get_stage(&id)?
                    .ok_or_else(|| JournalError::Corrupt(format!("run stage {id} disappeared")))
            })
            .collect()
    }
}

fn stage_from_row(stage_id: StageId, row: &DbRow) -> Result<StageRecord, JournalError> {
    let request_id = RequestId::new(row.get::<String>(0).map_err(corrupt_sql)?);
    let run_id = RunId::new(row.get::<String>(1).map_err(corrupt_sql)?);
    let label = StageLabel::new(row.get::<String>(2).map_err(corrupt_sql)?);
    let state = StageState::parse(&row.get::<String>(3).map_err(corrupt_sql)?);
    let unit = UnitName::new(row.get::<String>(4).map_err(corrupt_sql)?);
    let slice_unit = UnitName::new(row.get::<String>(5).map_err(corrupt_sql)?);
    let worker_user: String = row.get(6).map_err(corrupt_sql)?;
    let worker_uid: u32 = row.get(7).map_err(corrupt_sql)?;
    let invocation = row
        .get::<Option<String>>(8)
        .map_err(corrupt_sql)?
        .map(InvocationId::new)
        .transpose();
    let control_group = row
        .get::<Option<String>>(9)
        .map_err(corrupt_sql)?
        .map(AbsolutePath::new)
        .transpose();
    let slice_control_group = row
        .get::<Option<String>>(10)
        .map_err(corrupt_sql)?
        .map(AbsolutePath::new)
        .transpose();
    let control_group_identity = parse_cgroup_identity(row, 11, 12);
    let slice_control_group_identity = parse_cgroup_identity(row, 13, 14);
    (|| {
        let record = StageRecord {
            intent: StageIntent {
                stage_id,
                request_id: request_id.map_err(corrupt_protocol)?,
                run_id: run_id.map_err(corrupt_protocol)?,
                label: label.map_err(corrupt_protocol)?,
                unit: unit.map_err(corrupt_protocol)?,
                slice_unit: slice_unit.map_err(corrupt_protocol)?,
                worker_user,
                worker_uid,
            },
            state: state?,
            invocation_id: invocation.map_err(corrupt_protocol)?,
            control_group: control_group.map_err(corrupt_protocol)?,
            slice_control_group: slice_control_group.map_err(corrupt_protocol)?,
            control_group_identity: control_group_identity?,
            slice_control_group_identity: slice_control_group_identity?,
            main_pid: row.get(15).map_err(corrupt_sql)?,
            exit_status: row.get(16).map_err(corrupt_sql)?,
            created_ms: row.get(17).map_err(corrupt_sql)?,
            updated_ms: row.get(18).map_err(corrupt_sql)?,
        };
        validate_stage_record(&record)?;
        Ok(record)
    })()
}

fn validate_stage_record(record: &StageRecord) -> Result<(), JournalError> {
    let launch_metadata = (
        record.invocation_id.is_some(),
        record.control_group.is_some(),
        record.slice_control_group.is_some(),
        record.control_group_identity.is_some(),
        record.slice_control_group_identity.is_some(),
        record.main_pid.is_some(),
    );
    let no_launch_metadata = launch_metadata == (false, false, false, false, false, false);
    let all_launch_metadata = launch_metadata == (true, true, true, true, true, true);
    let valid = match record.state {
        StageState::Intent => no_launch_metadata && record.exit_status.is_none(),
        StageState::Running => all_launch_metadata && record.exit_status.is_none(),
        StageState::Succeeded => all_launch_metadata && record.exit_status == Some(0),
        StageState::Failed => {
            (no_launch_metadata || all_launch_metadata)
                && record.exit_status.is_some_and(|status| status != 0)
        }
    };
    if !valid {
        return Err(JournalError::Corrupt(format!(
            "stage {} state {:?} has inconsistent launch or exit metadata",
            record.intent.stage_id, record.state
        )));
    }
    Ok(())
}

fn validate_worker_identity(worker_user: &str, worker_uid: u32) -> Result<(), JournalError> {
    if worker_uid == 0
        || worker_user.is_empty()
        || worker_user.len() > 64
        || !worker_user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(JournalError::InvalidMetadata(
            "stage worker identity is not a non-root Linux account name/UID".into(),
        ));
    }
    Ok(())
}

fn parse_cgroup_identity(
    row: &DbRow,
    device_column: usize,
    inode_column: usize,
) -> Result<Option<RecordedCgroupIdentity>, JournalError> {
    let device: Option<String> = row.get(device_column).map_err(corrupt_sql)?;
    let inode: Option<String> = row.get(inode_column).map_err(corrupt_sql)?;
    match (device, inode) {
        (None, None) => Ok(None),
        (Some(device), Some(inode)) => Ok(Some(RecordedCgroupIdentity {
            device: device.parse().map_err(|error| {
                JournalError::Corrupt(format!("invalid cgroup device {device:?}: {error}"))
            })?,
            inode: inode.parse().map_err(|error| {
                JournalError::Corrupt(format!("invalid cgroup inode {inode:?}: {error}"))
            })?,
        })),
        _ => Err(JournalError::Corrupt(
            "cgroup device/inode pair is partially null".into(),
        )),
    }
}

fn require_stage_change(
    changed: u64,
    stage_id: &StageId,
    transition: &str,
) -> Result<(), JournalError> {
    if changed != 1 {
        return Err(JournalError::InvalidMetadata(format!(
            "stage {stage_id} is absent or not eligible for {transition}"
        )));
    }
    Ok(())
}
