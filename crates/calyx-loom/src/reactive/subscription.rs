//! Public subscription API over the reactive trigger engine.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::str::FromStr;

use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, Result};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::{ReactiveEngine, TriggerCondition, TriggerFired, TriggerId};
use crate::error::{
    CALYX_REACTIVE_DRAIN_OVERFLOW, CALYX_REACTIVE_REGISTRY_FULL,
    CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND, loom_error,
};

pub const DEFAULT_MAX_SUBSCRIPTIONS: usize = 256;
pub const DEFAULT_MAX_DRAIN_BUF: usize = 1024;

const SUBSCRIPTION_LEDGER_TAG: &str = "reactive_subscription_v1";
const SUBSCRIPTION_CREATED: &str = "SUBSCRIPTION_CREATED";
const SUBSCRIPTION_REMOVED: &str = "SUBSCRIPTION_REMOVED";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubscriptionId(Uuid);

impl SubscriptionId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for SubscriptionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SubscriptionId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Clone, Debug)]
pub struct SubscriptionHandle {
    pub id: SubscriptionId,
    pub trigger_id: TriggerId,
    pub condition: TriggerCondition,
    pub max_drain_buf: usize,
    drain_buf: VecDeque<TriggerFired>,
    overflowed: bool,
}

impl SubscriptionHandle {
    pub fn pending_len(&self) -> usize {
        self.drain_buf.len()
    }

    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    fn push(&mut self, event: TriggerFired) {
        if self.drain_buf.len() >= self.max_drain_buf {
            self.drain_buf.pop_front();
            self.overflowed = true;
        }
        self.drain_buf.push_back(event);
    }

    fn drain(&mut self) -> Vec<TriggerFired> {
        self.drain_buf.drain(..).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionDelta {
    pub subscription_id: SubscriptionId,
    pub events: Vec<TriggerFired>,
    pub overflowed: bool,
}

#[derive(Clone, Debug)]
pub struct SubscriptionStore {
    handles: HashMap<SubscriptionId, SubscriptionHandle>,
    max_subscriptions: usize,
    max_drain_buf: usize,
}

impl SubscriptionStore {
    pub fn new(max_subscriptions: usize, max_drain_buf: usize) -> Self {
        Self {
            handles: HashMap::new(),
            max_subscriptions: max_subscriptions.max(1),
            max_drain_buf: max_drain_buf.max(1),
        }
    }

    pub fn subscribe(
        &mut self,
        condition: TriggerCondition,
        trigger_id: TriggerId,
    ) -> Result<SubscriptionId> {
        if self.handles.len() >= self.max_subscriptions {
            return Err(loom_error(
                CALYX_REACTIVE_REGISTRY_FULL,
                format!(
                    "subscription store full at {} entries; cannot admit trigger {trigger_id}",
                    self.max_subscriptions
                ),
            ));
        }
        let id = SubscriptionId::new();
        self.handles.insert(
            id,
            SubscriptionHandle {
                id,
                trigger_id,
                condition,
                max_drain_buf: self.max_drain_buf,
                drain_buf: VecDeque::with_capacity(self.max_drain_buf),
                overflowed: false,
            },
        );
        Ok(id)
    }

    pub fn remove(&mut self, id: SubscriptionId) -> Option<SubscriptionHandle> {
        self.handles.remove(&id)
    }

    pub fn get(&self, id: SubscriptionId) -> Option<&SubscriptionHandle> {
        self.handles.get(&id)
    }

    pub fn len(&self) -> usize {
        self.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.max_subscriptions
    }

    pub fn dispatch(&mut self, event: &TriggerFired) {
        for handle in self.handles.values_mut() {
            if handle.trigger_id == event.trigger_id {
                handle.push(event.clone());
            }
        }
    }

    pub fn observe_delta(&mut self, id: SubscriptionId) -> Result<Vec<TriggerFired>> {
        let handle = self
            .handles
            .get_mut(&id)
            .ok_or_else(|| subscription_not_found(id))?;
        if handle.overflowed {
            return Err(drain_overflow(id, handle.pending_len()));
        }
        Ok(handle.drain())
    }

    pub fn observe_delta_report(&mut self, id: SubscriptionId) -> Result<SubscriptionDelta> {
        let handle = self
            .handles
            .get_mut(&id)
            .ok_or_else(|| subscription_not_found(id))?;
        let overflowed = handle.overflowed;
        handle.overflowed = false;
        Ok(SubscriptionDelta {
            subscription_id: id,
            events: handle.drain(),
            overflowed,
        })
    }
}

impl Default for SubscriptionStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_SUBSCRIPTIONS, DEFAULT_MAX_DRAIN_BUF)
    }
}

impl ReactiveEngine {
    pub fn with_subscription_caps(
        clock: std::sync::Arc<dyn Clock>,
        max_triggers: usize,
        max_queue_depth: usize,
        max_audit_entries: usize,
        max_subscriptions: usize,
        max_drain_buf: usize,
    ) -> Self {
        let mut engine = Self::with_caps(clock, max_triggers, max_queue_depth, max_audit_entries);
        engine.subscriptions = SubscriptionStore::new(max_subscriptions, max_drain_buf);
        engine
    }

    pub fn subscribe(
        &mut self,
        condition: TriggerCondition,
        owner: Option<String>,
    ) -> Result<SubscriptionId> {
        let trigger_id = self.register(condition.clone(), owner)?;
        match self.subscriptions.subscribe(condition, trigger_id) {
            Ok(id) => Ok(id),
            Err(error) => {
                self.deregister(trigger_id);
                Err(error)
            }
        }
    }

    pub fn unsubscribe(&mut self, id: SubscriptionId) -> Result<()> {
        let handle = self
            .subscriptions
            .remove(id)
            .ok_or_else(|| subscription_not_found(id))?;
        self.deregister(handle.trigger_id);
        Ok(())
    }

    pub fn observe_delta(&mut self, id: SubscriptionId) -> Result<Vec<TriggerFired>> {
        self.subscriptions.observe_delta(id)
    }

    pub fn observe_delta_report(&mut self, id: SubscriptionId) -> Result<SubscriptionDelta> {
        self.subscriptions.observe_delta_report(id)
    }

    pub fn observe_delta_stream(
        &mut self,
        id: SubscriptionId,
    ) -> Result<std::vec::IntoIter<TriggerFired>> {
        Ok(self.observe_delta(id)?.into_iter())
    }

    pub fn subscriptions(&self) -> &SubscriptionStore {
        &self.subscriptions
    }

    pub fn subscribe_durable<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        condition: TriggerCondition,
        owner: Option<String>,
    ) -> Result<SubscriptionId> {
        let id = self.subscribe(condition, owner.clone())?;
        let handle = self.subscriptions.get(id).expect("just inserted").clone();
        if let Err(error) = append_subscription_ledger(vault, SUBSCRIPTION_CREATED, &handle, owner)
        {
            self.subscriptions.remove(id);
            self.deregister(handle.trigger_id);
            return Err(error);
        }
        Ok(id)
    }

    pub fn unsubscribe_durable<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        id: SubscriptionId,
    ) -> Result<()> {
        let handle = self
            .subscriptions
            .remove(id)
            .ok_or_else(|| subscription_not_found(id))?;
        self.deregister(handle.trigger_id);
        append_subscription_ledger(vault, SUBSCRIPTION_REMOVED, &handle, None).map(|_| ())
    }

    pub(crate) fn dispatch_to_subscriptions(&mut self, event: &TriggerFired) {
        self.subscriptions.dispatch(event);
    }
}

fn append_subscription_ledger<C: Clock>(
    vault: &AsterVault<C>,
    action: &'static str,
    handle: &SubscriptionHandle,
    owner: Option<String>,
) -> Result<calyx_core::LedgerRef> {
    let payload = serde_json::to_vec(&json!({
        "tag": SUBSCRIPTION_LEDGER_TAG,
        "action": action,
        "subscription_id": handle.id.to_string(),
        "trigger_id": handle.trigger_id.to_string(),
        "condition": handle.condition,
        "owner": owner,
        "max_drain_buf": handle.max_drain_buf,
    }))
    .map_err(|error| {
        loom_error(
            CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND,
            format!("encode subscription ledger payload: {error}"),
        )
    })?;
    RedactionPolicy::check_payload(&payload)?;
    vault.append_ledger_entry(
        EntryKind::Guard,
        SubjectId::Guard(format!("subscription:{}", handle.id).into_bytes()),
        payload,
        ActorId::Service("calyx-loom-reactive".to_string()),
    )
}

fn subscription_not_found(id: SubscriptionId) -> calyx_core::CalyxError {
    loom_error(
        CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND,
        format!("reactive subscription {id} is not registered"),
    )
}

fn drain_overflow(id: SubscriptionId, retained: usize) -> calyx_core::CalyxError {
    loom_error(
        CALYX_REACTIVE_DRAIN_OVERFLOW,
        format!("reactive subscription {id} overflowed; {retained} events retained"),
    )
}
