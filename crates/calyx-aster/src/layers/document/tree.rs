use std::collections::BTreeMap;

use calyx_core::Result;
use serde_json::{Map, Value};

use super::codec::{DOC_ID_BYTES, decode_cell, parse_document_key, validate_segment};
use super::errors::corrupt_doc;

pub(super) fn flatten_document(
    path: &mut Vec<String>,
    value: &Value,
    out: &mut Vec<(Vec<String>, Value)>,
) -> Result<()> {
    if let Value::Object(map) = value
        && !map.is_empty()
    {
        let mut entries = map.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(right.0));
        for (name, child) in entries {
            validate_segment(name)?;
            path.push(name.clone());
            flatten_document(path, child, out)?;
            path.pop();
        }
        return Ok(());
    }
    out.push((path.clone(), value.clone()));
    Ok(())
}

pub(super) fn build_tree(cells: Vec<(Vec<String>, Value)>) -> Result<Value> {
    if cells.len() == 1 && cells[0].0.is_empty() {
        return Ok(cells.into_iter().next().unwrap().1);
    }
    let mut root = Value::Object(Map::new());
    for (path, value) in cells {
        if path.is_empty() {
            return Err(corrupt_doc("document root leaf collides with child leaves"));
        }
        insert_path(&mut root, &path, value)?;
    }
    Ok(root)
}

pub(super) fn docs_from_rows(rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Vec<Value>> {
    let mut grouped: BTreeMap<[u8; DOC_ID_BYTES], Vec<(Vec<String>, Value)>> = BTreeMap::new();
    for (key, value) in rows {
        let (doc_id, path) = parse_document_key(&key)?;
        if let Some(value) = decode_cell(&value)?.into_leaf_value()? {
            grouped.entry(doc_id).or_default().push((path, value));
        }
    }
    grouped
        .into_values()
        .filter(|cells| !cells.is_empty())
        .map(build_tree)
        .collect()
}

fn insert_path(node: &mut Value, path: &[String], value: Value) -> Result<()> {
    let Value::Object(map) = node else {
        return Err(corrupt_doc("document leaf collides with an object path"));
    };
    if path.len() == 1 {
        if map.insert(path[0].clone(), value).is_some() {
            return Err(corrupt_doc("duplicate document path leaf"));
        }
        return Ok(());
    }
    let child = map
        .entry(path[0].clone())
        .or_insert_with(|| Value::Object(Map::new()));
    insert_path(child, &path[1..], value)
}
