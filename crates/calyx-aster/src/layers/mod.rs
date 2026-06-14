//! Key-encoding layers over Aster's ordered transactional core.

use calyx_core::Result;

use crate::collection::{Collection, CollectionMode};

pub mod blob;
pub mod document;
pub mod kv;
pub mod relational;
pub mod timeseries;

pub use blob::{BlobId, BlobLayer, BlobManifest};
pub use document::{DocId, DocumentLayer};
pub use kv::KvLayer;
pub use relational::{RecordKey, RecordValue, RelationalLayer, Row};
pub use timeseries::{RollupValue, RollupWindow, TimeSeriesLayer};

pub trait Layer: Send + Sync {
    fn collection_mode() -> CollectionMode
    where
        Self: Sized;

    fn put(&self, col: &Collection, key: &[u8], value: &[u8]) -> Result<()>;
    fn get(&self, col: &Collection, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn range(
        &self,
        col: &Collection,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<Vec<u8>>>;
}
