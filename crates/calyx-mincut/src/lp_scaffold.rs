use std::collections::{BTreeSet, VecDeque};

use calyx_core::CxId;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{MincutError, Result};

pub const MFVS_LP_MAX_NODES: usize = 24;
pub const MFVS_LP_MAX_CYCLE_CONSTRAINTS: usize = 4096;
pub const MFVS_LP_MAX_SEARCH_STATES: usize = 1_000_000;

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
    let constraints = cycle_constraints(graph)?;
    let problem = LpProblem {
        vars,
        constraints,
        objective,
        sense: OptSense::Minimize,
    };
    problem.validate()?;
    Ok(problem)
}

pub fn solve_mfvs_lp(graph: &AssocGraph) -> Result<LpSolution> {
    if graph.is_empty() {
        return Ok(LpSolution {
            values: Vec::new(),
            objective_value: 0.0,
            status: SolveStatus::Optimal,
        });
    }
    if shortest_directed_cycle(graph, &BTreeSet::new()).is_none() {
        return Ok(LpSolution {
            values: vec![0.0; graph.node_count()],
            objective_value: 0.0,
            status: SolveStatus::Optimal,
        });
    }
    if graph.node_count() > MFVS_LP_MAX_NODES {
        return Err(MincutError::lp_solver_limit(format!(
            "bounded exact MFVS solver supports at most {MFVS_LP_MAX_NODES} cyclic nodes, got {}",
            graph.node_count()
        )));
    }

    let mut solver = ExactMfvsSolver::new(graph)?;
    let mut removed = BTreeSet::new();
    solver.search(&mut removed)?;
    let best = solver
        .best
        .ok_or_else(|| MincutError::lp_solve_failed("search ended without a feasible FVS"))?;
    if !is_feedback_vertex_set_indices(graph, &best) {
        return Err(MincutError::lp_solve_failed(
            "internal solver result failed residual acyclicity verification",
        ));
    }

    let mut values = vec![0.0; graph.node_count()];
    for index in &best {
        values[*index] = 1.0;
    }
    Ok(LpSolution {
        values,
        objective_value: best.len() as f64,
        status: SolveStatus::Optimal,
    })
}

pub fn verify_feedback_vertex_set(graph: &AssocGraph, members: &[CxId]) -> Result<bool> {
    let removed = members
        .iter()
        .map(|id| {
            graph
                .node_index(*id)
                .ok_or(MincutError::NodeNotFound { id: *id })
        })
        .collect::<Result<BTreeSet<_>>>()?;
    Ok(is_feedback_vertex_set_indices(graph, &removed))
}

fn cycle_constraints(graph: &AssocGraph) -> Result<Vec<LpConstraint>> {
    if graph.node_count() > MFVS_LP_MAX_NODES {
        if shortest_directed_cycle(graph, &BTreeSet::new()).is_some() {
            return Err(MincutError::lp_solver_limit(format!(
                "complete MFVS cycle-constraint model supports at most {MFVS_LP_MAX_NODES} cyclic nodes, got {}",
                graph.node_count()
            )));
        }
        return Ok(Vec::new());
    }

    enumerate_simple_cycles(graph)?
        .into_iter()
        .map(|cycle| {
            Ok(LpConstraint {
                coeffs: cycle.into_iter().map(|index| (index, 1.0)).collect(),
                sense: ConstraintSense::Geq,
                rhs: 1.0,
            })
        })
        .collect()
}

fn enumerate_simple_cycles(graph: &AssocGraph) -> Result<Vec<Vec<usize>>> {
    let mut cycles = Vec::new();
    let mut stack = Vec::new();
    let mut on_stack = vec![false; graph.node_count()];
    for start in 0..graph.node_count() {
        stack.push(start);
        on_stack[start] = true;
        enumerate_from(graph, start, start, &mut stack, &mut on_stack, &mut cycles)?;
        on_stack[start] = false;
        stack.pop();
    }
    Ok(cycles)
}

fn enumerate_from(
    graph: &AssocGraph,
    start: usize,
    current: usize,
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    cycles: &mut Vec<Vec<usize>>,
) -> Result<()> {
    for edge in graph.out_edges_by_index(current) {
        let next = edge.dst;
        if next < start {
            continue;
        }
        if next == start {
            push_cycle(cycles, stack.clone())?;
        } else if !on_stack[next] {
            on_stack[next] = true;
            stack.push(next);
            enumerate_from(graph, start, next, stack, on_stack, cycles)?;
            stack.pop();
            on_stack[next] = false;
        }
    }
    Ok(())
}

fn push_cycle(cycles: &mut Vec<Vec<usize>>, cycle: Vec<usize>) -> Result<()> {
    cycles.push(cycle);
    if cycles.len() > MFVS_LP_MAX_CYCLE_CONSTRAINTS {
        return Err(MincutError::lp_solver_limit(format!(
            "MFVS cycle model exceeded {MFVS_LP_MAX_CYCLE_CONSTRAINTS} directed-cycle constraints"
        )));
    }
    Ok(())
}

struct ExactMfvsSolver<'a> {
    graph: &'a AssocGraph,
    degrees: Vec<usize>,
    best: Option<BTreeSet<usize>>,
    states: usize,
}

impl<'a> ExactMfvsSolver<'a> {
    fn new(graph: &'a AssocGraph) -> Result<Self> {
        let degrees = total_degrees(graph);
        let greedy = greedy_upper_bound(graph, &degrees);
        if !is_feedback_vertex_set_indices(graph, &greedy) {
            return Err(MincutError::lp_solve_failed(
                "greedy warm start failed residual acyclicity verification",
            ));
        }
        Ok(Self {
            graph,
            degrees,
            best: Some(greedy),
            states: 0,
        })
    }

    fn search(&mut self, removed: &mut BTreeSet<usize>) -> Result<()> {
        self.states += 1;
        if self.states > MFVS_LP_MAX_SEARCH_STATES {
            return Err(MincutError::lp_solver_limit(format!(
                "MFVS exact search exceeded {MFVS_LP_MAX_SEARCH_STATES} branch states before proving optimality"
            )));
        }
        if self
            .best
            .as_ref()
            .is_some_and(|best| removed.len() >= best.len())
        {
            return Ok(());
        }
        let lower_bound = vertex_disjoint_cycle_lower_bound(self.graph, removed);
        if self
            .best
            .as_ref()
            .is_some_and(|best| removed.len() + lower_bound >= best.len())
        {
            return Ok(());
        }

        let Some(cycle) = shortest_directed_cycle(self.graph, removed) else {
            self.consider_solution(removed);
            return Ok(());
        };
        let mut candidates = cycle;
        candidates.sort_by(|left, right| {
            self.degrees[*right]
                .cmp(&self.degrees[*left])
                .then_with(|| left.cmp(right))
        });
        for candidate in candidates {
            if removed.insert(candidate) {
                self.search(removed)?;
                removed.remove(&candidate);
            }
        }
        Ok(())
    }

    fn consider_solution(&mut self, removed: &BTreeSet<usize>) {
        let replace = self.best.as_ref().is_none_or(|best| {
            removed.len() < best.len()
                || (removed.len() == best.len() && removed.iter().cmp(best.iter()).is_lt())
        });
        if replace {
            self.best = Some(removed.clone());
        }
    }
}

fn greedy_upper_bound(graph: &AssocGraph, degrees: &[usize]) -> BTreeSet<usize> {
    let mut removed = BTreeSet::new();
    while let Some(cycle) = shortest_directed_cycle(graph, &removed) {
        let candidate = cycle
            .into_iter()
            .max_by(|left, right| {
                degrees[*left]
                    .cmp(&degrees[*right])
                    .then_with(|| right.cmp(left))
            })
            .expect("cycle contains at least one vertex");
        removed.insert(candidate);
    }
    shrink_solution(graph, &mut removed);
    removed
}

fn shrink_solution(graph: &AssocGraph, removed: &mut BTreeSet<usize>) {
    let candidates: Vec<_> = removed.iter().copied().collect();
    for candidate in candidates {
        removed.remove(&candidate);
        if !is_feedback_vertex_set_indices(graph, removed) {
            removed.insert(candidate);
        }
    }
}

fn vertex_disjoint_cycle_lower_bound(graph: &AssocGraph, removed: &BTreeSet<usize>) -> usize {
    let mut blocked = removed.clone();
    let mut count = 0;
    while let Some(cycle) = shortest_directed_cycle(graph, &blocked) {
        count += 1;
        blocked.extend(cycle);
    }
    count
}

fn is_feedback_vertex_set_indices(graph: &AssocGraph, removed: &BTreeSet<usize>) -> bool {
    shortest_directed_cycle(graph, removed).is_none()
}

fn shortest_directed_cycle(graph: &AssocGraph, removed: &BTreeSet<usize>) -> Option<Vec<usize>> {
    let mut best: Option<Vec<usize>> = None;
    for edge in graph.edges() {
        if removed.contains(&edge.src) || removed.contains(&edge.dst) {
            continue;
        }
        if edge.src == edge.dst {
            return Some(vec![edge.src]);
        }
        let Some(path) = shortest_path(graph, edge.dst, edge.src, removed) else {
            continue;
        };
        let mut cycle = Vec::with_capacity(path.len());
        cycle.push(edge.src);
        cycle.extend(path.into_iter().take_while(|node| *node != edge.src));
        if best
            .as_ref()
            .is_none_or(|current| cycle_is_better(&cycle, current))
        {
            best = Some(cycle);
        }
    }
    best
}

fn cycle_is_better(candidate: &[usize], current: &[usize]) -> bool {
    candidate.len() < current.len()
        || (candidate.len() == current.len() && candidate.iter().cmp(current.iter()).is_lt())
}

fn shortest_path(
    graph: &AssocGraph,
    start: usize,
    target: usize,
    removed: &BTreeSet<usize>,
) -> Option<Vec<usize>> {
    let mut seen = vec![false; graph.node_count()];
    let mut prev = vec![None; graph.node_count()];
    let mut queue = VecDeque::from([start]);
    seen[start] = true;
    while let Some(current) = queue.pop_front() {
        for edge in graph.out_edges_by_index(current) {
            let next = edge.dst;
            if removed.contains(&next) || seen[next] {
                continue;
            }
            seen[next] = true;
            prev[next] = Some(current);
            if next == target {
                return Some(reconstruct_path(start, target, &prev));
            }
            queue.push_back(next);
        }
    }
    None
}

fn reconstruct_path(start: usize, target: usize, prev: &[Option<usize>]) -> Vec<usize> {
    let mut path = vec![target];
    let mut current = target;
    while current != start {
        current = prev[current].expect("target reached by BFS");
        path.push(current);
    }
    path.reverse();
    path
}

fn total_degrees(graph: &AssocGraph) -> Vec<usize> {
    let mut degrees = vec![0_usize; graph.node_count()];
    for edge in graph.edges() {
        degrees[edge.src] += 1;
        degrees[edge.dst] += 1;
    }
    degrees
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
