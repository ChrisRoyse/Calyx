const HOP_DECAY: f32 = 0.9;

pub fn attenuate(base_score: f32, hops: u32) -> f32 {
    base_score * HOP_DECAY.powi(hops as i32)
}

pub fn deattenuate(attenuated: f32, hops: u32) -> f32 {
    attenuated / HOP_DECAY.powi(hops as i32)
}
