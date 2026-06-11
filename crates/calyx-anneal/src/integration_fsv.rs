use calyx_aster::cf::full_content_hash;
use calyx_core::{CalyxError, Clock, Result};
use calyx_ledger::LedgerCfStore;
use serde::{Deserialize, Serialize};

use crate::shadow::AnnealAction as ShadowAnnealAction;
use crate::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, ArtifactKey, ArtifactPtr, BudgetEnforcer,
    BudgetProbe, ChangeId, HeldOutReplay, MetricSnapshot, ProcStatBudgetProbe, RollbackReadback,
    RollbackStorage, RollbackStore, ShadowExecutor, ShadowRevertReason, ShadowVerdict,
    TripwireRegistry, TripwireStatus,
};

pub const CALYX_LEDGER_WRITE_FAIL: &str = "CALYX_LEDGER_WRITE_FAIL";

const DEFAULT_SHADOW_CPU_WEIGHT: f64 = 0.01;
const DEFAULT_SHADOW_VRAM_BYTES: u64 = 0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChangeOutcome {
    Promoted(ChangeId),
    Reverted {
        reason: ShadowRevertReason,
        change_id: ChangeId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealStatus {
    pub tripwire_states: Vec<TripwireStatus>,
    pub budget: crate::BudgetStatus,
    pub recent_changes: Vec<AnnealLedgerEntry>,
}

pub struct AnnealSubstrate<'a, R, L, C, P = ProcStatBudgetProbe>
where
    R: RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: BudgetProbe,
{
    pub tripwires: TripwireRegistry,
    pub replay: HeldOutReplay,
    pub rollback: RollbackStore<'a, R>,
    pub ledger: AnnealLedger<L, C>,
    pub budget: BudgetEnforcer<'a, P>,
    clock: &'a dyn Clock,
    shadow_cpu_weight: f64,
    shadow_vram_bytes: u64,
}

impl<'a, R, L, C, P> AnnealSubstrate<'a, R, L, C, P>
where
    R: RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: BudgetProbe,
{
    pub fn new(
        tripwires: TripwireRegistry,
        replay: HeldOutReplay,
        rollback: RollbackStore<'a, R>,
        ledger: AnnealLedger<L, C>,
        budget: BudgetEnforcer<'a, P>,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            tripwires,
            replay,
            rollback,
            ledger,
            budget,
            clock,
            shadow_cpu_weight: DEFAULT_SHADOW_CPU_WEIGHT,
            shadow_vram_bytes: DEFAULT_SHADOW_VRAM_BYTES,
        }
    }

    pub const fn with_budget_request(mut self, cpu_weight: f64, vram_bytes: u64) -> Self {
        self.shadow_cpu_weight = cpu_weight;
        self.shadow_vram_bytes = vram_bytes;
        self
    }

    pub fn propose_change<A>(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        candidate: &A,
        incumbent: &A,
    ) -> Result<ChangeOutcome>
    where
        A: ShadowAnnealAction,
    {
        self.propose_change_with_description(
            key,
            candidate_ptr,
            candidate,
            incumbent,
            "anneal proposal",
        )
    }

    pub fn propose_change_with_description<A>(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        candidate: &A,
        incumbent: &A,
        description: impl Into<String>,
    ) -> Result<ChangeOutcome>
    where
        A: ShadowAnnealAction,
    {
        let description = description.into();
        let change_id =
            self.rollback
                .prepare_with_description(key, candidate_ptr, description.clone())?;
        let readback = self.rollback.readback(change_id)?;
        let verdict = self.shadow_verdict(candidate, incumbent)?;
        match verdict {
            ShadowVerdict::Promote { metrics } => {
                let entry =
                    ledger_entry(&readback, AnnealLedgerAction::Promote, metrics, description);
                self.write_ledger(entry)?;
                self.rollback.promote(change_id)?;
                Ok(ChangeOutcome::Promoted(change_id))
            }
            ShadowVerdict::Revert { reason, metrics } => {
                self.rollback.rollback(change_id)?;
                let reverted = self.rollback.readback(change_id)?;
                let entry =
                    ledger_entry(&reverted, AnnealLedgerAction::Revert, metrics, description);
                self.write_ledger(entry)?;
                Ok(ChangeOutcome::Reverted { reason, change_id })
            }
        }
    }

    pub fn rollback_explicit(&mut self, change_id: ChangeId) -> Result<()> {
        self.rollback.rollback(change_id)?;
        let readback = self.rollback.readback(change_id)?;
        let entry = ledger_entry(
            &readback,
            AnnealLedgerAction::Revert,
            MetricSnapshot::empty(self.clock.now()),
            "explicit rollback".to_string(),
        );
        self.write_ledger(entry)
    }

    pub fn status(&self) -> Result<AnnealStatus> {
        Ok(AnnealStatus {
            tripwire_states: self.tripwires.status(),
            budget: self.budget.status()?,
            recent_changes: self.ledger.read_recent(16)?,
        })
    }

    fn shadow_verdict<A>(&mut self, candidate: &A, incumbent: &A) -> Result<ShadowVerdict>
    where
        A: ShadowAnnealAction,
    {
        let budget = match self
            .budget
            .acquire(self.shadow_cpu_weight, self.shadow_vram_bytes)
        {
            Ok(handle) => handle,
            Err(error) if error.code == crate::CALYX_ANNEAL_BUDGET_EXHAUSTED => {
                return Ok(ShadowVerdict::Revert {
                    reason: ShadowRevertReason::BudgetExhausted,
                    metrics: MetricSnapshot::empty(self.clock.now()),
                });
            }
            Err(error) => return Err(error),
        };
        let mut executor = ShadowExecutor::new(
            self.tripwires.clone(),
            self.replay.clone(),
            budget,
            self.clock,
        );
        let verdict = executor.run_shadow(candidate, incumbent);
        self.tripwires = executor.registry;
        Ok(verdict)
    }

    fn write_ledger(&mut self, entry: AnnealLedgerEntry) -> Result<()> {
        self.ledger
            .write(entry)
            .map(|_| ())
            .map_err(ledger_write_fail)
    }
}

fn ledger_entry(
    readback: &RollbackReadback,
    action: AnnealLedgerAction,
    metrics: MetricSnapshot,
    description: String,
) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action,
        change_id: readback.snapshot.change_id,
        artifact_id: artifact_id(&readback.snapshot.key),
        prior_ptr_hash: ptr_hash(&readback.snapshot.prior_ptr),
        candidate_ptr_hash: ptr_hash(&readback.snapshot.candidate_ptr),
        metrics,
        ts: readback.snapshot.ts,
        description,
        prev_hash: None,
    }
}

fn artifact_id(key: &ArtifactKey) -> String {
    match key {
        ArtifactKey::ConfigCache(hash)
        | ArtifactKey::HnswGraph(hash)
        | ArtifactKey::QuantLevel(hash) => hex32(hash),
    }
}

fn ptr_hash(ptr: &ArtifactPtr) -> [u8; 32] {
    match ptr {
        ArtifactPtr::ConfigCacheKeyHash(hash) | ArtifactPtr::QuantLevelRecordHash(hash) => *hash,
        ArtifactPtr::HnswGraphPath(path) => full_content_hash([path.as_bytes()]),
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn ledger_write_fail(error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_LEDGER_WRITE_FAIL,
        message: format!(
            "Anneal ledger write failed: {}: {}",
            error.code, error.message
        ),
        remediation: "repair the ledger CF before mutating the live Anneal pointer",
    }
}
