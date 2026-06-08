//! Pipeline strategy helpers.

use calyx_core::CxId;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq)]
pub struct PipelineOutput {
    pub stage1_candidates: usize,
    pub final_hits: usize,
    pub subset_ok: bool,
    pub zeroizing_ok: bool,
    pub candidate_ids: Vec<CxId>,
}

pub fn candidate_texts(texts: &[String]) -> Vec<Zeroizing<String>> {
    texts.iter().cloned().map(Zeroizing::new).collect()
}

pub fn summarize_pipeline(stage1: &[CxId], final_ids: &[CxId], texts: &[String]) -> PipelineOutput {
    let request_scoped = candidate_texts(texts);
    PipelineOutput {
        stage1_candidates: stage1.len(),
        final_hits: final_ids.len(),
        subset_ok: final_ids.iter().all(|cx| stage1.contains(cx)),
        zeroizing_ok: request_scoped.len() == texts.len(),
        candidate_ids: final_ids.to_vec(),
    }
}
