use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::CalyxError;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

mod contract;

pub(crate) use contract::PgContractReport;
use contract::verify_pg_contract_for;

pub(crate) const CALYX_VAULT_NOT_FOUND: &str = "CALYX_VAULT_NOT_FOUND";
pub(crate) const CALYX_VAULT_SYNC_FAILED: &str = "CALYX_VAULT_SYNC_FAILED";
pub(crate) const CALYX_VAULT_MODE_ROLLBACK_DENIED: &str = "CALYX_VAULT_MODE_ROLLBACK_DENIED";
pub(crate) const CALYX_PG_CONTRACT_VIOLATION: &str = "CALYX_PG_CONTRACT_VIOLATION";
pub(crate) const CALYX_CONTRACT_NAME_MISSING: &str = "CALYX_CONTRACT_NAME_MISSING";
pub(crate) const CALYX_MANIFEST_CORRUPT: &str = "CALYX_MANIFEST_CORRUPT";

const MANIFEST_MAGIC: &[u8; 8] = b"CXSHDW1!";
const MANIFEST_NAME: &str = "MANIFEST";
const SHADOW_WAL: &str = "00000000000000000000.wal";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) enum VaultMode {
    Shadow,
    Calyx,
    CalyxOnly,
}

impl VaultMode {
    fn byte(self) -> u8 {
        match self {
            Self::Shadow => 0,
            Self::Calyx => 1,
            Self::CalyxOnly => 2,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Shadow),
            1 => Some(Self::Calyx),
            2 => Some(Self::CalyxOnly),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ShadowVault {
    sqlite: SqliteHandle,
    calyx: CalyxHandle,
}

#[derive(Debug)]
struct SqliteHandle {
    path: PathBuf,
    read_path: PathBuf,
    conn: Option<Connection>,
    database_name: String,
}

#[derive(Debug)]
struct CalyxHandle {
    root: PathBuf,
    manifest: ShadowManifest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ShadowManifest {
    schema_version: u32,
    mode: VaultMode,
    database_name: String,
    sqlite_path_digest: String,
    calyx_chunk_count: u64,
    created_at_ms: u64,
    #[serde(default)]
    features: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ShadowManifestReadback {
    pub manifest_path: PathBuf,
    pub magic: String,
    pub mode: VaultMode,
    pub mode_byte: u8,
    pub database_name: String,
    pub sqlite_path_digest: String,
    pub chunk_count: u64,
    pub wal_path: PathBuf,
    pub wal_bytes: u64,
    pub features: BTreeMap<String, String>,
}

impl ShadowVault {
    pub(crate) fn open(sqlite_path: &Path, calyx_dir: &Path) -> Result<Self, CalyxError> {
        let sqlite = SqliteHandle::open(sqlite_path)?;
        verify_pg_contract_for(sqlite.conn.as_ref().expect("sqlite handle open"))?;
        let calyx = CalyxHandle::open_or_create(calyx_dir, sqlite_path, &sqlite.database_name)?;
        Ok(Self { sqlite, calyx })
    }

    pub(crate) fn open_with_archived_sqlite(
        sqlite_path: &Path,
        archived_path: &Path,
        calyx_dir: &Path,
    ) -> Result<Self, CalyxError> {
        let sqlite = SqliteHandle::open_logical(sqlite_path, archived_path)?;
        verify_pg_contract_for(sqlite.conn.as_ref().expect("sqlite handle open"))?;
        let calyx = CalyxHandle::open_or_create(calyx_dir, sqlite_path, &sqlite.database_name)?;
        Ok(Self { sqlite, calyx })
    }

    pub(crate) fn close(mut self) -> Result<(), CalyxError> {
        self.calyx.sync()?;
        if let Some(conn) = self.sqlite.conn.take() {
            conn.close()
                .map_err(|(_, error)| vault_sync(format!("close sqlite: {error}")))?;
        }
        if self.sqlite.path.is_file() {
            sync_file(&self.sqlite.path)?;
        }
        Ok(())
    }

    pub(crate) fn vault_name(&self) -> &str {
        &self.sqlite.database_name
    }

    pub(crate) fn paths(&self) -> (&Path, &Path) {
        (&self.sqlite.path, &self.calyx.root)
    }

    pub(crate) fn sqlite_read_path(&self) -> &Path {
        &self.sqlite.read_path
    }

    pub(crate) fn mode(&self) -> VaultMode {
        self.calyx.manifest.mode
    }

    pub(crate) fn set_mode(&mut self, next: VaultMode) -> Result<(), CalyxError> {
        self.set_mode_with_features(next, &[])
    }

    pub(crate) fn set_mode_with_features(
        &mut self,
        next: VaultMode,
        entries: &[(&str, String)],
    ) -> Result<(), CalyxError> {
        let old = self.calyx.manifest.clone();
        if next < old.mode {
            let message = format!(
                "cannot move vault mode from {:?} back to {next:?}",
                old.mode
            );
            return Err(error(
                CALYX_VAULT_MODE_ROLLBACK_DENIED,
                message,
                "open a forward migration issue; vault mode is a one-way ratchet",
            ));
        }
        self.calyx.manifest.mode = next;
        for (key, value) in entries {
            self.calyx
                .manifest
                .features
                .insert((*key).to_string(), value.clone());
        }
        if let Err(error) = self.calyx.write_manifest() {
            self.calyx.manifest = old;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn verify_pg_contract(&self) -> Result<PgContractReport, CalyxError> {
        let conn = self
            .sqlite
            .conn
            .as_ref()
            .ok_or_else(|| vault_sync("sqlite handle is already released"))?;
        verify_pg_contract_for(conn)
    }

    pub(crate) fn manifest_readback(&self) -> Result<ShadowManifestReadback, CalyxError> {
        read_shadow_manifest(&self.calyx.root)
    }

    pub(crate) fn release_sqlite_for_archive(&mut self) -> Result<(), CalyxError> {
        if let Some(conn) = self.sqlite.conn.take() {
            conn.close()
                .map_err(|(_, error)| vault_sync(format!("close sqlite: {error}")))?;
        }
        if self.sqlite.path.is_file() {
            sync_file(&self.sqlite.path)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn append_shadow_wal_marker(&mut self, bytes: &[u8]) -> Result<(), CalyxError> {
        use std::io::Write;

        let path = shadow_wal_path(&self.calyx.root);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|error| vault_sync(format!("open shadow WAL {}: {error}", path.display())))?;
        file.write_all(bytes)
            .map_err(|error| vault_sync(format!("append shadow WAL: {error}")))?;
        file.sync_all()
            .map_err(|error| vault_sync(format!("sync shadow WAL: {error}")))
    }
}

pub(crate) fn read_shadow_manifest(vault: &Path) -> Result<ShadowManifestReadback, CalyxError> {
    let path = manifest_path(vault);
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|error| manifest_corrupt(format!("open {}: {error}", path.display())))?
        .read_to_end(&mut bytes)
        .map_err(|error| manifest_corrupt(format!("read {}: {error}", path.display())))?;
    decode_manifest_bytes(vault, &bytes)
}

pub(crate) fn update_shadow_chunk_count(
    vault: &Path,
    chunk_count: u64,
) -> Result<ShadowManifestReadback, CalyxError> {
    let path = manifest_path(vault);
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|error| manifest_corrupt(format!("open {}: {error}", path.display())))?
        .read_to_end(&mut bytes)
        .map_err(|error| manifest_corrupt(format!("read {}: {error}", path.display())))?;
    let mut manifest = decode_manifest_model(&bytes)?;
    manifest.calyx_chunk_count = chunk_count;
    let handle = CalyxHandle {
        root: vault.to_path_buf(),
        manifest,
    };
    handle.write_manifest()?;
    read_shadow_manifest(vault)
}

impl SqliteHandle {
    fn open(path: &Path) -> Result<Self, CalyxError> {
        Self::open_logical(path, path)
    }

    fn open_logical(logical_path: &Path, read_path: &Path) -> Result<Self, CalyxError> {
        if !read_path.is_file() {
            return Err(error(
                CALYX_VAULT_NOT_FOUND,
                format!("sqlite vault {} does not exist", read_path.display()),
                "provide the existing Leapable vault .db path",
            ));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(read_path, flags).map_err(|err| {
            error(
                CALYX_VAULT_NOT_FOUND,
                format!("open sqlite vault {}: {err}", read_path.display()),
                "provide a readable existing Leapable vault .db path",
            )
        })?;
        let database_name = read_database_name(&conn)?;
        Ok(Self {
            path: logical_path.to_path_buf(),
            read_path: read_path.to_path_buf(),
            conn: Some(conn),
            database_name,
        })
    }
}

impl CalyxHandle {
    fn open_or_create(
        root: &Path,
        sqlite_path: &Path,
        database_name: &str,
    ) -> Result<Self, CalyxError> {
        let manifest_path = manifest_path(root);
        let manifest = if manifest_path.exists() {
            let readback = read_shadow_manifest(root)?;
            if readback.database_name != database_name {
                return Err(manifest_corrupt(format!(
                    "manifest database_name {} does not match sqlite database_name {database_name}",
                    readback.database_name
                )));
            }
            ShadowManifest {
                schema_version: 1,
                mode: readback.mode,
                database_name: readback.database_name,
                sqlite_path_digest: readback.sqlite_path_digest,
                calyx_chunk_count: readback.chunk_count,
                created_at_ms: 0,
                features: readback.features,
            }
        } else {
            fs::create_dir_all(root).map_err(|error| {
                vault_sync(format!("create vault dir {}: {error}", root.display()))
            })?;
            fs::create_dir_all(root.join("wal")).map_err(|error| {
                vault_sync(format!("create shadow WAL dir {}: {error}", root.display()))
            })?;
            File::create(shadow_wal_path(root))
                .and_then(|file| file.sync_all())
                .map_err(|error| vault_sync(format!("create shadow WAL: {error}")))?;
            ShadowManifest {
                schema_version: 1,
                mode: VaultMode::Shadow,
                database_name: database_name.to_string(),
                sqlite_path_digest: blake3::hash(sqlite_path.to_string_lossy().as_bytes())
                    .to_hex()
                    .to_string(),
                calyx_chunk_count: 0,
                created_at_ms: now_ms(),
                features: BTreeMap::new(),
            }
        };
        let handle = Self {
            root: root.to_path_buf(),
            manifest,
        };
        handle.write_manifest()?;
        Ok(handle)
    }

    fn write_manifest(&self) -> Result<(), CalyxError> {
        let path = manifest_path(&self.root);
        let tmp = self.root.join("MANIFEST.tmp");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MANIFEST_MAGIC);
        bytes.push(self.manifest.mode.byte());
        bytes.extend_from_slice(
            &serde_json::to_vec(&self.manifest)
                .map_err(|error| manifest_corrupt(format!("encode shadow manifest: {error}")))?,
        );
        fs::write(&tmp, bytes)
            .map_err(|error| vault_sync(format!("write {}: {error}", tmp.display())))?;
        sync_file(&tmp)?;
        fs::rename(&tmp, &path)
            .map_err(|error| vault_sync(format!("rename {}: {error}", path.display())))?;
        sync_dir(&self.root)
    }

    fn sync(&self) -> Result<(), CalyxError> {
        if !manifest_path(&self.root).is_file() {
            return Err(vault_sync("shadow MANIFEST missing during close"));
        }
        if !shadow_wal_path(&self.root).is_file() {
            return Err(vault_sync("shadow WAL missing during close"));
        }
        sync_file(&manifest_path(&self.root))?;
        sync_file(&shadow_wal_path(&self.root))?;
        sync_dir(&self.root)
    }
}

fn decode_manifest_model(bytes: &[u8]) -> Result<ShadowManifest, CalyxError> {
    if bytes.len() <= MANIFEST_MAGIC.len() {
        return Err(manifest_corrupt("shadow manifest is truncated"));
    }
    if &bytes[..MANIFEST_MAGIC.len()] != MANIFEST_MAGIC {
        return Err(manifest_corrupt("shadow manifest magic mismatch"));
    }
    let mode_byte = bytes[MANIFEST_MAGIC.len()];
    let mode = VaultMode::from_byte(mode_byte)
        .ok_or_else(|| manifest_corrupt(format!("unknown shadow mode byte {mode_byte}")))?;
    let manifest: ShadowManifest = serde_json::from_slice(&bytes[MANIFEST_MAGIC.len() + 1..])
        .map_err(|error| manifest_corrupt(format!("decode shadow manifest json: {error}")))?;
    if manifest.schema_version != 1 || manifest.mode != mode {
        return Err(manifest_corrupt("shadow manifest header/body mismatch"));
    }
    Ok(manifest)
}

fn decode_manifest_bytes(vault: &Path, bytes: &[u8]) -> Result<ShadowManifestReadback, CalyxError> {
    let manifest = decode_manifest_model(bytes)?;
    let mode = manifest.mode;
    let wal_path = shadow_wal_path(vault);
    let wal_bytes = fs::metadata(&wal_path).map_or(0, |metadata| metadata.len());
    Ok(ShadowManifestReadback {
        manifest_path: manifest_path(vault),
        magic: String::from_utf8_lossy(MANIFEST_MAGIC).to_string(),
        mode,
        mode_byte: mode.byte(),
        database_name: manifest.database_name,
        sqlite_path_digest: manifest.sqlite_path_digest,
        chunk_count: manifest.calyx_chunk_count,
        wal_path,
        wal_bytes,
        features: manifest.features,
    })
}

fn read_database_name(conn: &Connection) -> Result<String, CalyxError> {
    let name = conn
        .query_row(
            "SELECT database_name FROM database_metadata ORDER BY rowid LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| {
            error(
                CALYX_CONTRACT_NAME_MISSING,
                "database_metadata.database_name row is absent",
                "preserve the source SQLite database_name metadata row before opening shadow mode",
            )
        })?;
    if name.is_empty() {
        return Err(error(
            CALYX_CONTRACT_NAME_MISSING,
            "database_metadata.database_name is empty",
            "restore the verbatim Leapable database_name before opening shadow mode",
        ));
    }
    Ok(name)
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_NAME)
}

fn shadow_wal_path(root: &Path) -> PathBuf {
    root.join("wal").join(SHADOW_WAL)
}

fn sync_file(path: &Path) -> Result<(), CalyxError> {
    File::options()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| vault_sync(format!("sync {}: {error}", path.display())))
}

fn sync_dir(path: &Path) -> Result<(), CalyxError> {
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
    #[cfg(not(windows))]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| vault_sync(format!("sync dir {}: {error}", path.display())))
}

fn now_ms() -> u64 {
    system_time_ms(SystemTime::now()).unwrap_or(0)
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn manifest_corrupt(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_MANIFEST_CORRUPT,
        message,
        "delete and recreate only the shadow vault dir after preserving the source SQLite vault",
    )
}

fn vault_sync(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_VAULT_SYNC_FAILED,
        message,
        "inspect MANIFEST and wal bytes, then retry close after storage is healthy",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
