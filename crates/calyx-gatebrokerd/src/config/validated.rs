//! Post-validation configuration wrappers. Construction happens only inside
//! [`super::rules`], so holding one of these types is proof that every
//! authority invariant in the schema has already been enforced.

use std::collections::BTreeMap;

use crate::protocol::{ExecutionRootAlias, RootAlias};

use super::schema::{BrokerConfig, ExecutionRootConfig, RootConfig, StateConfig};

#[derive(Debug, Clone)]
pub struct ValidatedConfig {
    pub(super) raw: BrokerConfig,
    pub(super) state: ValidatedStateConfig,
    pub(super) roots: BTreeMap<RootAlias, ValidatedRootConfig>,
    pub(super) execution_roots: BTreeMap<ExecutionRootAlias, ValidatedExecutionRootConfig>,
}

impl ValidatedConfig {
    pub fn raw(&self) -> &BrokerConfig {
        &self.raw
    }

    pub fn roots(&self) -> &BTreeMap<RootAlias, ValidatedRootConfig> {
        &self.roots
    }

    pub fn state(&self) -> &ValidatedStateConfig {
        &self.state
    }

    pub fn root(&self, alias: &RootAlias) -> Option<&ValidatedRootConfig> {
        self.roots.get(alias)
    }

    pub fn execution_roots(&self) -> &BTreeMap<ExecutionRootAlias, ValidatedExecutionRootConfig> {
        &self.execution_roots
    }

    pub fn execution_root(
        &self,
        alias: &ExecutionRootAlias,
    ) -> Option<&ValidatedExecutionRootConfig> {
        self.execution_roots.get(alias)
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedStateConfig {
    pub(super) raw: StateConfig,
    pub(super) anchor_mode: u32,
    pub(super) private_mode: u32,
    pub(super) journal_directory_mode: u32,
}

impl ValidatedStateConfig {
    pub fn raw(&self) -> &StateConfig {
        &self.raw
    }

    pub fn anchor_mode(&self) -> u32 {
        self.anchor_mode
    }

    pub fn private_mode(&self) -> u32 {
        self.private_mode
    }

    pub fn journal_directory_mode(&self) -> u32 {
        self.journal_directory_mode
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedRootConfig {
    pub(super) alias: RootAlias,
    pub(super) raw: RootConfig,
    pub(super) shared_mode: u32,
    pub(super) private_mode: u32,
    pub(super) published_mode: u32,
}

impl ValidatedRootConfig {
    pub fn alias(&self) -> &RootAlias {
        &self.alias
    }

    pub fn raw(&self) -> &RootConfig {
        &self.raw
    }

    pub fn shared_mode(&self) -> u32 {
        self.shared_mode
    }

    pub fn private_mode(&self) -> u32 {
        self.private_mode
    }

    pub fn published_mode(&self) -> u32 {
        self.published_mode
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedExecutionRootConfig {
    pub(super) alias: ExecutionRootAlias,
    pub(super) raw: ExecutionRootConfig,
    pub(super) expected_mode: u32,
}

impl ValidatedExecutionRootConfig {
    pub fn alias(&self) -> &ExecutionRootAlias {
        &self.alias
    }

    pub fn raw(&self) -> &ExecutionRootConfig {
        &self.raw
    }

    pub fn expected_mode(&self) -> u32 {
        self.expected_mode
    }
}
