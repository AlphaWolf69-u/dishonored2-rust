//! arkOccludedSensor (6ddb40/6debc0/6df300): retained candidate query,
//! next-phase occlusion queries, and ordered visible targets. Query slots belong
//! to the world's collision-query owner, never to an individual sensor.
use super::{collision_queries::QueryId, entity_lifetime::EntityId};

/// The source manager/group slot and agent+44 weak generation. This is not
/// an entity ID: the swarm record resolves its separate entity pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentId {
    pub manager: u8,
    pub index: u32,
    pub generation: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// A physics body's actual entity identity; absent for an actor-list entry.
    pub entity: Option<EntityId>,
    /// The actual weak actor identity returned by 6c73f0/6b2710.
    pub actor: Option<AgentId>,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub body_id: u32,
    /// Full source witness for consumers such as sensor27. Ordinary occlusion
    /// retains only its body identity, exactly as before.
    pub witness: Option<Witness>,
}
#[derive(Clone, Copy, Debug)]
pub enum Witness {
    Convex(super::shape_convex_query::ConvexContact),
    Ray(super::shape_ray_query::ShapeRayContact),
}
#[derive(Clone, Debug)]
pub struct ConvexRequest {
    pub vertices: std::sync::Arc<[[f32; 4]]>,
    pub shape_type: u8,
    pub radius: f32,
    pub transform: super::shape_transform::ShapeMatrix,
    pub displacement: [f32; 3],
    pub layers: u32,
    pub required_flags: u32,
}
#[derive(Clone, Debug)]
pub enum Request {
    ///095280 main/all-hit collector; the independent initial collector is null.
    Convex(ConvexRequest),
    ///095030 collector1 preserves the closest witness for candidate refinement.
    RayClosest {
        origin: [f32; 3],
        direction: [f32; 3],
        maximum_fraction: f32,
        layers: u32,
        required_flags: u32,
    },
    Sphere {
        origin: [f32; 3],
        radius: f32,
        layers: u32,
    },
    Ray {
        origin: [f32; 3],
        direction: [f32; 3],
        layers: u32,
    },
}
pub trait Host {
    fn submit(&mut self, request: Request) -> Result<QueryId, String>;
    fn take_completed(&mut self, id: &mut QueryId, hits: &mut Vec<Hit>) -> Result<bool, String>;
    fn cancel(&mut self, id: &mut QueryId) -> Result<(), String>;
    fn body_target(&self, body: u32) -> Result<Option<Target>, String>;
    fn actor_targets(
        &self,
        origin: [f32; 3],
        radius: f32,
        targets: &mut Vec<Target>,
    ) -> Result<(), String>;
    fn position(&self, target: Target) -> Result<Option<[f32; 3]>, String>;
}
struct Candidate {
    target: Target,
    position: [f32; 3],
    query: QueryId,
}
pub struct Sensor {
    candidates: Vec<Candidate>,
    results: Vec<Target>,
    hits: Vec<Hit>,
    actors: Vec<Target>,
    query: QueryId,
    radius: Option<f32>,
    layers: u32,
    occlusion_layers: u32,
    ignored: Option<EntityId>,
    physics_only: bool,
    origin: [f32; 3],
}
impl Default for Sensor {
    fn default() -> Self {
        Self::new()
    }
}
impl Sensor {
    /// 6ddb40 initializes empty retained lists, invalid query and zero origin.
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
            results: Vec::new(),
            hits: Vec::new(),
            actors: Vec::new(),
            query: QueryId::INVALID,
            radius: None,
            layers: 0,
            occlusion_layers: 0,
            ignored: None,
            physics_only: false,
            origin: [0.; 3],
        }
    }
    /// 6de860 owns one sphere; the host uses its exact one-vertex support form.
    pub fn initialize(
        &mut self,
        radius: f32,
        layers: u32,
        occlusion_layers: u32,
        physics_only: bool,
        ignored: Option<EntityId>,
    ) -> Result<(), String> {
        if self.radius.is_some() {
            return Err("occluded sensor initialized twice".into());
        }
        self.radius = Some(radius);
        self.layers = layers;
        self.occlusion_layers = occlusion_layers;
        self.physics_only = physics_only;
        self.ignored = ignored;
        Ok(())
    }
    pub fn set_origin(&mut self, origin: [f32; 3]) {
        self.origin = origin;
    }
    pub fn results(&self) -> &[Target] {
        &self.results
    }
    /// 6debc0 first publishes the PREVIOUS candidates' ray results, then pulls
    /// the sphere results into the next candidate bank. No sorting/deduplication.
    pub fn gather(&mut self, host: &mut impl Host) -> Result<(), String> {
        let radius = self
            .radius
            .ok_or("occluded sensor sphere is not initialized")?;
        self.results.clear();
        for candidate in &mut self.candidates {
            self.hits.clear();
            if !host.take_completed(&mut candidate.query, &mut self.hits)? || self.hits.is_empty() {
                self.results.push(candidate.target);
            }
            // Native frame096240 finishes requests before this bank is cleared.
            // If a host caller polls earlier, preserve the native visible
            // result while explicitly relinquishing the discarded ticket.
            if candidate.query.is_valid() {
                host.cancel(&mut candidate.query)?;
            }
        }
        self.candidates.clear();
        self.hits.clear();
        if host.take_completed(&mut self.query, &mut self.hits)? {
            for hit in &self.hits {
                if let Some(target) = host.body_target(hit.body_id)? {
                    self.candidates.push(Candidate {
                        target,
                        position: [0.; 3],
                        query: QueryId::INVALID,
                    });
                }
            }
        }
        if !self.physics_only {
            self.actors.clear();
            host.actor_targets(self.origin, radius, &mut self.actors)?;
            self.candidates
                .extend(self.actors.iter().map(|&target| Candidate {
                    target,
                    position: [0.; 3],
                    query: QueryId::INVALID,
                }));
        }
        for candidate in &mut self.candidates {
            if let Some(position) = host.position(candidate.target)? {
                candidate.position = position;
            }
        }
        Ok(())
    }
    /// 6df300 submits the next sphere and walks retained targets backwards,
    /// removing dead/self targets before submitting each actual occlusion ray.
    pub fn submit(&mut self, host: &mut impl Host) -> Result<(), String> {
        let radius = self
            .radius
            .ok_or("occluded sensor sphere is not initialized")?;
        if self.query.is_valid()
            || self
                .candidates
                .iter()
                .any(|candidate| candidate.query.is_valid())
        {
            return Err(
                "occluded sensor submitted again before its query results were gathered".into(),
            );
        }
        self.query = host.submit(Request::Sphere {
            origin: self.origin,
            radius,
            layers: self.layers,
        })?;
        for index in (0..self.candidates.len()).rev() {
            let candidate = &mut self.candidates[index];
            if (candidate.target.actor.is_none()
                && candidate
                    .target
                    .entity
                    .is_some_and(|id| Some(id) == self.ignored))
                || host.position(candidate.target)?.is_none()
            {
                self.candidates.remove(index);
                continue;
            }
            candidate.query = host.submit(Request::Ray {
                origin: self.origin,
                direction: std::array::from_fn(|i| candidate.position[i] - self.origin[i]),
                layers: self.occlusion_layers,
            })?;
        }
        Ok(())
    }
    /// 6de3e0 cancels every outstanding ticket before owner destruction. Keep
    /// capacities for reset/restart; 6ddbe0 then drops the owned lists and sphere.
    pub fn reset(&mut self, host: &mut impl Host) -> Result<(), String> {
        self.reset_with_cancel(|id| host.cancel(id))
    }
    fn reset_with_cancel(
        &mut self,
        mut cancel: impl FnMut(&mut QueryId) -> Result<(), String>,
    ) -> Result<(), String> {
        for candidate in &mut self.candidates {
            cancel(&mut candidate.query)?;
        }
        self.candidates.clear();
        cancel(&mut self.query)?;
        self.results.clear();
        self.hits.clear();
        self.actors.clear();
        Ok(())
    }
    pub fn retire(&mut self, host: &mut impl Host) -> Result<(), String> {
        self.retire_with_cancel(|id| host.cancel(id))
    }
    /// Teardown does not resolve target entities or require a live level roster.
    pub fn retire_with_cancel(
        &mut self,
        cancel: impl FnMut(&mut QueryId) -> Result<(), String>,
    ) -> Result<(), String> {
        self.reset_with_cancel(cancel)?;
        self.radius = None;
        self.candidates = Vec::new();
        self.results = Vec::new();
        self.hits = Vec::new();
        self.actors = Vec::new();
        Ok(())
    }
}
