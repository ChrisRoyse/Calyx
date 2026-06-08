//! BM25 scorer using Lucene-like defaults.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bm25 {
    pub k1: f32,
    pub b: f32,
}

impl Default for Bm25 {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

impl Bm25 {
    pub fn idf(self, total_docs: usize, doc_freq: usize) -> f32 {
        (((total_docs as f32 - doc_freq as f32 + 0.5) / (doc_freq as f32 + 0.5)) + 1.0).ln()
    }

    pub fn score_term(
        self,
        tf: usize,
        doc_len: usize,
        avg_doc_len: f32,
        total_docs: usize,
        doc_freq: usize,
    ) -> f32 {
        if tf == 0 || total_docs == 0 || doc_freq == 0 {
            return 0.0;
        }
        let tf = tf as f32;
        let len_norm = if avg_doc_len <= 0.0 {
            1.0
        } else {
            doc_len as f32 / avg_doc_len
        };
        let denom = tf + self.k1 * (1.0 - self.b + self.b * len_norm);
        self.idf(total_docs, doc_freq) * (tf * (self.k1 + 1.0)) / denom
    }
}
