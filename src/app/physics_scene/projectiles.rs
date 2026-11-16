//! Projectile finite-frame actor hooks. One canonical component/body registry.
use super::*;
use crate::gameplay::{
    collision_user_data::CollisionUserData,
    component_state::ComponentInstance,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities, Stage},
    pickable_component::CheckedComponent,
    projectiles::{self, Contact, ContactFlags, Handoff, Hit, RenderState, SpawnRequest},
    reference_frame::ReferenceFrame,
    shape_ray_query::ShapeRayContact,
};
use bevy_math::Vec3;

pub(in crate::app) struct ProjectileStep {
    pub entity: EntityId,
    pub frame: ReferenceFrame,
    pub velocity: Vec3,
    pub hidden: bool,
    pub retire: bool,
    pub handoff: Handoff,
    pub witness: Option<ShapeRayContact>,
    /// Bounce torque goes to the projectile; ordinary contact goes to the witnessed target body.
    pub impulse: Option<Vec3>,
    pub contact_impulse: Option<Vec3>,
}
impl PhysicsScene {
    pub(in crate::app) fn configure_projectile(
        &mut self,
        id: EntityId,
        request: SpawnRequest,
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("projectile actual actor absent")?;
        let handle = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|v| ComponentHandle::from_packed(*v).ok())
            .find(|h| h.class_id == self.classes[13])
            .ok_or("entity lacks actual Projectile component")?;
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload absent")?;
        if component.request.is_some() || component.runtime.is_some() {
            return Err("projectile source already configured".into());
        }
        component.request = Some(request);
        Ok(())
    }
    pub(super) fn initialize_projectile(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile initializer received another actual class".into());
        }
        let index = self
            .projectiles
            .get(handle.slot)
            .ok_or("projectile payload absent")?
            .owner
            .entity
            .ok_or("projectile unlinked")?;
        let throwable = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|v| ComponentHandle::from_packed(*v).ok())
            .find(|h| h.class_id == self.classes[12])
            .ok_or("Projectile Initialize requires actual attached Throwable")?;
        let generation = self.rigid_component(throwable)?.owner.generation;
        let id = self.entities[index].shared;
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload absent")?;
        let request = component
            .request
            .take()
            .ok_or("projectile birth has no accepted source request")?;
        component.initialize(
            id,
            request,
            CheckedComponent {
                handle: throwable,
                generation,
            },
        )
    }
    ///629590 is driven with this entity's actual time-space clock. The caller
    ///publishes the returned frame through its existing reference-frame actor,
    ///and performs real Throwable handoff before the next physics step.
    pub(in crate::app) fn step_projectile(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
        dt: f32,
        now: f64,
        gravity: Vec3,
        render: Option<RenderState>,
        target: Option<Vec3>,
        damage_factor: f32,
        overlapping_spawner: bool,
        mut random: impl FnMut() -> f32,
    ) -> Result<ProjectileStep, String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile update class mismatch".into());
        }
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload absent")?;
        let hidden = component.hidden;
        let mut runtime = component
            .runtime
            .take()
            .ok_or("Projectile has not initialized")?;
        let result = (|| {
            if live.stage(runtime.identity) != Some(Stage::Spawned) {
                return Err("projectile identity is stale or retiring".into());
            }
            let mut output = ProjectileStep {
                entity: runtime.identity,
                frame: runtime.frame(),
                velocity: runtime.velocity,
                hidden,
                retire: false,
                handoff: Handoff::None,
                witness: None,
                impulse: None,
                contact_impulse: None,
            };
            if let Some(removal) = runtime.removal.as_mut() {
                output.retire = removal.poll(now, render);
            }
            if output.retire || runtime.has_hit_and_stopped || runtime.picked_up {
                return Ok(output);
            }
            runtime.home(dt, target);
            let Some(flight) = runtime.flight(dt, gravity) else {
                return Ok(output);
            };
            let direction = (flight.end - flight.start).normalize_or_zero();
            runtime.orient_for_segment(dt, direction);
            let query_end = flight.end + direction * runtime.forward_extent;
            let world = self
                .world
                .read()
                .map_err(|_| "projectile canonical query world poisoned")?;
            let component = self
                .projectiles
                .get_mut(handle.slot)
                .ok_or("projectile payload absent")?;
            component.ray_hits(
                &world,
                &mut self.executor,
                flight.start,
                query_end,
                overlapping_spawner,
            )?;
            let mut selected = None;
            for hit in &self
                .projectiles
                .get(handle.slot)
                .ok_or("projectile payload absent")?
                .hits
            {
                let target = if hit.target.user_data == 0 {
                    None
                } else {
                    let index = CollisionUserData::decode(hit.target.user_data)?
                        .entity(|h| self.owner(h).ok())?;
                    Some(
                        self.entities
                            .get(index)
                            .ok_or("projectile contact actor absent")?
                            .shared,
                    )
                };
                if target == Some(runtime.identity)
                    || overlapping_spawner && target == runtime.attacker
                    || target.is_some_and(|id| runtime.pass_through_entities.contains(&id))
                {
                    continue;
                }
                // The source's shooter overlap collector is owned by the
                //source adapter; unignored actor contacts remain real hits.
                selected = Some((*hit, target));
                break;
            }
            let surface = selected
                .map(|(hit, _)| {
                    if hit.target.material_id == u16::MAX {
                        Ok::<_, String>((true, true))
                    } else {
                        let m = world
                            .storage
                            .materials
                            .material_for_id(hit.target.material_id)?;
                        Ok((m.projectile_can_stick, m.projectile_can_break))
                    }
                })
                .transpose()?;
            drop(world);
            if let Some((witness, target_id)) = selected {
                let hit = Hit {
                    position: Vec3::new(
                        witness.position[0],
                        witness.position[1],
                        witness.position[2],
                    ),
                    normal: Vec3::new(witness.normal[0], witness.normal[1], witness.normal[2]),
                    direction: (flight.end - flight.start).normalize_or_zero(),
                    region: 1,
                };
                if let Some(parameters) = runtime.hit_parameters(hit, damage_factor, false) {
                    self.damage_collision_target(
                        live,
                        events,
                        witness.target.user_data,
                        &parameters,
                    )?;
                }
                let (can_stick, can_break) = surface.ok_or("projectile material result absent")?;
                let class_matches = |id: EntityId, name: &[u8]| -> Result<bool, String> {
                    let (_, mut class) = live.get(id).ok_or("projectile target identity stale")?;
                    loop {
                        let classes = self.types.classes();
                        let metadata = classes
                            .metadata(class)
                            .ok_or("target class metadata absent")?;
                        if classes.name_text(metadata.name) == Some(name) {
                            return Ok(true);
                        }
                        let Some(base) = classes.hierarchy().node(class).and_then(|n| n.base)
                        else {
                            return Ok(false);
                        };
                        class = base;
                    }
                };
                let player = target_id
                    .map(|id| class_matches(id, b"arkPlayer"))
                    .transpose()?
                    .unwrap_or(false);
                let npc = target_id
                    .map(|id| class_matches(id, b"arkNPC"))
                    .transpose()?
                    .unwrap_or(false);
                if npc {
                    return Err(
                        "projectile NPC contact requires actual NPC armor/deflection/sleep owner"
                            .into(),
                    );
                }
                let contact = Contact {
                    player,
                    npc,
                    world: true,
                    can_stick,
                    can_break,
                    angle: projectiles::incidence_angle(hit.normal, runtime.velocity),
                    stick_blocked_by_pickable: false,
                    absorbed_by_armor: false,
                    desired_deflection: 0,
                    npc_break_probability: None,
                };
                let mut flags = ContactFlags {
                    always_break_on_hit: runtime.flags.always_break_on_hit,
                    always_stick_to_npcs: runtime.flags.always_stick_to_npcs,
                    pinned_limb: false,
                    bounce_once: runtime.flags.bounce_once_instead_of_breaking,
                };
                let response =
                    projectiles::response(&runtime.declaration, &contact, &mut flags, &mut random);
                runtime.flags.bounce_once_instead_of_breaking = flags.bounce_once;
                let motion = runtime.apply_response(response, hit, flight.velocity, &mut random);
                if matches!(motion.handoff, Handoff::Stick) {
                    let axis = runtime.sticking_axis(hit.normal, &mut random);
                    let frame = runtime.stick_frame(hit, axis, false, false, random());
                    runtime.position = Vec3::from_array(frame.position);
                    runtime.axis = bevy_math::Mat3::from_cols_array_2d(&frame.basis);
                }
                if response != projectiles::Response::None {
                    runtime.schedule_removal(now);
                }
                if response != projectiles::Response::Bounce {
                    if let Some(body_id) = witness.target.body_id {
                        let world = self.world.read().map_err(|_| "projectile world poisoned")?;
                        let body = world
                            .storage
                            .bodies
                            .get(body_id)
                            .ok_or("projectile hit body generation stale")?;
                        if body.flags & 5 == 0 {
                            let damage = runtime.declaration.damage.as_ref().ok_or(
                                "projectile contact impulse has no required damage declaration",
                            )?;
                            let factor = if damage.maximum > 0. {
                                (damage.maximum + runtime.damage_increment) / damage.maximum
                            } else {
                                1.
                            };
                            output.contact_impulse = Some(
                                hit.direction * (runtime.declaration.contact_impulse * factor),
                            );
                        }
                    }
                }
                output.handoff = motion.handoff;
                output.impulse = motion.impulse;
                output.hidden = motion.hidden;
                output.witness = Some(witness);
                let _ = target_id;
            } else {
                runtime.publish_flight(flight);
            }
            output.frame = runtime.frame();
            output.velocity = runtime.velocity;
            output.retire |= flight.out_of_range;
            Ok(output)
        })();
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload disappeared")?;
        if let Ok(output) = &result {
            component.hidden = output.hidden;
        }
        component.runtime = Some(runtime);
        result
    }
}
