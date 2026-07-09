pub struct LeapableMethod {
    pub name: &'static str,
    pub mutating: bool,
    pub tags: &'static [&'static str],
}

const fn method(
    name: &'static str,
    mutating: bool,
    tags: &'static [&'static str],
) -> LeapableMethod {
    LeapableMethod {
        name,
        mutating,
        tags,
    }
}

pub const LEAPABLE_METHODS: &[LeapableMethod] = &[
    method("engine.info", false, &["engine"]),
    method("vault.create", true, &["vault"]),
    method("vault.open", false, &["vault"]),
    method("vault.close", true, &["vault"]),
    method("vault.list", false, &["vault"]),
    method("vault.delete", true, &["vault"]),
    method("vault.snapshot", true, &["vault"]),
    method("vault.restore", true, &["vault"]),
    method("vault.clone", true, &["vault"]),
    method("vault.verify", false, &["vault"]),
    method("vault.stat", false, &["vault"]),
    method("cx.put", true, &["cx"]),
    method("cx.put_batch", true, &["cx"]),
    method("cx.get", false, &["cx"]),
    method("cx.scan", false, &["cx"]),
    method("cx.anchor", true, &["cx"]),
    method("cx.delete", true, &["cx"]),
    method("rel.insert", true, &["storage", "relational"]),
    method("rel.get", false, &["storage", "relational"]),
    method("rel.update_row", true, &["storage", "relational"]),
    method("rel.delete", true, &["storage", "relational"]),
    method("rel.scan", false, &["storage", "relational"]),
    method("rel.query", false, &["storage", "btree"]),
    method("kv.set", true, &["storage", "kv"]),
    method("kv.get", false, &["storage", "kv"]),
    method("kv.delete", true, &["storage", "kv"]),
    method("ts.write", true, &["storage", "timeseries"]),
    method("ts.range", false, &["storage", "timeseries"]),
    method("blob.put", true, &["storage", "blob"]),
    method("blob.get", false, &["storage", "blob"]),
    method("txn.commit", true, &["storage", "txn"]),
    method("engine.panic_probe", false, &["engine", "test"]),
];

/// Compile-time capability map for build-info. Runtime `engine.info` is derived
/// from `LEAPABLE_METHODS` plus these explicit non-served false entries.
pub const LEAPABLE_CAPABILITIES: &[(&str, bool)] = &[
    ("cpu-only", true),
    ("stdio-jsonrpc-ndjson", true),
    ("cx-crud", true),
    ("relational-btree-query", true),
    ("kv", true),
    ("blob", true),
    ("timeseries", true),
    ("hnsw-ram", false),
    ("vector-query", false),
    ("ann-query", false),
    ("inverted-query", false),
    ("kernel-query", false),
    ("cuda", false),
    ("diskann", false),
    ("spann", false),
];

pub fn served_method_names() -> impl Iterator<Item = &'static str> {
    LEAPABLE_METHODS.iter().map(|method| method.name)
}

pub fn mutating_method_requires_id(method: &str) -> bool {
    LEAPABLE_METHODS
        .iter()
        .any(|registered| registered.name == method && registered.mutating)
}
