//!79dc40's host handoff. Receipts retain the actual graph generation/node;
//!entity outputs retain managed generations rather than reusable names.
use crate::gameplay::{
    entity_lifetime::EntityId, event_graph::EntityReference, reference_frame::ReferenceFrame,
};
use std::{collections::HashSet, sync::Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Receipt {
    pub instance: u64,
    pub node: u32,
    pub attempt: u64,
}
#[derive(Clone, Debug)]
pub struct EntitySpawnRequest {
    pub receipt: Receipt,
    ///Provenance only; never used to route a continuation.
    pub owner: String,
    pub graph_entity: Option<EntityId>,
    pub declaration: String,
    pub target: Option<EntityReference>,
    ///79dc40 resolves the target transform at activation, before the factory.
    pub target_frame: Option<ReferenceFrame>,
    pub target_spawn_tag: Option<String>,
    pub location: [f32; 3],
    pub rotation: [f32; 3],
    pub spawn_tag: String,
}
pub struct Reply {
    pub receipt: Receipt,
    pub result: Result<EntityId, String>,
}
struct State {
    accepting: bool,
    requests: Vec<EntitySpawnRequest>,
    results: Vec<Reply>,
    pending: HashSet<Receipt>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            accepting: true,
            requests: Vec::new(),
            results: Vec::new(),
            pending: HashSet::new(),
        }
    }
}
#[derive(Default)]
pub struct EntitySpawner {
    state: Mutex<State>,
}
impl EntitySpawner {
    pub fn submit(&self, request: EntitySpawnRequest) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "entity spawner mailbox poisoned")?;
        if !state.accepting {
            return Err("entity spawner admission is closed".into());
        }
        if request.receipt.instance == 0
            || request.receipt.attempt == 0
            || !state.pending.insert(request.receipt)
        {
            return Err("entity spawner receipt is invalid or already accepted".into());
        }
        state.requests.push(request);
        Ok(())
    }
    pub fn take_requests(&self) -> Result<Vec<EntitySpawnRequest>, String> {
        Ok(std::mem::take(
            &mut self
                .state
                .lock()
                .map_err(|_| "entity spawner mailbox poisoned")?
                .requests,
        ))
    }
    pub fn complete(
        &self,
        receipt: Receipt,
        result: Result<EntityId, String>,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "entity spawner mailbox poisoned")?;
        if !state.pending.contains(&receipt)
            || state.results.iter().any(|reply| reply.receipt == receipt)
        {
            return Err(
                "entity spawner completed an unaccepted or already completed receipt".into(),
            );
        }
        state.results.push(Reply { receipt, result });
        Ok(())
    }
    pub fn take_results(&self) -> Result<Vec<Reply>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "entity spawner mailbox poisoned")?;
        let results = std::mem::take(&mut state.results);
        for reply in &results {
            state.pending.remove(&reply.receipt);
        }
        Ok(results)
    }
    pub fn close(&self) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "entity spawner mailbox poisoned")?
            .accepting = false;
        Ok(())
    }
    pub fn idle(&self) -> Result<bool, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "entity spawner mailbox poisoned")?
            .pending
            .is_empty())
    }
    pub fn references(&self, id: EntityId) -> Result<bool, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "entity spawner mailbox poisoned")?
            .requests
            .iter()
            .any(|request| {
                (request.graph_entity == Some(id)
                    || request.target == Some(EntityReference::Managed(id)))
            }))
    }
}
