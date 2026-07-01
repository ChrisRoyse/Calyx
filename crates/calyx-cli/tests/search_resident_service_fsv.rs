use std::fs;
use std::path::PathBuf;
use std::process::Command;

use calyx_core::VaultId;
use serde_json::{Value, json};

#[path = "support/search_resident_fsv.rs"]
mod search_resident_fsv;
use search_resident_fsv::*;

#[test]
fn search_uses_vault_resident_service_and_reads_back_physical_state() {
    let root = temp_root("search-resident-fsv");
    fs::create_dir_all(&root).expect("create search-resident FSV root");
    let template = "search-resident-fsv";
    write_algorithmic_catalog(&root, 10);
    run_ok(
        Command::new(calyx_exe())
            .arg("panel")
            .arg("template")
            .arg("save")
            .arg("--home")
            .arg(&root)
            .arg("--name")
            .arg(template)
            .arg("--all-current")
            .arg("--modality")
            .arg("text"),
        "save search-resident template",
    );
    let create = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("create-vault")
            .arg("search-resident-vault")
            .arg("--panel-template")
            .arg(template),
        "create search-resident vault",
    );
    let create_json: Value = serde_json::from_slice(&create.stdout).expect("parse create-vault");
    let vault_id: VaultId = create_json["vault_id"].as_str().unwrap().parse().unwrap();
    let vault_path = root.join("vaults").join(vault_id.to_string());
    println!("search_resident_fsv_root={}", root.display());
    println!("search_resident_fsv_vault={}", vault_path.display());

    let before = cf_state(&vault_path, vault_id, "search-resident-vault");
    println!("search_resident_fsv_before={before}");

    let template_progress = root.join("resident-template-progress.jsonl");
    let mut template_service = spawn_template_resident_service(&root, template, &template_progress);
    let template_ready = read_ready(&mut template_service);
    let template_addr = template_ready["bind"].as_str().unwrap().to_string();

    let batch = write_batch(
        &root,
        "search-resident.jsonl",
        &[
            "calyx resident search alpha exact marker 4901",
            "calyx resident search beta distractor 8802",
            "calyx resident search gamma distractor 7703",
        ],
    );
    let ingest = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("ingest")
            .arg(&vault_path)
            .arg("--batch")
            .arg(&batch)
            .arg("--resident-addr")
            .arg(&template_addr),
        "ingest search-resident corpus",
    );
    let after_ingest = cf_state(&vault_path, vault_id, "search-resident-vault");
    println!("search_resident_fsv_after_ingest={after_ingest}");
    let rebuild = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("rebuild-search-index")
            .arg(&vault_path),
        "rebuild search-resident index",
    );
    let rebuild_json = parse_json(&rebuild.stdout);
    let rebuild_progress_path = PathBuf::from(
        rebuild_json["progress_artifact"]
            .as_str()
            .expect("rebuild progress artifact path"),
    );
    let rebuild_progress_text = fs::read_to_string(&rebuild_progress_path)
        .expect("read rebuild progress artifact from source of truth");
    assert!(rebuild_progress_text.contains(r#""phase":"base_scan_page""#));
    assert!(rebuild_progress_text.contains(r#""phase":"manifest_write_ok""#));
    assert!(rebuild_progress_path.is_file());
    let after_rebuild = cf_state(&vault_path, vault_id, "search-resident-vault");
    let index = search_index_state(&vault_path);
    println!("search_resident_fsv_after_rebuild={after_rebuild}");
    println!("search_resident_fsv_index={index}");
    println!("search_resident_fsv_rebuild_progress={rebuild_progress_text}");

    let cold_local = run_fail(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("search")
            .arg(&vault_path)
            .arg("calyx resident search alpha exact marker 4901")
            .arg("--k")
            .arg("1"),
        "cold local GPU search without resident",
    );
    let after_cold_local = cf_state(&vault_path, vault_id, "search-resident-vault");
    println!(
        "search_resident_fsv_edge_cold_local status={:?} stderr={} before={} after={}",
        cold_local.status.code(),
        String::from_utf8_lossy(&cold_local.stderr),
        after_rebuild,
        after_cold_local
    );

    let missing_addr = unused_loopback_addr();
    let unavailable = run_fail(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("search")
            .arg(&vault_path)
            .arg("calyx resident search alpha exact marker 4901")
            .arg("--k")
            .arg("1")
            .arg("--resident-addr")
            .arg(&missing_addr),
        "search with unavailable resident",
    );
    let after_unavailable = cf_state(&vault_path, vault_id, "search-resident-vault");
    println!(
        "search_resident_fsv_edge_unavailable status={:?} stderr={} before={} after={}",
        unavailable.status.code(),
        String::from_utf8_lossy(&unavailable.stderr),
        after_rebuild,
        after_unavailable
    );

    let mismatch = run_fail(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("search")
            .arg(&vault_path)
            .arg("calyx resident search alpha exact marker 4901")
            .arg("--k")
            .arg("1")
            .arg("--resident-addr")
            .arg(&template_addr),
        "search with template-resident mismatch",
    );
    let after_mismatch = cf_state(&vault_path, vault_id, "search-resident-vault");
    println!(
        "search_resident_fsv_edge_mismatch status={:?} stderr={} before={} after={}",
        mismatch.status.code(),
        String::from_utf8_lossy(&mismatch.stderr),
        after_rebuild,
        after_mismatch
    );
    let template_service_output = stop_resident_service(&template_addr, template_service);

    let vault_progress = root.join("resident-vault-progress.jsonl");
    let mut vault_service = spawn_vault_resident_service(&root, &vault_path, &vault_progress);
    let vault_ready = read_ready(&mut vault_service);
    let vault_addr = vault_ready["bind"].as_str().unwrap().to_string();
    let process_id = vault_ready["process_id"].as_u64().unwrap();
    println!("search_resident_fsv_vault_ready={vault_ready}");

    let search = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("search")
            .arg(&vault_path)
            .arg("calyx resident search alpha exact marker 4901")
            .arg("--k")
            .arg("1")
            .arg("--explain")
            .arg("--resident-addr")
            .arg(&vault_addr),
        "search with vault resident",
    );
    let after_search = cf_state(&vault_path, vault_id, "search-resident-vault");
    let search_hits: Value = serde_json::from_slice(&search.stdout).expect("parse search output");
    println!("search_resident_fsv_search_stdout={search_hits}");
    println!(
        "search_resident_fsv_search_stderr={}",
        String::from_utf8_lossy(&search.stderr)
    );
    println!("search_resident_fsv_after_search={after_search}");
    let vault_service_output = stop_resident_service(&vault_addr, vault_service);
    let vault_progress_text = fs::read_to_string(&vault_progress).unwrap_or_default();
    println!("search_resident_fsv_vault_progress_log={vault_progress_text}");

    assert_eq!(before["base_rows"], 0);
    assert_eq!(after_ingest["base_rows"], 3);
    assert_eq!(after_ingest["slot_00_rows"], 3);
    assert_eq!(after_ingest["slot_09_rows"], 3);
    assert_eq!(after_ingest["ledger_rows"], 1);
    assert_eq!(after_cold_local, after_rebuild);
    assert_eq!(after_unavailable, after_rebuild);
    assert_eq!(after_mismatch, after_rebuild);
    assert_eq!(after_search, after_rebuild);
    assert_index_matches_manifest(&index);
    let hits = search_hits.as_array().expect("search hits array");
    assert_eq!(hits.len(), 1);
    let hit_id = hits[0]["cx_id"].as_str().expect("hit cx_id");
    assert!(
        after_search["base_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == hit_id)
    );
    assert!(hits[0]["provenance"]["ledger_seq"].as_u64().is_some());
    assert!(hits[0]["per_lens"].as_array().unwrap().len() >= 10);
    let search_stderr = String::from_utf8_lossy(&search.stderr);
    assert!(search_stderr.contains("phase=search_resident_service_ok"));
    assert!(search_stderr.contains(&format!("process_id={process_id}")));
    assert!(search_stderr.contains("protocol=binary"));
    assert!(String::from_utf8_lossy(&cold_local.stderr).contains("CALYX_SEARCH_RESIDENT_REQUIRED"));
    assert!(
        String::from_utf8_lossy(&unavailable.stderr).contains("CALYX_PANEL_RESIDENT_UNAVAILABLE")
    );
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("CALYX_SEARCH_RESIDENT_MISMATCH"));
    let vault_service_stderr = String::from_utf8_lossy(&vault_service_output.stderr);
    assert!(vault_service_stderr.contains("phase=measure_batch_binary_request"));
    assert!(vault_service_stderr.contains("phase=measure_batch_binary_response"));
    assert!(vault_progress_text.contains("\"phase\":\"resident_run_start\""));
    assert_eq!(
        vault_progress_text
            .matches("\"phase\":\"probe_start\"")
            .count(),
        10
    );
    assert_eq!(
        vault_progress_text
            .matches("\"phase\":\"probe_ok\"")
            .count(),
        10
    );
    assert!(!vault_progress_text.contains("\"phase\":\"probe_error\""));

    let evidence = json!({
        "source_of_truth": {
            "vault": vault_path,
            "base_ledger_slot_cfs": after_search,
            "search_index": index,
            "evidence_file": root.join("search-resident-fsv-evidence.json"),
        },
        "commands": {
            "ingest_stdout": parse_json(&ingest.stdout),
            "rebuild_stdout": rebuild_json,
            "rebuild_progress_log": rebuild_progress_text,
            "search_stdout": search_hits,
            "search_stderr": String::from_utf8_lossy(&search.stderr),
            "template_service_stderr": String::from_utf8_lossy(&template_service_output.stderr),
            "vault_service_stderr": vault_service_stderr,
            "vault_progress_log": vault_progress_text,
        },
        "states": {
            "before": before,
            "after_ingest": after_ingest,
            "after_rebuild": after_rebuild,
            "after_cold_local": after_cold_local,
            "after_unavailable": after_unavailable,
            "after_mismatch": after_mismatch,
            "after_search": after_search,
        },
        "edges": {
            "cold_local_gpu_without_resident": {
                "status": cold_local.status.code(),
                "stderr": String::from_utf8_lossy(&cold_local.stderr),
            },
            "resident_unavailable": {
                "status": unavailable.status.code(),
                "stderr": String::from_utf8_lossy(&unavailable.stderr),
            },
            "resident_template_source_mismatch": {
                "status": mismatch.status.code(),
                "stderr": String::from_utf8_lossy(&mismatch.stderr),
            },
        },
    });
    let evidence_path = root.join("search-resident-fsv-evidence.json");
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    let evidence_readback: Value =
        serde_json::from_slice(&fs::read(&evidence_path).unwrap()).unwrap();
    println!("search_resident_fsv_evidence={evidence_readback}");
    assert_eq!(evidence_readback["states"]["after_search"], after_rebuild);

    if std::env::var("CALYX_KEEP_SEARCH_RESIDENT_FSV_ROOT").as_deref() == Ok("1") {
        println!("search_resident_fsv_preserved_root={}", root.display());
    } else {
        fs::remove_dir_all(root).ok();
    }
}
