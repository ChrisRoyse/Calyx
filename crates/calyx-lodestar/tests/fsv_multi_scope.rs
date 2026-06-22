#![cfg(feature = "fsv")]

#[path = "support/multi_scope_fsv.rs"]
mod multi_scope_fsv;
#[allow(dead_code, unused_imports)]
#[path = "support/real_corpora.rs"]
mod real_corpora;

use real_corpora::{calyx_home, scifact_text};

#[test]
#[ignore = "manual FSV: reads real SciFact corpus and writes $CALYX_HOME/fsv reports"]
fn fsv_multi_scope_real_corpus_manual() {
    let home = calyx_home();
    let corpus = scifact_text(&home);
    let summary = multi_scope_fsv::run(&home, &corpus);

    assert!(summary.scope_count >= 4);
    assert!(summary.bridge_count > 0);
    assert!(summary.union_mfvs_not_naive);
}
