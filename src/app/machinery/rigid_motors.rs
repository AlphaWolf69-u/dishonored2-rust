//! View LOD and particle collision consume the canonical motor bodies.
use crate::{
    entities::machinery::recipes::{AssemblyRecipe, BodyCollision},
    gameplay::{
        entity_lod::{self, EntityLod, LodDistances, LodView},
        havok_fx::{dynamic_collide::{MotionRecord, MovingBody, MovingWorld}, world_collide::SurfaceMaterial},
        physics_world::storage::{JointId, WorldStorage},
    },
};
use std::sync::Arc;
use super::super::{physics_scene::PhysicsScene, shadow_updates::ModelShadowSources, RenderSourcePlacements, SceneRenderQueue};

struct Endpoint {
    placement: usize,
    lod: EntityLod,
    previous_visibility: bool,
    render_source: Option<usize>,
    collision: Option<Arc<BodyCollision>>,
}

pub(super) struct Motor {
    joint: JointId,
    ends: [Endpoint; 2],
    far: bool,

}

impl Motor {
    pub(super) fn new(assembly: AssemblyRecipe, joint: JointId) -> Result<Self, String> {
        let warmup = crate::platform::console::shared().read().cvars.int("lod_dist_warmup_framecount").unwrap_or(3);
        let hinge = &assembly.joints[0];
        let endpoint = |name: Option<&str>| -> Result<Endpoint, String> {
            let body = assembly.bodies.iter().find(|b| Some(b.entity_id.as_str()) == name)
                .ok_or("admitted motor missing endpoint recipe")?;
            Ok(Endpoint { placement: body.placement, lod: EntityLod::spawned(warmup), previous_visibility: false,
                render_source: None, collision: body.collision.clone() })
        };
        Ok(Self { joint, ends: [endpoint(hinge.entity_a.as_deref())?, endpoint(hinge.entity_b.as_deref())?],
            far: false })
    }

    pub(super) fn observe(&self, joint: JointId) -> Option<([usize; 2], [i32; 2], bool)> {
        (self.joint == joint).then(|| (self.ends.each_ref().map(|e| e.placement),
            self.ends.each_ref().map(|e| e.lod.level), self.far))
    }

    pub(super) fn update_lod(&mut self, physics: &mut PhysicsScene, view: &LodView,
        distances: &LodDistances, sources: &ModelShadowSources, scene: &SceneRenderQueue,
        placements: &RenderSourcePlacements) -> Result<(), String> {
        for end in &mut self.ends {
            if end.render_source.is_none() && !sources.0.is_empty() {
                // usize::MAX records an origin-only owner, once render sources exist.
                end.render_source = Some(sources.0.iter().position(|s| s.placement == end.placement).unwrap_or(usize::MAX));
            }
            let source = end.render_source.and_then(|index| sources.0.get(index));
            let visible_now = source.is_some_and(|s| scene.instances[s.instances.clone()].iter().any(|i| i.visible));
            let visible = visible_now || end.previous_visibility;
            end.previous_visibility = visible_now;
            if !end.lod.listed() { continue; }
            let level = match source {
                Some(s) => entity_lod::render_level(s.bounds, s.flags_1c30, visible, false, view, distances),
                None => match placements.0.get(end.placement) {
                    Some(p) => entity_lod::origin_level(p.position, view, distances),
                    None => continue,
                },
            };
            end.lod.request(level);
        }
        let far = self.ends.iter().all(|end| end.lod.level > 5);
        if far != self.far {
            physics.motor_lod(self.joint, far)?;
            self.far = far;
        }
        Ok(())
    }

    pub(super) fn publish_particle_bodies(&mut self, storage: &WorldStorage, moving: &mut MovingWorld) -> Result<(), String> {
        let bodies = storage.driven_bodies(self.joint);
        for (index, end) in self.ends.iter_mut().enumerate() {
            let id = moving.bodies.len() as u32;
            // Stable publication slots keep prior per-particle neighbour IDs generation-safe.
            moving.bodies.push(None);
            let Some(bodies) = bodies else { end.collision = None; continue; };
            let Some(shape) = &end.collision else { continue; };
            let Some(body) = storage.bodies.get(bodies[index]) else { end.collision = None; continue; };
            if body.flags & 2 == 0 || body.broadphase_id == u32::MAX { continue; }
            let motion = storage.motions.get(body.motion_id).ok_or("particle motor motion absent")?;
            let material = if let Some(material) = &shape.material {
                let index = u16::try_from(moving.materials.len()).map_err(|_| "particle material slots exhausted")?;
                moving.materials.push(SurfaceMaterial::from_physics(material));
                index
            } else { u16::MAX };
            moving.bodies[id as usize] = Some(MovingBody {
                id, active: body.flags & 8 != 0, transform: body.transform,
                motion: MotionRecord { centre: motion.pose.center, orientation: motion.pose.orientation,
                    linear: motion.linear_velocity, angular: motion.angular_velocity },
                bounds: storage.quantization.packed.unpack_bounds(body.bounds), shapes: shape.shapes.clone(),
                material, filter: body.filter_info,
            });
        }
        Ok(())
    }
}
