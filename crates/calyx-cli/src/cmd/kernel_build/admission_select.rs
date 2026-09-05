use calyx_core::SlotId;
use calyx_lodestar::TypedPanelVectors;
use calyx_registry::VaultPanelState;

use super::super::kernel_generation::{KernelAdmissionContract, KernelJurisdictionContract};
use super::super::vault::ResolvedVault;
use super::{KernelBuildArgs, admission};
use crate::error::{CliError, CliResult};

pub(super) fn select(
    resolved: &ResolvedVault,
    state: &VaultPanelState,
    embedding_slots: &[SlotId],
    rows: &std::collections::BTreeMap<calyx_core::CxId, TypedPanelVectors>,
    jurisdiction: Option<&KernelJurisdictionContract>,
    args: &KernelBuildArgs,
) -> CliResult<KernelAdmissionContract> {
    match (
        jurisdiction,
        args.admission_queries.as_deref(),
        args.resident_addr,
    ) {
        (Some(_), Some(path), Some(resident_addr)) => admission::calibrate_real_queries(
            resolved,
            state,
            embedding_slots,
            rows,
            path,
            resident_addr,
        ),
        (Some(_), _, _) => Err(CliError::usage(
            "legal kernel-build requires --admission-queries <jsonl> and --resident-addr <loopback-address>; admission must be calibrated through every sealed graph slot",
        )),
        (None, None, None) => admission::calibrate_corpus_neighbors(embedding_slots, rows),
        (None, _, _) => Err(CliError::usage(
            "--admission-queries and --resident-addr are only valid for a graph with a sealed jurisdiction contract",
        )),
    }
}
