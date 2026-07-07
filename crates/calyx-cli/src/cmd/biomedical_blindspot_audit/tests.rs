use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::io::persist;
use super::model::BiomedicalBlindspotAuditArgs;
use super::report::build_report;

#[test]
fn blindspot_audit_blocks_known_context_inversions_and_preserves_ready_rows() {
    let root = test_root("happy");
    seed_inputs(&root);
    let args = args(&root);
    let report = build_report(&args).expect("build report");
    assert_eq!(report.audited_count, 4);
    assert_eq!(report.ready_count, 1);
    assert_eq!(report.blocked_count, 3);
    assert_eq!(report.pending_count, 0);
    let readback = persist(&root.join("out"), &report).expect("persist");
    assert_eq!(readback.ready_hypotheses_rows, 1);
    assert_eq!(readback.blocked_hypotheses_rows, 3);
    let blocked = jsonl(&root.join("out").join("blocked_hypotheses.jsonl"));
    assert!(blocked.iter().any(|row| {
        row["reason_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "CALYX_BLINDSPOT_GERMLINE_SYNTHETIC_LETHALITY_RISK")
    }));
    assert!(blocked.iter().any(|row| {
        row["reason_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "CALYX_BLINDSPOT_DRUG_NOT_VIABLE")
    }));
    assert!(blocked.iter().any(|row| {
        row["reason_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "CALYX_BLINDSPOT_TRANSCRIPTOMIC_LOW_SPECIFICITY")
    }));
    let generic = blocked
        .iter()
        .find(|row| row["hypothesis_id"] == "generic-hdac-reversal")
        .expect("generic reversal row");
    assert!(
        !generic["reason_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "CALYX_BLINDSPOT_DRUG_LIFECYCLE_MISSING")
    );
}

#[test]
fn missing_required_context_persists_pending_state() {
    let root = test_root("pending");
    seed_inputs(&root);
    let report_path = root.join("hypotheses.json");
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&json!({
            "hypotheses": [{
                "hypothesis_id": "missing-context",
                "source_name": "olaparib",
                "source_type": "chemical",
                "target_name": "Fanconi anemia",
                "target_type": "disease",
                "drug_names": ["olaparib"],
                "target_names": ["BRCA2"],
                "disease_names": ["Fanconi anemia"],
                "score": 0.7,
                "novelty_score": 0.8
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let args = args(&root);
    let report = build_report(&args).expect("build report");
    assert_eq!(report.pending_count, 1);
    let readback = persist(&root.join("out"), &report).expect("persist");
    assert_eq!(readback.blocked_hypotheses_rows, 1);
    let row = &jsonl(&root.join("out").join("blocked_hypotheses.jsonl"))[0];
    assert_eq!(row["final_status"], "pending_blindspot_evidence");
    assert!(
        row["reason_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "CALYX_BLINDSPOT_PATIENT_CONTEXT_MISSING")
    );
}

#[test]
fn malformed_source_rows_fail_closed() {
    let root = test_root("malformed");
    seed_inputs(&root);
    fs::write(root.join("drug_lifecycle.jsonl"), "{}\n").unwrap();
    let error = build_report(&args(&root)).expect_err("missing drug name must fail");
    assert!(
        error
            .message()
            .contains("drug_lifecycle line 1 missing drug_name/name")
    );
}

fn seed_inputs(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
            root.join("hypotheses.json"),
            serde_json::to_vec_pretty(&json!({
                "hypotheses": [
                    {
                        "hypothesis_id": "braf-trametinib-cfc",
                        "source_name": "trametinib",
                        "source_type": "chemical",
                        "target_name": "cardiofaciocutaneous syndrome",
                        "target_type": "disease",
                        "drug_names": ["trametinib"],
                        "target_names": ["BRAF"],
                        "disease_names": ["cardiofaciocutaneous syndrome"],
                        "patient_context": "germline RASopathy with gain-of-function pathway activation",
                        "therapeutic_rationale": "MEK inhibition dampens over-active RAS/MAPK signaling",
                        "score": 0.91,
                        "novelty_score": 0.72
                    },
                    {
                        "hypothesis_id": "olaparib-fanconi-brca2",
                        "source_name": "olaparib",
                        "source_type": "chemical",
                        "target_name": "Fanconi anemia",
                        "target_type": "disease",
                        "drug_names": ["olaparib"],
                        "target_names": ["BRCA2"],
                        "disease_names": ["Fanconi anemia"],
                        "patient_context": "germline biallelic BRCA2 deficiency present in every cell",
                        "therapeutic_rationale": "synthetic lethality selectively kills BRCA deficient cells",
                        "score": 0.89,
                        "novelty_score": 0.82
                    },
                    {
                        "hypothesis_id": "tarextumab-cadasil-notch3",
                        "source_name": "tarextumab",
                        "source_type": "chemical",
                        "target_name": "CADASIL cerebral arteriopathy",
                        "target_type": "disease",
                        "drug_names": ["tarextumab"],
                        "target_names": ["NOTCH3"],
                        "disease_names": ["CADASIL cerebral arteriopathy"],
                        "patient_context": "germline NOTCH3 vascular disease",
                        "therapeutic_rationale": "anti-Notch2/3 target-name match",
                        "score": 0.87,
                        "novelty_score": 0.83
                    },
                    {
                        "hypothesis_id": "generic-hdac-reversal",
                        "source_name": "HDAC inhibitor class",
                        "source_type": "chemical",
                        "target_name": "inflammatory disease signature",
                        "target_type": "disease",
                        "drug_names": ["vorinostat"],
                        "disease_names": ["inflammatory disease signature"],
                        "patient_context": "somatic cell-line signature",
                        "candidate_type": "transcriptomic_reversal",
                        "evidence_type": "LINCS transcriptomic reversal",
                        "therapeutic_rationale": "broad HDAC inhibitor reversal signature",
                        "score": 0.44,
                        "novelty_score": 0.66
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();
    write_jsonl(
        &root.join("literature.jsonl"),
        &[
            json!({"hypothesis_id":"braf-trametinib-cfc","co_mention_count":0,"publication_count":0,"source_system":"Europe PMC","query":"trametinib BRAF cardiofaciocutaneous"}),
            json!({"hypothesis_id":"olaparib-fanconi-brca2","co_mention_count":4,"publication_count":4,"source_system":"PubMed","query":"olaparib Fanconi BRCA2"}),
            json!({"hypothesis_id":"tarextumab-cadasil-notch3","co_mention_count":0,"publication_count":0,"source_system":"Europe PMC","query":"tarextumab CADASIL NOTCH3"}),
            json!({"hypothesis_id":"generic-hdac-reversal","co_mention_count":1,"publication_count":1,"source_system":"LINCS literature snapshot","query":"HDAC inflammatory reversal"}),
        ],
    );
    write_jsonl(
        &root.join("stability.jsonl"),
        &[
            json!({"hypothesis_id":"braf-trametinib-cfc","run_count":3,"present_count":3}),
            json!({"hypothesis_id":"olaparib-fanconi-brca2","run_count":3,"present_count":3}),
            json!({"hypothesis_id":"tarextumab-cadasil-notch3","run_count":3,"present_count":3}),
            json!({"hypothesis_id":"generic-hdac-reversal","run_count":3,"present_count":3}),
        ],
    );
    write_jsonl(
        &root.join("drug_lifecycle.jsonl"),
        &[
            json!({"drug_name":"trametinib","lifecycle_status":"approved_active","max_phase":4,"source_system":"ChEMBL"}),
            json!({"drug_name":"olaparib","lifecycle_status":"approved_active","max_phase":4,"source_system":"ChEMBL"}),
            json!({"drug_name":"tarextumab","lifecycle_status":"discontinued","trial_status":"terminated","source_system":"ClinicalTrials.gov"}),
            json!({"drug_name":"vorinostat","lifecycle_status":"approved_active","max_phase":4,"source_system":"ChEMBL"}),
        ],
    );
    write_jsonl(
        &root.join("transcriptomic.jsonl"),
        &[json!({
            "hypothesis_id":"generic-hdac-reversal",
            "perturbagen_id":"BRD-K81418486",
            "signature_id":"sig_generic",
            "cell_context":"many",
            "mechanism_class":"HDAC inhibitor",
            "class_breadth":250,
            "is_gold":false,
            "reproducible":false,
            "self_connected":false
        })],
    );
}

fn args(root: &Path) -> BiomedicalBlindspotAuditArgs {
    BiomedicalBlindspotAuditArgs {
        hypotheses_reports: vec![root.join("hypotheses.json")],
        literature_audit: root.join("literature.jsonl"),
        stability_audit: root.join("stability.jsonl"),
        drug_lifecycle: root.join("drug_lifecycle.jsonl"),
        transcriptomic_audit: root.join("transcriptomic.jsonl"),
        out_dir: root.join("out"),
        known_literature_threshold: 3,
        min_stability_frequency: 0.67,
        max_transcriptomic_class_breadth: 25,
    }
}

fn write_jsonl(path: &Path, rows: &[Value]) {
    let mut bytes = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut bytes, row).unwrap();
        bytes.push(b'\n');
    }
    fs::write(path, bytes).unwrap();
}

fn jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn test_root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-biomedical-blindspot-{name}-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    root
}
