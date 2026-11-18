//! On-demand F8 observation of the existing body/contact/render owners.
use super::*;
use crate::gameplay::{collision_hit::ShapeHitInfo, entity_lifetime::EntityId};
use std::fmt::Write;

impl PhysicsScene {
    /// Opt-in fixture read of the actual completed solver body, not actor velocity.
    pub(in crate::app) fn observe_rigid_completed_motion(
        &self,live:&crate::gameplay::entity_lifetime::LiveEntities,id:EntityId,
    )->Result<Option<(i32,u32,[f32;4],[f32;4],[f32;4])>,String>{
        let Some(frame)=self.simulation.as_ref().and_then(|driver|driver.observed_completed_frame()) else{return Ok(None)};
        if live.get(id).is_none(){return Err("throw motion entity generation retired".into())}
        let index=*self.shared_indices.get(&id).ok_or("throw motion Scene identity absent")?;
        let handle=self.document_rigid(index)?.ok_or("throw motion rigid component absent")?;
        let body=self.rigid_component(handle)?.body().ok_or("throw motion body absent")?;
        let world=self.world.read().map_err(|_|"throw motion world poisoned")?;
        let body=world.storage.bodies.get(body).ok_or("throw motion body generation stale")?;
        let motion=world.storage.motions.get(body.motion_id).ok_or("throw motion actual motion absent")?;
        Ok(Some((frame,body.id,motion.pose.center,motion.linear_velocity,motion.world_angular_velocity())))
    }

    /// Reads the accepted solver's retained rows; never submits a query or
    /// substitutes the level navigation mesh for the actual physical world.
    pub(in crate::app) fn observe_rigid_world_support(
        &self,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
        id: EntityId,
    ) -> Result<Option<crate::gameplay::physics_world::contacts::WorldSupportObservation>, String> {
        use crate::gameplay::component_state::entity_lookup::ComponentLookupState;
        if live.get(id).is_none() { return Err("world-support entity generation retired".into()); }
        let index = *self.shared_indices.get(&id).ok_or("world-support Scene identity absent")?;
        let entity = self.entities.get(index).ok_or("world-support Scene entity absent")?;
        let range = self.rigid_class_range()?;
        let Some(handle) = entity.components.find_component(range, [0x20, 0], |handle| {
            let component = self.rigid_component(handle)?;
            Ok(ComponentLookupState {
                flags: component.owner.flags,
                start_phases: self.declarations.get(&handle.packed()).map(|decl| decl.scheduling.start_phases),
            })
        })? else { return Ok(None) };
        let Some(body) = self.rigid_component(handle)?.body() else { return Ok(None) };
        let storage = self.world.read().map_err(|_| "world-support physics owner poisoned")?;
        Ok(self.simulation.as_ref().and_then(|driver| driver.observe_world_support(&storage.storage, body)))
    }

    pub(in crate::app) fn write_rigid_diagnostics(
        &self,
        world: &bevy::prelude::World,
        eye: [f32; 3],
        last_hit: Option<(Option<EntityId>, ShapeHitInfo)>,
        report: &mut String,
    ) {
        let guard = match self.world.read() {
            Ok(guard) => guard,
            Err(_) => { let _ = writeln!(report, "\nRigid diagnostics: world owner poisoned"); return; }
        };
        let storage = &guard.storage;
        let _ = writeln!(report, "\nCanonical rigid owners (nearest32 plus last actual melee contact): last_hit={last_hit:?} frame_idle={:?} active_bodies={}",
            self.simulation.as_ref().map(|driver| driver.is_idle()), storage.bodies.active_body_ids().len());
        let _ = writeln!(report, "  physics_step duration={:?} contact_distance_scale={:?} tolerance={}",
            storage.step.duration, storage.step.contact_distance_scale, storage.info.collision_tolerance);
        if let Some((_, hit)) = last_hit {
            if let Some(id) = hit.body_id {
                let body = storage.bodies.get(id);
                let _ = writeln!(report, "  last_contact_body id={id:#x} current={body:?}");
                if let Some(driver) = self.simulation.as_ref() { driver.write_body_diagnostics(storage, id, report); }
            }
        }
        let mut nearest = Vec::new();
        for (family, pool) in [&self.rigid_bodies, &self.throwables].into_iter().enumerate() {
            for &slot in pool.roster.indices.iter().take(pool.roster.used.max(0) as usize) {
                let Some(component) = pool.get(slot) else { continue };
                let Some(index) = component.owner.entity else { continue };
                let Some(entity) = self.entities.get(index) else { continue };
                let distance = entity.frame.pose.position.iter().zip(eye).map(|(a,b)|(a-b)*(a-b)).sum::<f32>();
                let selected = last_hit.is_some_and(|(target, hit)| target == Some(entity.shared)
                    || hit.body_id.is_some_and(|id| component.body() == Some(id)));
                nearest.push((!selected, distance, family, slot));
            }
        }
        nearest.sort_by(|a,b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
        let mut ordinary = 0;
        for (not_selected, distance, family, slot) in nearest {
            if not_selected { if ordinary == 32 { continue; } ordinary += 1; }
            let pool = if family == 0 { &self.rigid_bodies } else { &self.throwables };
            let Some(component) = pool.get(slot) else { continue };
            let Some(index) = component.owner.entity else { continue };
            let entity = &self.entities[index];
            let class = self.classes[if family == 0 { 8 } else { 12 }];
            let movement = component.movement_body();
            let _ = writeln!(report, "  rigid name={} entity={:?} index={index} selected={} distance={} class={class} slot={slot} generation={} flags={:#x} entity_flags={:#x}/{:?}/{:#x} body={:?} movement={movement:?} timespace={} frame={:?} velocities={:?}",
                entity.name, entity.shared, !not_selected, distance.sqrt(), component.owner.generation,
                component.owner.flags, entity.entity_flags, self.entity_flags_d4.get(index), entity.entity_flags_d7,
                component.body(), entity.timespace, entity.frame, component.velocities);
            if let Ok(movement) = movement {
                if let Some(link) = self.actors.bodies.get(movement) {
                    let _ = writeln!(report, "    interface actor={:?} body={:#x}", link.actor, link.body_id);
                    if let Some(actor) = link.actor.and_then(|actor| self.actors.actors.get_at(actor)) {
                        let _ = writeln!(report, "    actor current={} default={} flags={:#x} parent={:?} binding_entity={:?} frame={:?} motion={:?}",
                            actor.current_body, actor.default_body, actor.flags, actor.parent, actor.binding.entity,
                            actor.frame.published(), actor.motion);
                    }
                }
            }
            if let Some(body) = component.body().and_then(|id| storage.bodies.get(id)) {
                let _ = writeln!(report, "    body={body:?}");
                let _ = writeln!(report, "    mass={:?} quality={:?} material={:?}",
                    storage.bodies.mass_properties(body.id), storage.qualities.get(body.quality_id),
                    storage.materials.library.get(body.material_id));
                if let Some(motion) = storage.motions.get(body.motion_id) {
                    let _ = writeln!(report, "    motion={motion:?} world_angular={:?}", motion.world_angular_velocity());
                }
                for selection in 0..8 {
                    match body.selected_shape(selection) {
                        Ok(shape) => if let Some(asset) = guard.geometry.shapes.get(shape).and_then(Option::as_ref) {
                            let _ = writeln!(report, "    shape selection={selection} resource={shape} path={} header={:?} local_bounds={:?}", asset.path, asset.header, asset.bounds);
                        } else { let _ = writeln!(report, "    shape selection={selection} resource={shape} retired=true"); },
                        Err(error) => { let _ = writeln!(report, "    shape selection={selection} error={error}"); }
                    }
                }
                if let Some(driver) = self.simulation.as_ref() { driver.write_body_diagnostics(storage, body.id, report); }
            }
            if let Some(&placement) = self.rigid_placements.get(&index) {
                let _ = writeln!(report, "    render placement={placement} initial_body_frame={:?}", self.initial_rigid_frames.get(&index));
                if let Some(model) = world.get_resource::<super::super::RenderSourcePlacements>().and_then(|owner| owner.0.get(placement)) {
                    let _ = writeln!(report, "    placed resource={} position={:?} rotation={:?} scale={:?}", model.resource_path, model.position, model.rotation, model.scale);
                }
                if let Some(queue) = world.get_resource::<super::super::SceneRenderQueue>() {
                    for instance in queue.instances.iter().filter(|instance| instance.source.is_some_and(|source| match source.surface {
                        super::super::RenderModelSurface::Bmodel { placement: candidate, .. } | super::super::RenderModelSurface::Bmd6 { placement: candidate, .. } => candidate == placement,
                        _ => false,
                    })).take(4) {
                        let _ = writeln!(report, "    render_instance visible={} transform={:?} bounds={:?}", instance.visible, instance.transform, instance.bounds);
                    }
                }
            }
            if let Some(models) = world.get_resource::<super::super::world_models::WorldModels>() {
                let _ = writeln!(report, "    runtime_render pose={:?} visibility={:?}", models.publication_pose(entity.shared), models.publication_visibility(entity.shared));
            }
        }
    }
}
