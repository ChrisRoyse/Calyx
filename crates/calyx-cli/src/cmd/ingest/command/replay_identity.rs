use super::batch_support::source_columns;
use super::*;

pub(super) struct PreparedBatchIdentity {
    pub(super) cx_id: CxId,
    pub(super) input_ref: InputRef,
    pub(super) modality: Modality,
    pub(super) scalars: BTreeMap<String, f64>,
    pub(super) metadata: BTreeMap<String, String>,
    pub(super) retained_bytes: Vec<u8>,
}

pub(super) fn prepare_batch_identity(
    vault: &AsterVault,
    state: &VaultPanelState,
    _vault_path: &std::path::Path,
    row: &BatchRow,
) -> CliResult<PreparedBatchIdentity> {
    let input = retention::prepared_hybrid_text_input(&row.text, row.structured.as_ref());
    let cx_id = {
        let identity = input.identity_bytes()?;
        vault.cx_id_for_input(identity.as_ref(), state.panel.version)
    };
    let (scalars, metadata) = source_columns(&input, row.metadata.clone(), row.oracle.as_ref())?;
    Ok(PreparedBatchIdentity {
        cx_id,
        input_ref: InputRef {
            hash: input_hash(&input.bytes),
            pointer: input.pointer,
            redacted: false,
        },
        modality: input.modality,
        scalars,
        metadata,
        retained_bytes: input.bytes,
    })
}
