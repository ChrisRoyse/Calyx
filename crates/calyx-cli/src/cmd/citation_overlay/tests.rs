use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use serde_json::{Value, json};
use ulid::Ulid;

use super::{
    DEFAULT_COLLECTION, EDGE_TYPE, MaterializeCitationOverlayArgs, materialize_with_home,
    parse_materialize_citation_overlay, preflight_report_paths,
};
use crate::cmd::Subcommand;
use crate::cmd::opinion_alias_overlay;
use crate::cmd::vault::{resolve_vault_info, vault_salt};

#[test]
fn parses_required_and_optional_flags() {
    let args = [
        "legal-cuyahoga",
        "--idmap",
        "/fsv/idmap.csv",
        "--citations",
        "/fsv/citations_cuyahoga.csv",
        "--collection",
        "legal-citations-v1",
        "--skip-report",
        "/fsv/skips.json",
        "--report",
        "/fsv/report.json",
        "--home",
        "/home/calyx",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    let parsed = parse_materialize_citation_overlay(&args).expect("parse command");
    assert_eq!(
        parsed,
        Subcommand::MaterializeCitationOverlay(MaterializeCitationOverlayArgs {
            vault: "legal-cuyahoga".to_string(),
            idmap: "/fsv/idmap.csv".into(),
            citations: "/fsv/citations_cuyahoga.csv".into(),
            frontier: None,
            frontier_authorities: None,
            collection: Some("legal-citations-v1".to_string()),
            skip_report: Some("/fsv/skips.json".into()),
            report: Some("/fsv/report.json".into()),
            home: Some("/home/calyx".into()),
        })
    );
}

#[test]
fn rejects_missing_idmap() {
    let args = ["legal-cuyahoga", "--citations", "/fsv/c.csv"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let error = parse_materialize_citation_overlay(&args).expect_err("missing idmap");
    assert!(error.message().contains("--idmap"));
}

#[test]
fn rejects_report_path_aliases_before_materialization() {
    let home = temp_root("report-alias");
    fs::create_dir_all(home.join("alias-parent")).expect("create physical parent");
    let report = home.join("report.json");
    let alias = home.join("alias-parent").join("..").join("report.json");
    let error = preflight_report_paths(&MaterializeCitationOverlayArgs {
        vault: "legal-cuyahoga".to_string(),
        idmap: home.join("idmap.csv"),
        citations: home.join("citations.csv"),
        frontier: None,
        frontier_authorities: None,
        collection: None,
        skip_report: Some(report),
        report: Some(alias),
        home: None,
    })
    .expect_err("canonical aliases must fail before DB mutation");

    assert!(error.message().contains("must identify distinct"));
    fs::remove_dir_all(&home).expect("clean physical parent");
}

fn temp_root(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-citation-overlay-{tag}-{}-{}",
        std::process::id(),
        Ulid::new()
    ))
}

/// Creates a real durable vault + CLI index so the overlay materializer can
/// open it exactly like the production Cuyahoga vault (#1469) would present.
fn make_vault(home: &Path, name: &str) -> VaultId {
    let vault_id = VaultId::from_ulid(Ulid::new());
    let relative = format!("vaults/{vault_id}");
    let vault_dir = home.join(&relative);
    AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions::default(),
    )
    .expect("create durable vault");
    let index = json!({
        "vaults": [ {
            "name": name,
            "vault_id": vault_id.to_string(),
            "path": relative,
            "panel_template": "legal-v1",
        } ]
    });
    let index_path = home.join("vaults").join("index.json");
    fs::write(&index_path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();
    vault_id
}

/// Seeds the accepted DB-native opinion-alias relation that citation-overlay
/// materialization now requires as a precondition (#1629): every idmap opinion
/// must be proven against the `legal-opinion-aliases-v1` collection before any
/// `cites` edge is written. Ingests one canonical constellation per opinion into
/// Base, then drives the real `materialize-opinion-aliases` command so the
/// alias Graph generation is produced exactly as production would. Returns the
/// resolved `opinion_id -> canonical cx_id` map used to build the citation idmap.
fn seed_alias_relation(home: &Path, vault_name: &str, opinions: &[&str]) -> BTreeMap<String, CxId> {
    let resolved = resolve_vault_info(home, vault_name).expect("resolve seeded vault");
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )
    .expect("open vault for alias seeding");

    let mut map = BTreeMap::new();
    let mut csv = String::from(
        "opinion_id,cx_id,canonical_opinion_id,content_sha256,is_canonical,source_url\n",
    );
    for opinion in opinions {
        let input = format!("canonical-opinion-{opinion}").into_bytes();
        let cx_id = vault.cx_id_for_input(&input, 1);
        let content_sha256 = opinion_alias_overlay::sha256_hex(&input);
        let metadata = BTreeMap::from([
            ("opinion_id".to_string(), (*opinion).to_string()),
            ("canonical_opinion_id".to_string(), (*opinion).to_string()),
            ("ingest_text_sha256".to_string(), content_sha256.clone()),
        ]);
        let constellation = Constellation {
            cx_id,
            vault_id: resolved.vault_id,
            panel_version: 1,
            created_at: 1,
            input_ref: InputRef {
                hash: *blake3::hash(&input).as_bytes(),
                pointer: None,
                redacted: false,
            },
            modality: Modality::Text,
            slots: BTreeMap::new(),
            scalars: BTreeMap::new(),
            metadata,
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags::default(),
        };
        vault
            .put(constellation)
            .expect("put canonical constellation into Base");
        csv.push_str(&format!(
            "{opinion},{cx_id},{opinion},{content_sha256},true,https://www.courtlistener.com/opinion/{opinion}/seed/\n"
        ));
        map.insert((*opinion).to_string(), cx_id);
    }
    vault.flush().expect("flush seeded Base constellations");
    drop(vault);

    let aliases_path = home.join(format!("opinion-aliases-{vault_name}.csv"));
    fs::write(&aliases_path, csv).expect("write opinion-alias CSV");
    opinion_alias_overlay::try_run(&[
        "materialize-opinion-aliases".to_string(),
        vault_name.to_string(),
        "--aliases".to_string(),
        aliases_path.display().to_string(),
        "--home".to_string(),
        home.display().to_string(),
    ])
    .expect("materialize-opinion-aliases must dispatch")
    .expect("materialize the accepted opinion-alias relation");
    map
}

#[test]
fn materializes_cites_overlay_and_reads_back_from_cf_bytes() {
    let home = temp_root("happy");
    fs::create_dir_all(home.join("vaults")).unwrap();
    let name = "legal-cuyahoga";
    make_vault(&home, name);

    // Four in-slice opinions resolve to existing constellation CxIds through the
    // accepted DB-native opinion-alias relation; a fifth (op-outside) has no cx
    // mapping to force an unresolved skip.
    let aliases = seed_alias_relation(&home, name, &["1001", "1002", "1003", "1004"]);
    let a = aliases["1001"];
    let b = aliases["1002"];
    let c = aliases["1003"];
    let d = aliases["1004"];
    let idmap = home.join("idmap.csv");
    fs::write(
        &idmap,
        format!("opinion_id,cx_id\n1001,{a}\n1002,{b}\n1003,{c}\n1004,{d}\n"),
    )
    .unwrap();

    // In-slice rows: two clean edges, one depth>10 (capped weight=1.0), one
    // duplicate pair (now coalesced into the base edge, taking the max depth),
    // one zero-depth invalid skip. The out-of-corpus citation 1001 -> 9999 can no
    // longer be an in-slice skip (that is a hard failure now); it must be declared
    // through the separate frontier input.
    let citations = home.join("citations_cuyahoga.csv");
    fs::write(
        &citations,
        "id,depth,citing_opinion_id,cited_opinion_id\n\
         r1,3,1001,1002\n\
         r2,7,1001,1003\n\
         r3,15,1002,1004\n\
         r5,4,1001,1002\n\
         r6,0,1003,1004\n",
    )
    .unwrap();

    // The single out-of-corpus endpoint is declared as a frontier citation.
    let frontier = home.join("frontier_cuyahoga.csv");
    fs::write(
        &frontier,
        "citation_id,depth,citing_opinion_id,cited_opinion_id\n\
         f1,2,1001,9999\n",
    )
    .unwrap();

    let report = materialize_with_home(
        &home,
        MaterializeCitationOverlayArgs {
            vault: name.to_string(),
            idmap,
            citations,
            frontier: Some(frontier),
            frontier_authorities: None,
            collection: None,
            skip_report: Some(home.join("skips.json")),
            report: Some(home.join("report.json")),
            home: None,
        },
    )
    .expect("materialize overlay");

    // 5 in-slice rows: 3 in-slice edges + 1 frontier edge; 1 invalid-depth skip;
    // 1 duplicate pair coalesced (never skipped).
    assert_eq!(report.skip_report.total_rows, 5);
    assert_eq!(report.skip_report.edges_built, 4); // 3 in-slice + 1 frontier
    assert_eq!(report.skip_report.skipped_total, 1);
    assert_eq!(report.skip_report.skipped_unresolved_cited, 0);
    assert_eq!(report.skip_report.skipped_duplicate_pair, 0);
    assert_eq!(report.skip_report.coalesced_duplicate_pair, 1);
    assert_eq!(report.skip_report.skipped_invalid_depth, 1);
    // Frontier accounting is tracked separately from the in-slice counters.
    assert_eq!(report.skip_report.frontier_rows, 1);
    assert_eq!(report.skip_report.frontier_edges_built, 1);
    assert_eq!(report.skip_report.frontier_nodes_built, 1);
    // Every idmap opinion was proven against the physical alias relation first.
    assert_eq!(report.skip_report.physical_alias_rows_verified, 4);
    // In-slice acceptance identity: in-slice edges == rows - skips - coalesced.
    let in_slice_edges = report.skip_report.edges_built - report.skip_report.frontier_edges_built;
    assert_eq!(
        in_slice_edges,
        report.skip_report.total_rows
            - report.skip_report.skipped_total
            - report.skip_report.coalesced_duplicate_pair
    );

    // Readback counts from the physical Graph CF + CSR: 4 in-corpus opinions plus
    // the single frontier node; 3 in-slice edges plus the frontier edge.
    assert_eq!(report.readback.node_rows_written, 5);
    assert_eq!(report.readback.edge_rows_written, 4);
    assert_eq!(report.readback.physical_edge_out_keys, 4);
    assert_eq!(report.readback.csr_edges, 4);
    assert_eq!(report.readback.csr_nodes, 5);
    assert!(report.readback.all_edge_values_read_back);

    // Spot-verify the coalesced A->B pair in the accepted generation's CF bytes:
    // r5 (depth 4) coalesced into r1 (depth 3), so the edge carries the max depth
    // and weight while preserving the first row's source id.
    let vault_dir = home.join("vaults").join(&report.vault_id);
    let physical = PhysicalPlainGraph::open_latest_accepted(&vault_dir, DEFAULT_COLLECTION)
        .expect("open accepted");
    let edge_bytes = physical
        .get_edge(a, EDGE_TYPE, b)
        .expect("read edge")
        .expect("edge A->B present");
    let edge: Value = serde_json::from_slice(&edge_bytes).unwrap();
    assert_eq!(edge["edge_type"], "cites");
    assert_eq!(edge["depth"], 4);
    assert_eq!(edge["weight"], json!(0.4));
    assert_eq!(
        edge["provenance_dataset"],
        "courtlistener-citation-map-2026-06-30"
    );
    assert_eq!(edge["source_row_id"], "r1");

    // depth>10 capped to weight 1.0 in the CF bytes.
    let capped = physical.get_edge(b, EDGE_TYPE, d).unwrap().unwrap();
    let capped: Value = serde_json::from_slice(&capped).unwrap();
    assert_eq!(capped["depth"], 15);
    assert_eq!(capped["weight"], json!(1.0));

    // The declared out-of-corpus citation is written as a frontier edge.
    let frontier_dst = super::frontier_node_id("9999");
    let frontier_edge = physical
        .get_edge(a, super::FRONTIER_EDGE_TYPE, frontier_dst)
        .expect("read frontier edge")
        .expect("frontier edge A->9999 present");
    let frontier_edge: Value = serde_json::from_slice(&frontier_edge).unwrap();
    assert_eq!(frontier_edge["edge_type"], "cites_outside_corpus");
    assert_eq!(frontier_edge["source_row_id"], "f1");

    // Skip report persisted to disk with matching counts.
    let skip_disk: Value =
        serde_json::from_slice(&fs::read(home.join("skips.json")).unwrap()).unwrap();
    assert_eq!(skip_disk["edges_built"], 4);
    assert_eq!(skip_disk["skipped_total"], 1);
    assert_eq!(skip_disk["frontier_edges_built"], 1);

    fs::remove_dir_all(&home).ok();
}

#[test]
fn refuses_when_no_edges_resolve() {
    let home = temp_root("empty");
    fs::create_dir_all(home.join("vaults")).unwrap();
    let name = "legal-cuyahoga";
    make_vault(&home, name);
    let aliases = seed_alias_relation(&home, name, &["1001", "1002"]);
    let a = aliases["1001"];
    let b = aliases["1002"];
    let idmap = home.join("idmap.csv");
    fs::write(&idmap, format!("opinion_id,cx_id\n1001,{a}\n1002,{b}\n")).unwrap();
    let citations = home.join("citations.csv");
    // Both endpoints resolve, but the only row is a zero-depth invalid skip, so no
    // edge survives. An unresolved in-slice endpoint is a hard failure now, not a
    // skip, so the empty-overlay guard must be reached through a genuine skip.
    fs::write(
        &citations,
        "id,depth,citing_opinion_id,cited_opinion_id\nr1,0,1001,1002\n",
    )
    .unwrap();
    let error = materialize_with_home(
        &home,
        MaterializeCitationOverlayArgs {
            vault: name.to_string(),
            idmap,
            citations,
            frontier: None,
            frontier_authorities: None,
            collection: None,
            skip_report: None,
            report: None,
            home: None,
        },
    )
    .expect_err("no edges resolved");
    assert!(error.message().contains("no citation edges resolved"));
    fs::remove_dir_all(&home).ok();
}
