use std::collections::BTreeMap;

use calyx_core::Ts;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct CollectionSpecParam {
    #[serde(default)]
    pub(super) schema: Option<Vec<FieldParam>>,
    #[serde(default)]
    pub(super) indexes: Vec<IndexParam>,
    #[serde(default)]
    pub(super) retention: Option<RetentionParam>,
}

#[derive(Deserialize)]
pub(super) struct FieldParam {
    pub(super) name: String,
    pub(super) ty: FieldTypeParam,
    #[serde(default)]
    pub(super) nullable: bool,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FieldTypeParam {
    Bool,
    I64,
    U64,
    F64,
    Text,
    Bytes,
    Timestamp,
}

#[derive(Deserialize)]
pub(super) struct IndexParam {
    pub(super) name: String,
    #[serde(default = "default_btree_kind")]
    pub(super) kind: IndexKindParam,
    pub(super) fields: Vec<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum IndexKindParam {
    Btree,
    Inverted,
    Ann,
    Kernel,
}

fn default_btree_kind() -> IndexKindParam {
    IndexKindParam::Btree
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RetentionParam {
    Forever,
    RollupOnly,
    DropAfterMs(u64),
}

#[derive(Clone, Deserialize)]
pub(super) struct KeyParam {
    #[serde(default)]
    pub(super) u64: Option<u64>,
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) hex: Option<String>,
    #[serde(default)]
    pub(super) bytes: Option<Vec<u8>>,
}

#[derive(Clone, Deserialize)]
pub(super) struct BytesParam {
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) hex: Option<String>,
    #[serde(default)]
    pub(super) bytes: Option<Vec<u8>>,
}

#[derive(Clone, Deserialize)]
pub(super) struct RecordValueParam {
    #[serde(default)]
    pub(super) bool: Option<bool>,
    #[serde(default)]
    pub(super) i64: Option<i64>,
    #[serde(default)]
    pub(super) u64: Option<u64>,
    #[serde(default)]
    pub(super) f64: Option<f64>,
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) hex: Option<String>,
    #[serde(default)]
    pub(super) bytes: Option<Vec<u8>>,
    #[serde(default)]
    pub(super) timestamp: Option<i64>,
    #[serde(default)]
    pub(super) null: Option<bool>,
}

pub(super) type RowParam = BTreeMap<String, RecordValueParam>;

#[derive(Deserialize)]
pub(super) struct RelInsertParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    #[serde(default)]
    pub(super) collection: Option<CollectionSpecParam>,
    pub(super) pk: KeyParam,
    pub(super) row: RowParam,
}

#[derive(Deserialize)]
pub(super) struct RelGetParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    pub(super) pk: KeyParam,
    #[serde(default)]
    pub(super) snapshot: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct RelUpdateParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    pub(super) pk: KeyParam,
    #[serde(default)]
    pub(super) set: RowParam,
    #[serde(default)]
    pub(super) unset: Vec<String>,
}

#[derive(Deserialize)]
pub(super) struct RelDeleteParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    pub(super) pk: KeyParam,
}

#[derive(Deserialize)]
pub(super) struct RelScanParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    #[serde(default)]
    pub(super) cursor: Option<KeyParam>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) snapshot: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct RelQueryParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    pub(super) index_name: String,
    #[serde(default)]
    pub(super) gte: Option<RecordValueParam>,
    #[serde(default)]
    pub(super) lte: Option<RecordValueParam>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) snapshot: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct KvSetParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    #[serde(default)]
    pub(super) collection: Option<CollectionSpecParam>,
    #[serde(default)]
    pub(super) ns: u64,
    pub(super) key: BytesParam,
    pub(super) value: BytesParam,
    #[serde(default)]
    pub(super) ttl_ms: Option<u64>,
    #[serde(default)]
    pub(super) echo_value: bool,
}

#[derive(Deserialize)]
pub(super) struct KvGetParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    #[serde(default)]
    pub(super) ns: u64,
    pub(super) key: BytesParam,
    #[serde(default)]
    pub(super) include_text: bool,
}

#[derive(Deserialize)]
pub(super) struct KvDeleteParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    #[serde(default)]
    pub(super) ns: u64,
    pub(super) key: BytesParam,
}

#[derive(Deserialize)]
pub(super) struct TsWriteParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    #[serde(default)]
    pub(super) collection: Option<CollectionSpecParam>,
    pub(super) series: u64,
    pub(super) point_ts: u64,
    pub(super) value: f64,
}

#[derive(Deserialize)]
pub(super) struct TsRangeParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    pub(super) series: u64,
    pub(super) start_ts: u64,
    pub(super) end_ts: u64,
}

#[derive(Deserialize)]
pub(super) struct BlobPutParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    #[serde(default)]
    pub(super) collection: Option<CollectionSpecParam>,
    pub(super) input: BytesParam,
}

#[derive(Deserialize)]
pub(super) struct BlobGetParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) collection_name: String,
    pub(super) blob_id: String,
    #[serde(default)]
    pub(super) include_data: bool,
    #[serde(default)]
    pub(super) include_text: bool,
}

#[derive(Deserialize)]
pub(super) struct TxnCommitParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) cost_cap_ms: u32,
    #[serde(default = "default_txn_timeout_ms")]
    pub(super) timeout_ms: u64,
    #[serde(default = "default_isolation")]
    pub(super) isolation: IsolationParam,
    #[serde(default)]
    pub(super) inject_crash_after_stage: bool,
    pub(super) ops: Vec<TxnOpParam>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum IsolationParam {
    ReadCommitted,
    Serializable,
}

fn default_isolation() -> IsolationParam {
    IsolationParam::Serializable
}

fn default_txn_timeout_ms() -> u64 {
    50
}

#[derive(Deserialize)]
#[serde(tag = "op")]
pub(super) enum TxnOpParam {
    #[serde(rename = "rel.insert")]
    RelInsert {
        collection_name: String,
        #[serde(default)]
        collection: Option<CollectionSpecParam>,
        pk: KeyParam,
        row: RowParam,
    },
    #[serde(rename = "kv.set")]
    KvSet {
        collection_name: String,
        #[serde(default)]
        collection: Option<CollectionSpecParam>,
        #[serde(default)]
        ns: u64,
        key: BytesParam,
        value: BytesParam,
        #[serde(default)]
        ttl_ms: Option<u64>,
    },
    #[serde(rename = "ts.write")]
    TsWrite {
        collection_name: String,
        #[serde(default)]
        collection: Option<CollectionSpecParam>,
        series: u64,
        point_ts: u64,
        value: f64,
    },
}
