use std::collections::VecDeque;

use calyx_core::{CalyxError, LensCost, LensId, Placement, SlotResource};
use serde::{Deserialize, Serialize};

use crate::LensRuntime;

pub const CALYX_VRAM_BUDGET_EXCEEDED: &str = "CALYX_VRAM_BUDGET_EXCEEDED";
pub const CALYX_RAM_BUDGET_EXCEEDED: &str = "CALYX_RAM_BUDGET_EXCEEDED";
pub const LENS_VRAM_REMEDIATION: &str =
    "lower precision, evict cold GPU lenses, move to CPU, or raise the Forge VRAM budget";
pub const LENS_RAM_REMEDIATION: &str =
    "raise the CPU lens pool cap or evict cold resident CPU lenses";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementBudget {
    pub vram_soft_cap_bytes: u64,
    pub tei_reserved_bytes: u64,
    pub vram_allocated_bytes: u64,
    pub ram_soft_cap_bytes: u64,
    pub ram_used_bytes: u64,
    pub cpu_resident_limit: usize,
    pub cpu_resident_count: usize,
}

impl PlacementBudget {
    pub fn available_vram_bytes(&self) -> u64 {
        self.vram_soft_cap_bytes
            .saturating_sub(self.tei_reserved_bytes)
            .saturating_sub(self.vram_allocated_bytes)
    }

    pub fn available_ram_bytes(&self) -> u64 {
        self.ram_soft_cap_bytes.saturating_sub(self.ram_used_bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacementPlan {
    pub resource: SlotResource,
    pub reason: String,
    pub available_vram_bytes: u64,
    pub available_ram_bytes: u64,
}

pub fn choose_placement(
    runtime: &LensRuntime,
    cost: LensCost,
    budget: PlacementBudget,
) -> Result<PlacementPlan, CalyxError> {
    if cost.is_zero_cost() && matches!(runtime, LensRuntime::Algorithmic { .. }) {
        return Ok(plan(cost, Placement::Cpu, budget, "zero-cost lens admits"));
    }

    if runtime_prefers_cpu(runtime) {
        ensure_cpu_budget(cost, budget)?;
        return Ok(plan(cost, Placement::Cpu, budget, "CPU-native runtime"));
    }

    if cost.vram_bytes <= budget.available_vram_bytes() {
        return Ok(plan(
            cost,
            Placement::Gpu,
            budget,
            "fits GPU budget after TEI reservation",
        ));
    }

    Err(vram_budget_error(format!(
        "lens requires {} VRAM bytes, available {} after TEI reservation {} and allocated {}",
        cost.vram_bytes,
        budget.available_vram_bytes(),
        budget.tei_reserved_bytes,
        budget.vram_allocated_bytes
    )))
}

fn plan(
    cost: LensCost,
    placement: Placement,
    budget: PlacementBudget,
    reason: &str,
) -> PlacementPlan {
    PlacementPlan {
        resource: SlotResource { cost, placement },
        reason: reason.to_string(),
        available_vram_bytes: budget.available_vram_bytes(),
        available_ram_bytes: budget.available_ram_bytes(),
    }
}

fn runtime_prefers_cpu(runtime: &LensRuntime) -> bool {
    matches!(
        runtime,
        LensRuntime::Algorithmic { .. }
            | LensRuntime::MultimodalAdapter { .. }
            | LensRuntime::StaticLookup { .. }
            | LensRuntime::ExternalCmd { .. }
    )
}

fn ensure_cpu_budget(cost: LensCost, budget: PlacementBudget) -> Result<(), CalyxError> {
    if cost.is_zero_cost() {
        return Ok(());
    }
    if budget.cpu_resident_count >= budget.cpu_resident_limit {
        return Err(ram_budget_error(format!(
            "CPU lens pool full: resident_count={} limit={}",
            budget.cpu_resident_count, budget.cpu_resident_limit
        )));
    }
    if cost.ram_bytes > budget.available_ram_bytes() {
        return Err(ram_budget_error(format!(
            "lens requires {} RAM bytes, available {}",
            cost.ram_bytes,
            budget.available_ram_bytes()
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuPoolAdmission {
    pub evicted_lenses: Vec<LensId>,
    pub resident_lenses: usize,
    pub resident_ram_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CpuResidentLens {
    lens_id: LensId,
    ram_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuLensPool {
    resident_limit: usize,
    ram_soft_cap_bytes: u64,
    resident: VecDeque<CpuResidentLens>,
}

impl CpuLensPool {
    pub fn new(resident_limit: usize, ram_soft_cap_bytes: u64) -> Self {
        Self {
            resident_limit,
            ram_soft_cap_bytes,
            resident: VecDeque::new(),
        }
    }

    pub fn admit(
        &mut self,
        lens_id: LensId,
        cost: LensCost,
    ) -> Result<CpuPoolAdmission, CalyxError> {
        if cost.is_zero_cost() {
            return Ok(self.admission(Vec::new()));
        }
        if self.resident_limit == 0 || cost.ram_bytes > self.ram_soft_cap_bytes {
            return Err(ram_budget_error(format!(
                "lens {lens_id} requires {} RAM bytes with pool_limit={} ram_cap={}",
                cost.ram_bytes, self.resident_limit, self.ram_soft_cap_bytes
            )));
        }

        self.remove(lens_id);
        let mut evicted = Vec::new();
        while self.resident.len() >= self.resident_limit
            || self.resident_ram_bytes().saturating_add(cost.ram_bytes) > self.ram_soft_cap_bytes
        {
            let Some(old) = self.resident.pop_front() else {
                break;
            };
            evicted.push(old.lens_id);
        }

        self.resident.push_back(CpuResidentLens {
            lens_id,
            ram_bytes: cost.ram_bytes,
        });
        Ok(self.admission(evicted))
    }

    pub fn resident_ram_bytes(&self) -> u64 {
        self.resident
            .iter()
            .map(|entry| entry.ram_bytes)
            .fold(0_u64, u64::saturating_add)
    }

    pub fn resident_lenses(&self) -> usize {
        self.resident.len()
    }

    fn remove(&mut self, lens_id: LensId) {
        if let Some(index) = self
            .resident
            .iter()
            .position(|entry| entry.lens_id == lens_id)
        {
            self.resident.remove(index);
        }
    }

    fn admission(&self, evicted_lenses: Vec<LensId>) -> CpuPoolAdmission {
        CpuPoolAdmission {
            evicted_lenses,
            resident_lenses: self.resident_lenses(),
            resident_ram_bytes: self.resident_ram_bytes(),
        }
    }
}

fn vram_budget_error(message: String) -> CalyxError {
    CalyxError {
        code: CALYX_VRAM_BUDGET_EXCEEDED,
        message,
        remediation: LENS_VRAM_REMEDIATION,
    }
}

fn ram_budget_error(message: String) -> CalyxError {
    CalyxError {
        code: CALYX_RAM_BUDGET_EXCEEDED,
        message,
        remediation: LENS_RAM_REMEDIATION,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;

    #[test]
    fn deterministic_placement_across_fixture_budget() {
        let budget = budget(8 * GIB, 3 * GIB, 2 * GIB, 16 * GIB, GIB, 8, 1);

        let static_plan = choose_placement(&static_runtime(), cost(1, 0, 128 * MIB), budget)
            .expect("static lens admits");
        let gpu_plan = choose_placement(&onnx_runtime(), cost(10, 2 * GIB, 256 * MIB), budget)
            .expect("onnx lens admits");

        assert_eq!(static_plan.resource.placement, Placement::Cpu);
        assert_eq!(gpu_plan.resource.placement, Placement::Gpu);
        assert_eq!(gpu_plan.available_vram_bytes, 3 * GIB);
    }

    #[test]
    fn gpu_cap_exhaustion_fails_closed_for_gpu_capable_onnx() {
        let budget = budget(4 * GIB, 2 * GIB, 1536 * MIB, 8 * GIB, 0, 4, 0);

        let error = choose_placement(&onnx_runtime(), cost(10, GIB, 64 * MIB), budget)
            .expect_err("hidden CPU fallback is not allowed");

        assert_eq!(error.code, CALYX_VRAM_BUDGET_EXCEEDED);
        assert!(error.message.contains("available"));
    }

    #[test]
    fn oversized_gpu_lens_refuses_with_remediation() {
        let budget = budget(4 * GIB, 3 * GIB, 0, 8 * GIB, 0, 4, 0);

        let error = choose_placement(&candle_runtime(), cost(10, 2 * GIB, 64 * MIB), budget)
            .expect_err("candle has no CPU fallback");

        assert_eq!(error.code, CALYX_VRAM_BUDGET_EXCEEDED);
        assert!(error.remediation.contains("lower precision"));
    }

    #[test]
    fn zero_cost_lens_always_admits() {
        let budget = budget(0, 0, 0, 0, 0, 0, 0);

        let plan = choose_placement(&algorithmic_runtime(), LensCost::zero(), budget)
            .expect("zero-cost admits");

        assert_eq!(plan.resource.placement, Placement::Cpu);
    }

    #[test]
    fn cpu_pool_lru_evicts_cold_lenses_before_refusing() {
        let mut pool = CpuLensPool::new(2, 3 * MIB);
        let a = LensId::from_bytes([1; 16]);
        let b = LensId::from_bytes([2; 16]);
        let c = LensId::from_bytes([3; 16]);

        pool.admit(a, cost(1, 0, MIB)).unwrap();
        pool.admit(b, cost(1, 0, MIB)).unwrap();
        let admission = pool.admit(c, cost(1, 0, 2 * MIB)).unwrap();

        assert_eq!(admission.evicted_lenses, vec![a]);
        assert_eq!(admission.resident_lenses, 2);
        assert_eq!(admission.resident_ram_bytes, 3 * MIB);
    }

    #[test]
    fn cpu_pool_refuses_oversized_lens() {
        let mut pool = CpuLensPool::new(4, MIB);

        let error = pool
            .admit(LensId::from_bytes([9; 16]), cost(1, 0, 2 * MIB))
            .expect_err("oversized CPU lens refuses");

        assert_eq!(error.code, CALYX_RAM_BUDGET_EXCEEDED);
    }

    fn budget(
        vram_soft_cap_bytes: u64,
        tei_reserved_bytes: u64,
        vram_allocated_bytes: u64,
        ram_soft_cap_bytes: u64,
        ram_used_bytes: u64,
        cpu_resident_limit: usize,
        cpu_resident_count: usize,
    ) -> PlacementBudget {
        PlacementBudget {
            vram_soft_cap_bytes,
            tei_reserved_bytes,
            vram_allocated_bytes,
            ram_soft_cap_bytes,
            ram_used_bytes,
            cpu_resident_limit,
            cpu_resident_count,
        }
    }

    fn cost(ms_per_input: u32, vram_bytes: u64, ram_bytes: u64) -> LensCost {
        LensCost {
            total_ms: ms_per_input as f32,
            ms_per_input: ms_per_input as f32,
            vram_bytes,
            ram_bytes,
            batch_ceiling: 1_000 / ms_per_input.max(1),
        }
    }

    fn static_runtime() -> LensRuntime {
        LensRuntime::StaticLookup {
            embeddings_file: PathBuf::from("embeddings.bin"),
            tokenizer: PathBuf::from("tokenizer.json"),
            dim: 384,
        }
    }

    fn onnx_runtime() -> LensRuntime {
        LensRuntime::Onnx {
            model_id: "fixture-onnx".to_string(),
            files: Vec::new(),
        }
    }

    fn candle_runtime() -> LensRuntime {
        LensRuntime::CandleLocal {
            model_id: "fixture-candle".to_string(),
            files: Vec::new(),
            dtype: "f16".to_string(),
            pooling: "mean".to_string(),
        }
    }

    fn algorithmic_runtime() -> LensRuntime {
        LensRuntime::Algorithmic {
            kind: "byte_features".to_string(),
        }
    }
}
