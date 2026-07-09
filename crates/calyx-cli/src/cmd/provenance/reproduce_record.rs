use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use calyx_core::{CalyxError, Input, Result as CalyxResult, VaultStore};
use calyx_forge::{QuantLevel, Quantizer, TurboQuantCodec, new_seed};
use calyx_ledger::{ForgeBackend, RecordedSlot, ReproduceInputResolver};
use calyx_registry::load_vault_panel_state;

use super::{ReproduceOut, ledger_entries, open_vault, reproduce_report};
use crate::cmd::vault::ResolvedVault;
use crate::error::CliResult;

const RETAINED_INPUT_PREFIX: &str = "calyx-vault://inputs/";

pub(super) fn record(resolved: &ResolvedVault, answer_id: &[u8]) -> CliResult<ReproduceOut> {
    let vault = open_vault(resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let resolver = VaultInputResolver {
        vault: &vault,
        vault_path: &resolved.path,
    };
    let mut forge = DeterministicForge;
    let answer_id = answer_id.to_vec();
    vault.record_reproduce_with_input_resolver(
        &state.registry,
        &mut forge,
        &resolver,
        &answer_id,
    )?;
    vault.flush()?;
    let entries = ledger_entries(&resolved.path)?;
    reproduce_report(&entries, &answer_id)
}

struct VaultInputResolver<'a> {
    vault: &'a calyx_aster::vault::AsterVault,
    vault_path: &'a Path,
}

impl ReproduceInputResolver for VaultInputResolver<'_> {
    fn resolve_input(&self, slot: &RecordedSlot) -> CalyxResult<Input> {
        if let Some(input) = &slot.input {
            return Ok(input.clone());
        }
        let stored = self.vault.get(slot.cx_id, self.vault.snapshot())?;
        let pointer = stored.input_ref.pointer.as_ref().ok_or_else(|| {
            CalyxError::reproduce_nondeterministic(format!(
                "cx {} has no retained input pointer for reproduce",
                slot.cx_id
            ))
        })?;
        let path = retained_input_path(self.vault_path, pointer)?;
        let bytes = fs::read(&path).map_err(|error| read_input_error(pointer, &path, error))?;
        Ok(Input::new(stored.modality, bytes).with_pointer(pointer.clone()))
    }
}

fn retained_input_path(vault_path: &Path, pointer: &str) -> CalyxResult<PathBuf> {
    let relative = pointer
        .strip_prefix(RETAINED_INPUT_PREFIX)
        .ok_or_else(|| unsupported_pointer(pointer))?;
    if relative.is_empty() {
        return Err(unsupported_pointer(pointer));
    }
    let relative_path = Path::new(relative);
    if relative_path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(unsupported_pointer(pointer));
    }
    Ok(vault_path.join("inputs").join(relative_path))
}

fn unsupported_pointer(pointer: &str) -> CalyxError {
    CalyxError::reproduce_nondeterministic(format!(
        "unsupported retained input pointer {pointer:?}; expected {RETAINED_INPUT_PREFIX}<name>"
    ))
}

fn read_input_error(pointer: &str, path: &Path, error: std::io::Error) -> CalyxError {
    if error.kind() == ErrorKind::NotFound {
        return CalyxError::reproduce_nondeterministic(format!(
            "retained input pointer {pointer:?} resolved to missing file {}",
            path.display()
        ));
    }
    CalyxError::disk_pressure(format!("read retained input {}: {error}", path.display()))
}

struct DeterministicForge;

impl ForgeBackend for DeterministicForge {
    fn activate_determinism(&mut self, seed: u64) -> CalyxResult<()> {
        let rotation = new_seed(16, &seed.to_le_bytes());
        let codec =
            TurboQuantCodec::new(rotation.clone(), QuantLevel::Bits3p5).map_err(map_forge)?;
        let vector = deterministic_vector(seed);
        let encoded = codec.encode(&vector).map_err(map_forge)?;
        let decoded = codec.decode(&encoded).map_err(map_forge)?;
        if decoded.len() != vector.len() {
            return Err(CalyxError::forge_numerical_invariant(format!(
                "decoded determinism vector len {} != {}",
                decoded.len(),
                vector.len()
            )));
        }
        let replay = TurboQuantCodec::new(rotation, QuantLevel::Bits3p5).map_err(map_forge)?;
        let encoded_again = replay.encode(&vector).map_err(map_forge)?;
        if encoded != encoded_again {
            return Err(CalyxError::forge_numerical_invariant(
                "TurboQuant deterministic seed replay changed encoded bytes",
            ));
        }
        Ok(())
    }
}

fn deterministic_vector(seed: u64) -> Vec<f32> {
    (0..16)
        .map(|idx| {
            let byte = seed.rotate_left(idx as u32).to_le_bytes()[0];
            (f32::from(byte) + 1.0) / 257.0
        })
        .collect()
}

fn map_forge(error: calyx_forge::ForgeError) -> CalyxError {
    CalyxError::forge_numerical_invariant(error.to_string())
}
