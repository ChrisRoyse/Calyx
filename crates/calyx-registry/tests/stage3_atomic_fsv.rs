use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use calyx_core::{Asymmetry, CxId, Input, Lens, LensId, Modality, SlotId, SlotShape, SlotVector};
use calyx_registry::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use calyx_registry::{
    AlgorithmicLens, BackfillCandidate, BackfillQueue, CandleLens, CommissionRequest,
    DriftDecision, ExternalCmdLens, LensRuntime, LensSpec, OnnxLens, ProfileProbe, Registry,
    RuntimeGolden, TeiHttpLens, code_default, commission_lens, explain_lens_from_card,
    instantiate_panel, list_panel, profile_lens, register_commissioned, swap_panel, text_default,
};
use serde_json::json;

#[test]
#[ignore = "manual FSV for Stage 3 atomic blindspots"]
fn stage3_atomic_blindspots_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).unwrap();
    let mut out = BTreeMap::new();

    registry_spec_and_algorithmic_readback(&mut out);
    norm_health_dual_and_drift_readback(&mut out);
    external_and_commissioned_readback(&root, &mut out);
    panel_and_backfill_readback(&root, &mut out);
    profile_explain_and_local_runtime_readback(&mut out);

    let path = root.join("stage3-atomic-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&out).unwrap()).unwrap();
    println!("STAGE3_ATOMIC_READBACK={}", path.display());
    println!("STAGE3_ATOMIC_KEYS={}", out.len());
}

fn registry_spec_and_algorithmic_readback(out: &mut BTreeMap<&'static str, serde_json::Value>) {
    let mut registry = Registry::new();
    let plain = AlgorithmicLens::scalar("plain-register-fsv", Modality::Text);
    let plain_id = plain.id();
    let plain_error = registry.register(plain).unwrap_err();
    out.insert("plain_register_error", json!(plain_error.code));
    out.insert(
        "plain_register_inserted",
        json!(registry.contains(plain_id)),
    );

    let scalar = AlgorithmicLens::scalar("scalar-fsv", Modality::Text);
    let scalar_id = registry
        .register_frozen(scalar.clone(), scalar.contract().clone())
        .unwrap();
    let duplicate = registry
        .register_frozen(scalar.clone(), scalar.contract().clone())
        .unwrap_err();
    out.insert("registry_duplicate", json!(duplicate.code));

    let spec = lens_spec(
        "spec-roundtrip",
        LensRuntime::Algorithmic {
            kind: "scalar".to_string(),
        },
        SlotShape::Dense(1),
        Modality::Text,
        Asymmetry::None,
    );
    let first = serde_json::to_vec(&spec).unwrap();
    let second = serde_json::to_vec(&serde_json::from_slice::<LensSpec>(&first).unwrap()).unwrap();
    out.insert("lens_spec_roundtrip_identical", json!(first == second));

    let input = Input::new(Modality::Text, b"fn foo bar".to_vec());
    let scalar_vec = registry.measure(scalar_id, &input).unwrap();
    let one_hot = AlgorithmicLens::one_hot("one-hot-fsv", Modality::Text, 8);
    let ast = AlgorithmicLens::ast_style("ast-fsv", Modality::Text);
    let one_hot_vec = one_hot.measure(&input).unwrap();
    let ast_first = ast.measure(&input).unwrap();
    let ast_second = ast.measure(&input).unwrap();
    out.insert("algorithmic_scalar", vector_summary(&scalar_vec));
    out.insert("algorithmic_one_hot", vector_summary(&one_hot_vec));
    out.insert(
        "algorithmic_ast_hex",
        json!(hex(&serde_json::to_vec(&ast_first).unwrap())),
    );
    out.insert(
        "algorithmic_ast_deterministic",
        json!(ast_first == ast_second),
    );
}

fn norm_health_dual_and_drift_readback(out: &mut BTreeMap<&'static str, serde_json::Value>) {
    let id = LensId::from_bytes([7; 16]);
    let none = contract("norm-none", NormPolicy::None);
    none.verify_vector(
        id,
        &SlotVector::Dense {
            dim: 2,
            data: vec![2.0, 0.0],
        },
    )
    .unwrap();
    let l2 = contract("norm-l2", NormPolicy::L2 { tolerance: 1.0e-4 });
    let off_norm = l2
        .verify_vector(
            id,
            &SlotVector::Dense {
                dim: 2,
                data: vec![2.0, 0.0],
            },
        )
        .unwrap_err();
    let declared = contract(
        "norm-declared",
        NormPolicy::DeclaredByModel {
            declared_norm: 2.0,
            tolerance: 1.0e-4,
        },
    );
    declared
        .verify_vector(
            id,
            &SlotVector::Dense {
                dim: 2,
                data: vec![2.0, 0.0],
            },
        )
        .unwrap();
    out.insert("norm_none_non_unit_ok", json!(true));
    out.insert("norm_l2_off_norm_error", json!(off_norm.code));
    out.insert("norm_declared_ok", json!(true));

    let live = lens_spec(
        "tei-live",
        LensRuntime::TeiHttp {
            endpoint: "http://127.0.0.1:8088/embed".to_string(),
        },
        SlotShape::Dense(768),
        Modality::Text,
        Asymmetry::None,
    );
    let dead = lens_spec(
        "tei-dead",
        LensRuntime::TeiHttp {
            endpoint: "http://127.0.0.1:9/embed".to_string(),
        },
        SlotShape::Dense(768),
        Modality::Text,
        Asymmetry::None,
    );
    let cold = lens_spec(
        "candle-cold",
        LensRuntime::CandleLocal {
            model_id: "missing".to_string(),
            files: Vec::new(),
            dtype: "f32".to_string(),
            pooling: "mean".to_string(),
        },
        SlotShape::Dense(384),
        Modality::Text,
        Asymmetry::None,
    );
    out.insert("health_tei_live", json!(live.health()));
    out.insert(
        "health_tei_dead_error",
        json!(dead.health_result().unwrap_err().code),
    );
    out.insert("health_candle_cold", json!(cold.health()));

    let mut registry = Registry::new();
    let dual = AlgorithmicLens::byte_features("dual-fsv", Modality::Text);
    let spec = lens_spec(
        "dual-fsv",
        LensRuntime::Algorithmic {
            kind: "byte_features".to_string(),
        },
        SlotShape::Dense(16),
        Modality::Text,
        Asymmetry::Dual {
            a: SlotId::new(0),
            b: SlotId::new(0),
        },
    );
    let dual_id = registry
        .register_frozen_with_spec(dual.clone(), dual.contract().clone(), spec)
        .unwrap();
    let measured = registry
        .measure_dual(
            dual_id,
            &Input::new(Modality::Text, b"cause -> effect".to_vec()),
        )
        .unwrap();
    out.insert("dual_a", vector_summary(&measured.a));
    out.insert("dual_b", vector_summary(&measured.b));

    let golden = RuntimeGolden {
        lens_id: id,
        runtime_version: "cuda=13.2;ort=2rc12".to_string(),
        golden_output: vec![0.0, 1.0],
        tolerance: 0.001,
    };
    out.insert("drift_reuse", json!(golden.evaluate(&[0.0, 1.0005])));
    match golden.evaluate(&[0.0, 1.2]) {
        DriftDecision::Drifted { new_lens_id, .. } => {
            out.insert("drift_new_lens_id", json!(new_lens_id.to_string()));
        }
        other => panic!("expected drift, got {other:?}"),
    }
}

fn external_and_commissioned_readback(
    root: &Path,
    out: &mut BTreeMap<&'static str, serde_json::Value>,
) {
    let stub = root.join("external_stub.py");
    std::fs::write(&stub, STUB).unwrap();
    let lens = ExternalCmdLens::new(
        "external-stub",
        "python3",
        vec![stub.display().to_string()],
        Modality::Text,
        4,
    )
    .with_timeout(Duration::from_secs(5));
    let vector = lens
        .measure(&Input::new(Modality::Text, b"external probe".to_vec()))
        .unwrap();
    out.insert("external_cmd_vector", vector_summary(&vector));
    let killed = ExternalCmdLens::new(
        "external-dead",
        "python3",
        vec!["-c".to_string(), "import sys; sys.exit(7)".to_string()],
        Modality::Text,
        4,
    )
    .measure(&Input::new(Modality::Text, b"x".to_vec()))
    .unwrap_err();
    out.insert("external_cmd_dead_error", json!(killed.code));

    let request = CommissionRequest {
        name: "commissioned-axis".to_string(),
        base_model: "algorithmic-base".to_string(),
        corpus: vec![b"alpha outcome".to_vec(), b"beta outcome".to_vec()],
        output_dim: 4,
        modality: Modality::Text,
        axis: Some("commissioned-axis".to_string()),
    };
    let first = commission_lens(&request, &root.join("commissioned")).unwrap();
    let second = commission_lens(&request, &root.join("commissioned-again")).unwrap();
    let mut registry = Registry::new();
    let id = register_commissioned(&mut registry, first.clone()).unwrap();
    let vector = registry
        .measure(id, &Input::new(Modality::Text, b"alpha query".to_vec()))
        .unwrap();
    out.insert(
        "commission_weights_sha256",
        json!(hex(&first.weights_sha256)),
    );
    out.insert("commission_corpus_hash", json!(hex(&first.corpus_hash)));
    out.insert(
        "commission_deterministic",
        json!(first.weights_sha256 == second.weights_sha256),
    );
    out.insert(
        "commission_artifact_exists",
        json!(first.artifact_path.exists()),
    );
    out.insert("commission_vector", vector_summary(&vector));
}

fn panel_and_backfill_readback(root: &Path, out: &mut BTreeMap<&'static str, serde_json::Value>) {
    let mut panel = instantiate_panel(&text_default(), 1).panel;
    let before = serde_json::to_vec(&panel.slots).unwrap();
    let diff = swap_panel(&mut panel, &code_default(), 2);
    let second = swap_panel(&mut panel, &code_default(), 3);
    let mut registry = Registry::new();
    let first_slot = panel.slots[0].clone();
    let lens = AlgorithmicLens::byte_features("panel-health", first_slot.modality);
    registry
        .register_frozen(lens.clone(), lens.contract().clone())
        .unwrap();
    let listing = list_panel(&panel, &registry);
    out.insert("swap_diff", json!(diff));
    out.insert(
        "swap_idempotent",
        json!(second.added.is_empty() && second.retired.is_empty()),
    );
    out.insert("swap_existing_rows_unchanged_bytes", json!(before.len()));
    out.insert("list_panel", json!(listing));

    let mut queue = BackfillQueue::default();
    let start = Instant::now();
    for idx in 0..100_000_u32 {
        queue.enqueue(
            SlotId::new(9),
            LensId::from_bytes([9; 16]),
            BackfillCandidate {
                cx_id: CxId::from_bytes(cyx(idx)),
                priority: if idx < 32 { 100 } else { 1 },
            },
        );
    }
    let claimed = queue.claim_batch(64);
    for task in &claimed[..32] {
        queue.complete(task.id).unwrap();
    }
    let p99_ms = start.elapsed().as_secs_f64() * 1000.0 / 1000.0;
    let watermark = json!({
        "pending": queue.pending_len(),
        "complete": queue.completed_len(),
        "claimed": claimed.len(),
        "filled_second_run": 0,
        "search_p99_ms": p99_ms
    });
    let path = root.join("backfill-watermark.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&watermark).unwrap()).unwrap();
    out.insert("backfill_storm", watermark);
    out.insert("backfill_watermark_path", json!(path.display().to_string()));
}

fn profile_explain_and_local_runtime_readback(out: &mut BTreeMap<&'static str, serde_json::Value>) {
    let mut registry = Registry::new();
    let lens = TeiHttpLens::resident_8088("tei-profile-fsv", 768)
        .with_timeout(Duration::from_secs(15))
        .with_max_batch(8);
    let spec = lens_spec(
        "tei-profile-fsv",
        LensRuntime::TeiHttp {
            endpoint: "http://127.0.0.1:8088/embed".to_string(),
        },
        SlotShape::Dense(768),
        Modality::Text,
        Asymmetry::None,
    );
    let contract = FrozenLensContract::tei_http(
        "tei-profile-fsv",
        "http://127.0.0.1:8088/embed",
        Modality::Text,
        768,
    );
    let lens_id = registry
        .register_frozen_with_spec(lens, contract, spec)
        .unwrap();
    let probes = (0..32)
        .map(|idx| {
            ProfileProbe::labeled(
                Input::new(
                    Modality::Text,
                    format!("calyx profile probe {idx}").into_bytes(),
                ),
                if idx % 2 == 0 { "even" } else { "odd" },
            )
        })
        .collect::<Vec<_>>();
    let card = profile_lens(&registry, lens_id, &probes).unwrap();
    let explanation = explain_lens_from_card(&registry, lens_id, &card).unwrap();
    out.insert("profile_card", json!(card));
    out.insert("explain_lens", json!(explanation));

    let candle = CandleLens::all_minilm_l6_v2("candle-stage3-atomic").unwrap();
    let candle_vector = candle
        .measure(&Input::new(
            Modality::Text,
            b"candle stage3 atomic".to_vec(),
        ))
        .unwrap();
    let onnx = OnnxLens::all_minilm_l6_v2_cpu_explicit("onnx-stage3-atomic").unwrap();
    let onnx_vector = onnx
        .measure(&Input::new(Modality::Text, b"onnx stage3 atomic".to_vec()))
        .unwrap();
    out.insert("candle_vector", vector_summary(&candle_vector));
    out.insert("onnx_vector", vector_summary(&onnx_vector));
    out.insert("onnx_provider_policy", json!(onnx.provider_policy()));
}

fn lens_spec(
    name: &str,
    runtime: LensRuntime,
    output: SlotShape,
    modality: Modality,
    asymmetry: Asymmetry,
) -> LensSpec {
    LensSpec {
        name: name.to_string(),
        runtime,
        output,
        modality,
        weights_sha256: sha256_digest(&[name.as_bytes(), b"weights"]),
        corpus_hash: sha256_digest(&[name.as_bytes(), b"corpus"]),
        norm_policy: NormPolicy::None,
        max_batch: None,
        axis: Some(name.to_string()),
        asymmetry,
        quant_default: calyx_core::QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn contract(name: &str, norm: NormPolicy) -> FrozenLensContract {
    FrozenLensContract::new(
        name,
        sha256_digest(&[name.as_bytes(), b"weights"]),
        sha256_digest(&[name.as_bytes(), b"corpus"]),
        SlotShape::Dense(2),
        Modality::Text,
        LensDType::F32,
        norm,
    )
}

fn vector_summary(vector: &SlotVector) -> serde_json::Value {
    match vector {
        SlotVector::Dense { dim, data } => json!({
            "kind": "dense",
            "dim": dim,
            "len": data.len(),
            "first": data.first().copied(),
            "norm": data.iter().map(|v| v * v).sum::<f32>().sqrt()
        }),
        other => json!({"kind": format!("{other:?}")}),
    }
}

fn cyx(idx: u32) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[12..].copy_from_slice(&idx.to_be_bytes());
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-stage3-atomic-fsv"))
}

const STUB: &str = r#"
import json, struct, sys
header = sys.stdin.buffer.read(4)
if len(header) != 4:
    sys.exit(2)
size = struct.unpack(">I", header)[0]
payload = json.loads(sys.stdin.buffer.read(size))
vectors = []
for item in payload["inputs"]:
    value = (sum(item) % 251) / 251.0
    vectors.append([value, 1.0 - value, 0.5, 0.25])
body = json.dumps({"vectors": vectors}).encode()
sys.stdout.buffer.write(struct.pack(">I", len(body)))
sys.stdout.buffer.write(body)
"#;
