//! Navigation primitives over per-slot indexes.

use std::collections::BTreeMap;

use calyx_core::{Constellation, CxId, Result, SlotId, SlotVector};
use serde::{Deserialize, Serialize};

use crate::hit::Hit;
use crate::query::Query;
use crate::search::SearchEngine;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensComparison {
    pub slot: SlotId,
    pub hits: Vec<Hit>,
}

pub fn neighbors(engine: &SearchEngine, cx_id: CxId, slot: SlotId, k: usize) -> Result<Vec<Hit>> {
    let vector = engine
        .indexes
        .vector(slot, cx_id)?
        .ok_or_else(|| crate::slot_index_map::SlotIndexMap::missing_slot_error(slot))?;
    let query = Query::new("neighbors")
        .with_vector(vector)
        .with_slots(vec![slot])
        .explain(true);
    engine.search(&Query { k, ..query })
}

pub fn compare_lenses(
    engine: &SearchEngine,
    query: &Query,
    slots: &[SlotId],
) -> Result<Vec<LensComparison>> {
    let mut out = Vec::new();
    for slot in slots {
        let mut per_slot = query.clone();
        per_slot.slots = vec![*slot];
        per_slot.fusion = Some(crate::fusion::FusionStrategy::SingleLens { slot: *slot });
        out.push(LensComparison {
            slot: *slot,
            hits: engine.search(&per_slot)?,
        });
    }
    Ok(out)
}

pub fn define(engine: &SearchEngine, cx_id: CxId, slot: SlotId, k: usize) -> Result<Constellation> {
    let neighborhood = neighbors(engine, cx_id, slot, k)?;
    let seed = engine
        .constellation(cx_id)
        .cloned()
        .ok_or_else(|| crate::slot_index_map::SlotIndexMap::missing_slot_error(slot))?;
    let mut gathered = BTreeMap::<SlotId, SlotVector>::new();
    for index_slot in engine.indexes.slots() {
        let mut dense_sum: Option<Vec<f32>> = None;
        let mut count = 0.0;
        for hit in &neighborhood {
            if let Some(SlotVector::Dense { data, .. }) =
                engine.indexes.vector(index_slot, hit.cx_id)?
            {
                if dense_sum.is_none() {
                    dense_sum = Some(vec![0.0; data.len()]);
                }
                if let Some(sum) = &mut dense_sum {
                    for (acc, value) in sum.iter_mut().zip(data) {
                        *acc += value;
                    }
                }
                count += 1.0;
            }
        }
        if let Some(mut sum) = dense_sum {
            for value in &mut sum {
                *value /= count;
            }
            gathered.insert(
                index_slot,
                SlotVector::Dense {
                    dim: sum.len() as u32,
                    data: sum,
                },
            );
        }
    }
    Ok(Constellation {
        slots: gathered,
        ..seed
    })
}
