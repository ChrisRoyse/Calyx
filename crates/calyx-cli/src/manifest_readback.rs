use std::fs::File;
use std::io::Read;
use std::path::Path;

use calyx_aster::manifest::ManifestStore;
use calyx_core::CalyxError;
use serde_json::json;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

/// Native Aster vaults carry a `CURRENT` pointer file naming the live
/// immutable `manifest-<seq>.json`; a Leapable shadow vault has no `CURRENT`.
const NATIVE_CURRENT_FILE: &str = "CURRENT";

/// Leapable shadow vaults carry this binary magic at the start of `MANIFEST`.
const SHADOW_MANIFEST_MAGIC: &[u8] = b"CXSHDW1!";
const SHADOW_MANIFEST_FILE: &str = "MANIFEST";

/// Stable code for a `--show-manifest` target that is neither a native Aster
/// vault nor a Leapable shadow vault (issue #1262). Not a PRD 18 catalog entry;
/// constructed locally with a fixed remediation so an agent can dispatch on it.
const CALYX_VAULT_FORMAT_UNRECOGNIZED: &str = "CALYX_VAULT_FORMAT_UNRECOGNIZED";

/// `readback --vault <dir> --show-manifest`.
///
/// Auto-detects the vault format from its on-disk bytes (issue #1262) and
/// dispatches to the matching reader, instead of assuming every vault is a
/// Leapable shadow vault. Detection order is content-first:
///
/// 1. If `MANIFEST` begins with the shadow magic `CXSHDW1!`, read the shadow
///    manifest (and surface any shadow-specific corruption verbatim).
/// 2. Otherwise, if a native `CURRENT` pointer exists, read the native Aster
///    manifest via [`ManifestStore::load_current`].
/// 3. Otherwise fail closed with [`CALYX_VAULT_FORMAT_UNRECOGNIZED`]; the target
///    is not a materialized vault of either kind.
///
/// Before this routed by format, a valid native vault reported the misleading
/// `CALYX_MANIFEST_CORRUPT: shadow manifest magic mismatch` because the shadow
/// reader was parsing the native JSON `MANIFEST` mirror.
pub fn readback_vault_manifest(vault: &Path) -> CliResult {
    if is_shadow_vault(vault)? {
        return crate::leapable::readback_shadow_manifest(vault);
    }
    if vault.join(NATIVE_CURRENT_FILE).is_file() {
        return readback_native_manifest(vault);
    }
    Err(unrecognized_vault(vault))
}

/// Content-based detection of a Leapable shadow vault (issue #1262).
///
/// A shadow vault's `MANIFEST` begins with `CXSHDW1!`; a native Aster vault's
/// `MANIFEST` is a JSON mirror that begins with `{`. Dispatching by the bytes
/// prevents a valid native vault from being misreported as a corrupt shadow
/// vault.
fn is_shadow_vault(vault: &Path) -> CliResult<bool> {
    let path = vault.join(SHADOW_MANIFEST_FILE);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(CliError::runtime(format!(
                "open {} while detecting vault format: {error}",
                path.display()
            )));
        }
    };
    let mut magic = [0u8; SHADOW_MANIFEST_MAGIC.len()];
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(magic.as_slice() == SHADOW_MANIFEST_MAGIC),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(CliError::runtime(format!(
            "read {} while detecting vault format: {error}",
            path.display()
        ))),
    }
}

/// Reads and prints the full native Aster manifest, tagged with
/// `vault_format: "native-aster"` so a consumer can tell it apart from the
/// shadow readback shape (which carries `magic`/`mode` fields).
fn readback_native_manifest(vault: &Path) -> CliResult {
    let manifest = ManifestStore::open(vault).load_current()?;
    let manifest_json = serde_json::to_value(&manifest)
        .map_err(|error| CliError::runtime(format!("serialize vault manifest: {error}")))?;
    print_json(&json!({
        "vault_format": "native-aster",
        "manifest": manifest_json,
    }))
}

/// `readback vault-manifest --field <name> --vault <dir>` - one native field.
pub fn readback_vault_manifest_field(vault: &Path, field: &str) -> CliResult {
    let manifest = ManifestStore::open(vault).load_current()?;
    let manifest_json = serde_json::to_value(&manifest)
        .map_err(|error| CliError::runtime(format!("serialize vault manifest: {error}")))?;
    let value = manifest_json
        .get(field)
        .ok_or_else(|| CliError::usage(format!("manifest field `{field}` not found")))?;
    print_json(value)
}

fn unrecognized_vault(vault: &Path) -> CliError {
    CliError::Calyx(CalyxError {
        code: CALYX_VAULT_FORMAT_UNRECOGNIZED,
        message: format!(
            "{} is neither a native Aster vault (no CURRENT pointer) nor a Leapable \
             shadow vault (MANIFEST does not begin with CXSHDW1!)",
            vault.display()
        ),
        remediation: "point --show-manifest at a materialized vault directory: a native \
                      Aster vault has a CURRENT file, a Leapable shadow vault has a MANIFEST \
                      beginning with CXSHDW1!",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_aster::manifest::{ImmutableRef, ManifestStore, VaultManifest};
    use std::fs;

    /// Materializes a real, loadable native Aster vault (not a mock): valid
    /// immutable panel/codebook assets plus a `CURRENT`-pointed manifest.
    fn write_native_vault(dir: &Path) {
        fs::create_dir_all(dir.join("panel")).expect("panel dir");
        fs::create_dir_all(dir.join("codebooks")).expect("codebook dir");
        fs::write(dir.join("panel/panel-0001.json"), b"panel").expect("panel bytes");
        fs::write(dir.join("codebooks/slot_00.cb"), b"codebook").expect("codebook bytes");
        let manifest = VaultManifest::new(
            1,
            10,
            ImmutableRef::from_bytes("panel/panel-0001.json", b"panel").unwrap(),
            vec![ImmutableRef::from_bytes("codebooks/slot_00.cb", b"codebook").unwrap()],
        )
        .expect("build native manifest");
        ManifestStore::open(dir)
            .write_current(&manifest)
            .expect("write native vault manifest");
    }

    /// Regression for #1262: a valid native vault must route to the native
    /// reader and load, not report `shadow manifest magic mismatch`.
    #[test]
    fn native_vault_routes_to_native_reader_not_shadow() {
        let dir = std::env::temp_dir().join(format!("calyx-1262-native-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write_native_vault(&dir);

        // Detection: a native vault is not a shadow vault.
        assert!(
            !is_shadow_vault(&dir).expect("detect native"),
            "native vault must not be detected as shadow"
        );
        // Routing: the full readback succeeds instead of erroring.
        readback_vault_manifest(&dir).expect("native --show-manifest must succeed");
        // And the single-field native readback still works on the same vault.
        readback_vault_manifest_field(&dir, "manifest_seq").expect("native field readback");

        fs::remove_dir_all(&dir).ok();
    }

    /// A directory with a shadow-magic MANIFEST is detected as shadow.
    #[test]
    fn shadow_magic_manifest_detected_as_shadow() {
        let dir = std::env::temp_dir().join(format!("calyx-1262-shadow-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("shadow dir");
        // CXSHDW1! magic + mode byte; enough to trip content detection.
        fs::write(dir.join("MANIFEST"), b"CXSHDW1!\x00").expect("shadow manifest");

        assert!(
            is_shadow_vault(&dir).expect("detect shadow"),
            "shadow-magic MANIFEST must be detected as shadow"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// Materializes a persistent native vault at `CALYX_1262_FIXTURE_DIR` for
    /// end-to-end CLI FSV (drive the real `calyx` binary at it). Ignored in the
    /// normal run; invoked explicitly with the env var set.
    #[test]
    #[ignore = "fixture materializer for manual CLI FSV"]
    fn materialize_native_vault_fixture() {
        let dir = std::env::var("CALYX_1262_FIXTURE_DIR")
            .expect("set CALYX_1262_FIXTURE_DIR to the output vault path");
        let dir = Path::new(&dir);
        let _ = fs::remove_dir_all(dir);
        write_native_vault(dir);
        eprintln!("materialized native vault at {}", dir.display());
    }

    /// A directory that is neither format fails closed with a distinct code -
    /// never the misleading shadow corruption error.
    #[test]
    fn unrecognized_dir_fails_closed_with_distinct_code() {
        let dir = std::env::temp_dir().join(format!("calyx-1262-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("empty dir");

        let err = readback_vault_manifest(&dir).expect_err("empty dir must fail");
        assert_eq!(err.code(), CALYX_VAULT_FORMAT_UNRECOGNIZED);
        assert_ne!(err.code(), "CALYX_MANIFEST_CORRUPT");

        fs::remove_dir_all(&dir).ok();
    }
}
