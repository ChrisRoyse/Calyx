use crate::raw_large_corpus_onchain_chunks::ChunkSourceSpec;

pub(crate) const POLYGON_DRPC_URL: &str = "https://polygon.drpc.org";
pub(crate) const GOLDSKY_ORDERBOOK_URL: &str = "https://api.goldsky.com/api/public/project_cl6mb8i9h0003e201j6li0diw/subgraphs/orderbook-subgraph/prod/gn";
pub(crate) const GOLDSKY_ACTIVITY_URL: &str = "https://api.goldsky.com/api/public/project_cl6mb8i9h0003e201j6li0diw/subgraphs/activity-subgraph/0.0.4/gn";
pub(crate) const CONTRACTS_DOCS_URL: &str = "https://docs.polymarket.com/resources/contracts";
pub(crate) const GOLDSKY_DOCS_URL: &str = "https://docs.goldsky.com/chains/polymarket";
pub(crate) const V2_ORDER_FILLED_TOPIC: &str =
    "0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee";
pub(crate) const CTF_EXCHANGE_V2: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
pub(crate) const NEG_RISK_EXCHANGE_V2: &str = "0xe2222d279d744050d28e00520010520000310F59";

pub(crate) fn chunk_source_specs() -> [ChunkSourceSpec<'static>; 2] {
    [ctf_chunk_spec(), neg_risk_chunk_spec()]
}

pub(crate) fn ctf_chunk_spec() -> ChunkSourceSpec<'static> {
    ChunkSourceSpec {
        dataset: "polygon_rpc_ctf_exchange_v2_order_filled_chunked_large",
        endpoint: "ctf-exchange-v2-order-filled-logs-chunked",
        address: CTF_EXCHANGE_V2,
        topic: V2_ORDER_FILLED_TOPIC,
        rpc_url: POLYGON_DRPC_URL,
        docs_url: CONTRACTS_DOCS_URL,
    }
}

fn neg_risk_chunk_spec() -> ChunkSourceSpec<'static> {
    ChunkSourceSpec {
        dataset: "polygon_rpc_neg_risk_exchange_v2_order_filled_chunked_large",
        endpoint: "neg-risk-exchange-v2-order-filled-logs-chunked",
        address: NEG_RISK_EXCHANGE_V2,
        topic: V2_ORDER_FILLED_TOPIC,
        rpc_url: POLYGON_DRPC_URL,
        docs_url: CONTRACTS_DOCS_URL,
    }
}
