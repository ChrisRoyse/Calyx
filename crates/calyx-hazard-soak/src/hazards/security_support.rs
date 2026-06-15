use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
};
use calyx_forge::{QuantLevel, Quantizer, TurboQuantCodec, new_seed, seed_id_hex};
use calyx_sextant::{
    FreshnessRequirement, HnswIndex, Query, RerankRequest, RerankerClient, SearchEngine,
    SlotIndexMap,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) const START_TS: u64 = 1_800_600_000_000;
pub(super) const MEMTABLE_BYTES: usize = 64 * 1024 * 1024;
const DIM: usize = 32;

pub(super) struct RerankObservation {
    pub request_contained_candidate: bool,
    pub request_text_count: usize,
    pub score: f32,
}

pub(super) fn run_successful_rerank(candidate: &str) -> Result<RerankObservation, String> {
    let server = spawn_reranker("HTTP/1.1 200 OK", r#"{"scores":[0.42]}"#)?;
    let response = RerankerClient::new(server.endpoint.clone(), Duration::from_secs(1))
        .rerank(&RerankRequest::new(
            "ph59 privacy query",
            vec![candidate.to_string()],
        ))
        .map_err(|error| error.to_string())?;
    server.join()?;
    let request = server.request()?;
    let texts = request_texts(request_body(&request)?)?;
    Ok(RerankObservation {
        request_contained_candidate: texts.first().is_some_and(|text| text == candidate),
        request_text_count: texts.len(),
        score: response.scores.first().copied().unwrap_or_default(),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct HitSignature {
    pub cx_id: String,
    pub rank: usize,
    pub score_bits: u32,
}

#[derive(Clone)]
pub(super) struct ReplayObservation {
    pub determinism_enabled: bool,
    pub seed_id: String,
    pub quantized_bytes: Vec<u8>,
    pub decoded_vector: Vec<f32>,
    pub serialized: Vec<u8>,
    hits: Vec<HitSignature>,
}

impl ReplayObservation {
    pub(super) fn summary(&self) -> Value {
        json!({
            "determinism_enabled": self.determinism_enabled,
            "seed_id": self.seed_id,
            "quantized_bytes": self.quantized_bytes.len(),
            "result_sha256": hash_hex(&self.serialized),
            "hits": self.hits
        })
    }
}

pub(super) fn replay_observation(text: &str) -> Result<ReplayObservation, String> {
    let determinism_enabled = env::var("CALYX_DETERMINISM").ok().as_deref() == Some("1");
    let query_vec = deterministic_vector(23, DIM);
    let seed = new_seed(DIM, b"ph59-h23-determinism");
    let codec =
        TurboQuantCodec::new(seed.clone(), QuantLevel::Bits3p5).map_err(|e| e.to_string())?;
    let quantized = codec.encode(&query_vec).map_err(|e| e.to_string())?;
    let decoded_vector = codec.decode(&quantized).map_err(|e| e.to_string())?;
    let hits = search_hits(text, query_vec)?;
    let serialized = serde_json::to_vec(&json!({
        "determinism_enabled": determinism_enabled,
        "seed_id": seed_id_hex(&quantized.seed_id),
        "quantized_hex": hex_bytes(&quantized.bytes),
        "hits": hits
    }))
    .map_err(|error| error.to_string())?;
    Ok(ReplayObservation {
        determinism_enabled,
        seed_id: seed_id_hex(&seed.id),
        quantized_bytes: quantized.bytes,
        decoded_vector,
        serialized,
        hits,
    })
}

pub(super) fn search_hits(text: &str, query_vec: Vec<f32>) -> Result<Vec<HitSignature>, String> {
    let slot = SlotId::new(23);
    let indexes = SlotIndexMap::new();
    indexes
        .register(HnswIndex::new(slot, DIM as u32, 23))
        .map_err(|error| error.to_string())?;
    for seed in 1..=8_u8 {
        indexes
            .insert(
                slot,
                cx(seed),
                dense_slot(deterministic_vector(seed, DIM)),
                u64::from(seed),
            )
            .map_err(|error| error.to_string())?;
    }
    let mut engine = SearchEngine::new(indexes);
    for seed in 1..=8_u8 {
        engine.put_constellation(constellation(seed, 23));
    }
    let mut query = Query::new(text.to_string())
        .with_vector(dense_slot(query_vec))
        .with_slots(vec![slot]);
    query.k = 3;
    query.ef = Some(8);
    query.freshness = FreshnessRequirement::StaleOk { seq_lag: 0 };
    Ok(engine
        .search(&query)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|hit| HitSignature {
            cx_id: hit.cx_id.to_string(),
            rank: hit.rank,
            score_bits: hit.score.to_bits(),
        })
        .collect())
}

pub(super) fn with_determinism<T>(
    value: &str,
    f: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let previous = env::var_os("CALYX_DETERMINISM");
    // SAFETY: calyx-hazard-soak is single-threaded while invoking this probe; the
    // variable is restored before returning.
    unsafe {
        env::set_var("CALYX_DETERMINISM", value);
    }
    let result = f();
    unsafe {
        match previous {
            Some(previous) => env::set_var("CALYX_DETERMINISM", previous),
            None => env::remove_var("CALYX_DETERMINISM"),
        }
    }
    result
}

pub(super) fn max_abs_delta(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0_f32, f32::max)
}

struct TestServer {
    endpoint: String,
    request: Arc<Mutex<String>>,
    handle: Mutex<Option<JoinHandle<Result<(), String>>>>,
}

impl TestServer {
    fn request(&self) -> Result<String, String> {
        self.request
            .lock()
            .map(|guard| guard.clone())
            .map_err(|error| format!("read reranker request lock: {error}"))
    }

    fn join(&self) -> Result<(), String> {
        let Some(handle) = self
            .handle
            .lock()
            .map_err(|error| format!("join reranker lock: {error}"))?
            .take()
        else {
            return Ok(());
        };
        handle
            .join()
            .map_err(|_| "reranker thread panicked".to_string())?
    }
}

fn spawn_reranker(status: &'static str, body: &'static str) -> Result<TestServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let endpoint = format!(
        "http://{}",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let request = Arc::new(Mutex::new(String::new()));
    let request_for_thread = Arc::clone(&request);
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    bytes.extend_from_slice(&chunk[..read]);
                    if http_request_complete(&bytes) {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(format!("read reranker request: {error}")),
            }
        }
        *request_for_thread
            .lock()
            .map_err(|error| format!("store reranker request: {error}"))? =
            String::from_utf8(bytes).map_err(|error| error.to_string())?;
        let response = format!(
            "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .map_err(|error| error.to_string())
    });
    Ok(TestServer {
        endpoint,
        request,
        handle: Mutex::new(Some(handle)),
    })
}

fn http_request_complete(bytes: &[u8]) -> bool {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_len = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    bytes.len() >= header_end + 4 + content_len
}

fn request_body(request: &str) -> Result<&str, String> {
    request
        .split("\r\n\r\n")
        .nth(1)
        .ok_or_else(|| "reranker request body missing".to_string())
}

fn request_texts(body: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    Ok(value["texts"]
        .as_array()
        .ok_or_else(|| "reranker texts array missing".to_string())?
        .iter()
        .map(|value| value.as_str().unwrap_or_default().to_string())
        .collect())
}

pub(super) fn deterministic_vector(seed: u8, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|idx| {
            let raw = (u32::from(seed) * 37 + idx as u32 * 17 + 11) % 997;
            raw as f32 / 997.0
        })
        .collect()
}

pub(super) fn dense_slot(values: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: values.len() as u32,
        data: values,
    }
}

pub(super) fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

pub(super) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid PH59 soak vault id")
}

pub(super) fn constellation(seed: u8, panel_version: u32) -> Constellation {
    Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version,
        created_at: START_TS + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("zfs://calyx/ph59/security/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

pub(super) fn scan_dir_for_bytes(root: &Path, needle: &[u8]) -> Result<Vec<String>, String> {
    let mut hits = Vec::new();
    for path in list_files_abs(root)? {
        if fs::read(&path)
            .map_err(|error| format!("read scan file {}: {error}", path.display()))?
            .windows(needle.len())
            .any(|window| window == needle)
        {
            hits.push(relative_path(root, &path));
        }
    }
    hits.sort();
    Ok(hits)
}

pub(super) fn list_files(root: &Path) -> Result<Vec<String>, String> {
    let mut files = list_files_abs(root)?
        .into_iter()
        .map(|path| relative_path(root, &path))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn list_files_abs(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in
            fs::read_dir(&dir).map_err(|error| format!("read dir {}: {error}", dir.display()))?
        {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(super) fn hash_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub(super) fn hash_bytes(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
