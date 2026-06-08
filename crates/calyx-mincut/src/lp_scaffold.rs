use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{MincutError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintSense {
    Leq,
    Geq,
    Eq,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptSense {
    Minimize,
    Maximize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    Optimal,
    Infeasible,
    Unbounded,
    NotSolved,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LpVariable {
    pub id: usize,
    pub name: String,
    pub lb: f64,
    pub ub: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LpConstraint {
    pub coeffs: Vec<(usize, f64)>,
    pub sense: ConstraintSense,
    pub rhs: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LpProblem {
    pub vars: Vec<LpVariable>,
    pub constraints: Vec<LpConstraint>,
    pub objective: Vec<(usize, f64)>,
    pub sense: OptSense,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LpSolution {
    pub values: Vec<f64>,
    pub objective_value: f64,
    pub status: SolveStatus,
}

impl LpVariable {
    pub fn new(id: usize, name: impl Into<String>, lb: f64, ub: f64) -> Result<Self> {
        if !lb.is_finite() || !ub.is_finite() || lb > ub {
            return Err(MincutError::lp_invalid(format!(
                "invalid bounds for variable {id}: [{lb}, {ub}]"
            )));
        }
        Ok(Self {
            id,
            name: name.into(),
            lb,
            ub,
        })
    }
}

impl LpProblem {
    pub fn validate(&self) -> Result<()> {
        for (index, var) in self.vars.iter().enumerate() {
            if var.id != index {
                return Err(MincutError::lp_invalid(format!(
                    "variable id {} is not dense index {index}",
                    var.id
                )));
            }
            if !var.lb.is_finite() || !var.ub.is_finite() || var.lb > var.ub {
                return Err(MincutError::lp_invalid(format!(
                    "invalid bounds for variable {}",
                    var.id
                )));
            }
        }
        for (var, coeff) in &self.objective {
            validate_var_ref(*var, self.vars.len())?;
            validate_finite(*coeff, "objective coefficient")?;
        }
        for constraint in &self.constraints {
            validate_finite(constraint.rhs, "constraint rhs")?;
            for (var, coeff) in &constraint.coeffs {
                validate_var_ref(*var, self.vars.len())?;
                validate_finite(*coeff, "constraint coefficient")?;
            }
        }
        Ok(())
    }
}

pub fn mfvs_lp_problem(graph: &AssocGraph) -> Result<LpProblem> {
    let vars: Vec<_> = graph
        .node_ids()
        .enumerate()
        .map(|(index, id)| LpVariable::new(index, format!("x_{id}"), 0.0, 1.0))
        .collect::<Result<_>>()?;
    let objective = vars.iter().map(|var| (var.id, 1.0)).collect();
    let problem = LpProblem {
        vars,
        constraints: Vec::new(),
        objective,
        sense: OptSense::Minimize,
    };
    problem.validate()?;
    Ok(problem)
}

fn validate_var_ref(var: usize, len: usize) -> Result<()> {
    if var < len {
        Ok(())
    } else {
        Err(MincutError::lp_invalid(format!(
            "variable reference {var} out of range for {len} vars"
        )))
    }
}

fn validate_finite(value: f64, field: &'static str) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(MincutError::lp_invalid(format!("{field} is not finite")))
    }
}
