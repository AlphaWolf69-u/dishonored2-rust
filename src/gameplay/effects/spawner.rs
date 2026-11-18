//! arkKisculeNodeEntitySpawnerControl (14079dc40): the graph requests one
//! entity per node; the host constructs it with the main world and returns
//! the created entity or the failure, which the node then reports.
use crate::gameplay::event_graph::EntityReference;
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct EntitySpawnRequest {
    pub owner: String,
    pub node: u32,
    /// m_entityDef.
    pub declaration: String,
    /// m_target: its origin and axis replace m_location/m_rotation.
    pub target: Option<EntityReference>,
    pub location: [f32; 3],
    pub rotation: [f32; 3],
    /// m_spawnTag, applied through 14087c360.
    pub spawn_tag: String,
}

/// Requests from graph evaluation and the host's results, in arrival order.
#[derive(Default)]
pub struct EntitySpawner {
    requests: Mutex<Vec<EntitySpawnRequest>>,
    results: Mutex<Vec<(String, u32, Result<String, String>)>>,
}
impl EntitySpawner {
    pub fn submit(&self, request: EntitySpawnRequest) -> Result<(), String> {
        self.requests
            .lock()
            .map_err(|_| "entity spawner requests poisoned")?
            .push(request);
        Ok(())
    }
    pub fn take_requests(&self) -> Result<Vec<EntitySpawnRequest>, String> {
        Ok(std::mem::take(
            &mut *self
                .requests
                .lock()
                .map_err(|_| "entity spawner requests poisoned")?,
        ))
    }
    /// `Ok` carries the created entity's name for m_createdEntity.
    pub fn complete(&self, owner: String, node: u32, result: Result<String, String>) -> Result<(), String> {
        self.results
            .lock()
            .map_err(|_| "entity spawner results poisoned")?
            .push((owner, node, result));
        Ok(())
    }
    pub fn take_results(&self) -> Result<Vec<(String, u32, Result<String, String>)>, String> {
        Ok(std::mem::take(
            &mut *self
                .results
                .lock()
                .map_err(|_| "entity spawner results poisoned")?,
        ))
    }
}
