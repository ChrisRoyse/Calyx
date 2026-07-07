use calyx_core::{AnchorKind, AnchorValue};
use serde_json::{Value, json};

pub struct Prediction {
    pub associated: u32,
    pub positive: u32,
    pub p_model: f64,
    pub neighbors: Vec<Value>,
}

impl Prediction {
    pub fn to_json(&self) -> Value {
        json!({"associated": self.associated, "positive": self.positive, "p_model": self.p_model, "neighbors": self.neighbors})
    }
}

#[derive(Debug)]
pub struct ScenarioError {
    pub code: &'static str,
    pub message: String,
}

impl ScenarioError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({"code": self.code, "message": self.message})
    }
}

pub fn scalar(cx: &calyx_core::Constellation, key: &str) -> Result<f64, ScenarioError> {
    cx.scalars.get(key).copied().ok_or_else(|| {
        ScenarioError::new(
            "CALYX_POLY_SCENARIO_MISSING_SCALAR",
            format!("missing scalar {key}"),
        )
    })
}

pub fn outcome_anchor(cx: &calyx_core::Constellation) -> Option<bool> {
    cx.anchors.iter().find_map(|anchor| {
        if anchor.kind == AnchorKind::TestPass
            && let AnchorValue::Bool(value) = &anchor.value
        {
            return Some(*value);
        }
        None
    })
}
