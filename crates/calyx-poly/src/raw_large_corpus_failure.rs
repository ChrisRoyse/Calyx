use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusFailure {
    pub code: String,
    pub message: String,
}
