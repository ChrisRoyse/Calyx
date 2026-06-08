use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use proptest::prelude::*;

use super::*;
use crate::{BackendKind, BestConfig};

fn key(recall_tgt: f32) -> AutotuneKey {
    AutotuneKey {
        recall_tgt,
        ..AutotuneKey::default_for("gemm", &[4, 8, 16], "f32", "cuda:0")
    }
}

fn config(tile: usize) -> BestConfig {
    BestConfig {
        backend: BackendKind::Cuda,
        tile_m: tile,
        tile_n: tile / 2,
        tile_k: 16,
        extra: HashMap::from([("kernel".to_string(), format!("tile-{tile}"))]),
    }
}

fn unique_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx_autotune_test_{}_{}_{}.json",
        name,
        std::process::id(),
        nanos
    ))
}

fn hash_of(key: &AutotuneKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

fn read_string(path: &Path) -> String {
    fs::read_to_string(path).expect("read autotune cache JSON")
}

#[test]
fn insert_get_roundtrip_returns_config() -> Result<()> {
    let path = unique_path("insert_get_roundtrip");
    let mut cache = AutotuneCache::load(&path)?;
    let key = key(0.95);
    let expected = config(64);

    cache.insert(key.clone(), expected.clone());

    assert_eq!(cache.get(&key), Some(&expected));
    println!(
        "autotune_insert_get PASSED len={} op={} tile_m={}",
        cache.len(),
        key.op,
        expected.tile_m
    );
    Ok(())
}

#[test]
fn persist_load_roundtrip() -> Result<()> {
    let path = unique_path("persist_load_roundtrip");
    let mut cache = AutotuneCache::load(&path)?;
    let key = key(0.95);
    let expected = config(128);

    cache.insert(key.clone(), expected.clone());
    cache.persist()?;

    let json = read_string(&path);
    assert!(json.contains("\"op\""));
    assert!(json.contains("\"gemm\""));
    let loaded = AutotuneCache::load(&path)?;

    assert_eq!(loaded.get(&key), Some(&expected));
    println!(
        "autotune_persist_load_roundtrip PASSED path={} bytes={} op={} tile_m={}",
        path.display(),
        json.len(),
        key.op,
        expected.tile_m
    );
    Ok(())
}

#[test]
fn load_missing_path_returns_empty_cache() -> Result<()> {
    let path = unique_path("missing_path");
    let cache = AutotuneCache::load(&path)?;

    assert!(cache.is_empty());
    assert_eq!(cache.path(), path.as_path());
    println!(
        "autotune_missing_load PASSED path={} len={}",
        path.display(),
        cache.len()
    );
    Ok(())
}

proptest! {
    #[test]
    fn recall_hash_quantizes_to_centipoints(op in "[a-z]{1,8}") {
        let base = AutotuneKey {
            op,
            ..key(0.951)
        };
        let same = AutotuneKey {
            recall_tgt: 0.954,
            ..base.clone()
        };
        let different = AutotuneKey {
            recall_tgt: 0.96,
            ..base.clone()
        };

        prop_assert_eq!(&base, &same);
        prop_assert_eq!(hash_of(&base), hash_of(&same));
        prop_assert_ne!(key(0.95), key(0.96));
        prop_assert_ne!(hash_of(&key(0.95)), hash_of(&key(0.96)));
        prop_assert_ne!(&base, &different);
    }
}

#[test]
fn persist_empty_cache_writes_valid_json() -> Result<()> {
    let path = unique_path("persist_empty_cache");
    let cache = AutotuneCache::load(&path)?;

    cache.persist()?;

    let json = read_string(&path);
    assert!(json.contains("\"entries\""));
    assert!(json.contains("[]"));
    println!(
        "autotune_persist_empty PASSED path={} bytes={} json={}",
        path.display(),
        json.len(),
        json.replace('\n', " ")
    );
    Ok(())
}

#[test]
fn malformed_json_load_returns_cache_error() {
    let path = unique_path("malformed_load");
    fs::write(&path, b"{not-json").expect("write malformed cache JSON");

    let err = AutotuneCache::load(&path).expect_err("malformed JSON must fail closed");

    assert!(matches!(err, ForgeError::CacheError { .. }));
    assert!(err.to_string().contains(&path.display().to_string()));
    println!("autotune_malformed_load PASSED {err}");
}

#[test]
fn rollback_restores_previous() -> Result<()> {
    let path = unique_path("rollback_restores_previous");
    let mut cache = AutotuneCache::load(&path)?;
    let key = key(0.95);
    let current = config(128);
    let previous = config(64);

    cache.insert(key.clone(), current);
    cache.rollback(&key, previous.clone());

    assert_eq!(cache.get(&key), Some(&previous));
    println!(
        "autotune_rollback_restores_previous PASSED op={} tile_m={}",
        key.op, previous.tile_m
    );
    Ok(())
}

#[test]
fn rollback_missing_key_inserts_previous() -> Result<()> {
    let path = unique_path("rollback_missing_key");
    let mut cache = AutotuneCache::load(&path)?;
    let key = key(0.95);
    let previous = config(32);

    cache.rollback(&key, previous.clone());

    assert_eq!(cache.get(&key), Some(&previous));
    println!(
        "autotune_rollback_missing PASSED len={} tile_m={}",
        cache.len(),
        previous.tile_m
    );
    Ok(())
}

#[test]
fn persist_parent_file_fails_closed() -> Result<()> {
    let parent = unique_path("persist_parent_file");
    fs::write(&parent, b"not a directory").expect("create parent file");
    let path = parent.join("cache.json");
    let mut cache = AutotuneCache {
        entries: HashMap::new(),
        path,
    };
    cache.insert(key(0.95), config(16));
    let result = cache.persist();

    fs::remove_file(&parent).expect("remove parent file");

    let err = result.expect_err("file parent must fail closed");
    assert!(matches!(err, ForgeError::CacheError { .. }));
    assert!(err.to_string().contains("CALYX_FORGE_CACHE_ERROR"));
    println!("autotune_persist_parent_file PASSED {err}");
    Ok(())
}
