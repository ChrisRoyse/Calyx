use calyx_lodestar::RecallQuery;
use calyx_paths::AssocGraph;

const DIM: usize = 64;

pub(crate) fn similarity_graph(rows: &[RecallQuery], fanout: usize) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for row in rows {
        builder.add_node(row.cx_id, 1.0).expect("node");
    }
    for row in rows {
        let mut scored: Vec<_> = rows
            .iter()
            .filter(|other| other.cx_id != row.cx_id)
            .map(|other| (other.cx_id, cosine(&row.vector, &other.vector).max(0.0)))
            .collect();
        scored.sort_by(|left, right| right.1.total_cmp(&left.1));
        for (dst, score) in scored.into_iter().take(fanout) {
            if score > 0.0 {
                builder.add_edge(row.cx_id, dst, score).expect("edge");
            }
        }
    }
    builder.build()
}

pub(crate) fn token_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; DIM];
    for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.len() < 3 {
            continue;
        }
        let digest = blake3::hash(token.to_ascii_lowercase().as_bytes());
        let idx = u16::from_be_bytes([digest.as_bytes()[0], digest.as_bytes()[1]]) as usize % DIM;
        vector[idx] += 1.0;
    }
    normalize(&mut vector);
    vector
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let an = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let bn = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if an == 0.0 || bn == 0.0 {
        0.0
    } else {
        dot / (an * bn)
    }
}
