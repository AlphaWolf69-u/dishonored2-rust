//!691550's actual body filter lifetime and691970's obstruction/catch host.
use super::*;
use crate::gameplay::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    entity_lifetime::{EntityId, LiveEntities},
    pickable_component::CheckedComponent,
    player::{
        grapple_catch::{Change, Targets},
        Player,
    },
    world_ray_query::WorldRayRequest,
};
use bevy::prelude::World;

impl PhysicsScene {
    pub(super) fn grapple_dependency(
        &self,
        index: usize,
        kind: usize,
        masks: [u32; 2],
    ) -> Result<Option<ComponentHandle>, String> {
        let Some(range) = self.grapple_dependency_ranges[kind] else {
            return Ok(None);
        };
        self.entities[index]
            .components
            .find_component(range, masks, |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(handle)?.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|d| d.scheduling.start_phases),
                    },
                )
            })
    }
    ///Source mode2 lookup, without substituting focus/usability admission for
    ///the native component reference. Source consumers own their own tests.
    pub(in crate::app) fn grapple_pickable(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<
        Option<(
            CheckedComponent,
            &crate::gameplay::pickable_component::PickableComponent,
        )>,
        String,
    > {
        if live.get(id).is_none() {
            return Ok(None);
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        let handle = self.entities[index].components.find_component(
            [self.classes[6]; 2],
            [0x28, 0],
            |h| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(h)?.flags,
                        start_phases: self
                            .declarations
                            .get(&h.packed())
                            .map(|d| d.scheduling.start_phases),
                    },
                )
            },
        )?;
        let Some(handle) = handle else {
            return Ok(None);
        };
        let component = self
            .pickables
            .get(handle.slot)
            .ok_or("Grapple Pickable payload absent")?;
        Ok(Some((
            CheckedComponent {
                handle,
                generation: component.owner.generation,
            },
            component,
        )))
    }
    pub(in crate::app) fn grapple_has_interactable(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<bool, String> {
        if live.get(id).is_none() {
            return Ok(false);
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(false);
        };
        Ok(self.grapple_dependency(index, 0, [0x28, 0])?.is_some())
    }
    ///Host capability/resource admission precedes47. This does not replace
    ///sensor27's source predicates or pretend an unimplemented family exists.
    pub(in crate::app) fn grapple_pickup_available(
        &self,
        live: &LiveEntities,
        player: &Player,
        id: EntityId,
    ) -> Result<bool, String> {
        let Some((_, pickable)) = self.grapple_pickable(live, id)? else {
            return Ok(false);
        };
        if pickable.interaction_flags & 0xa != 0 {
            return Ok(false);
        }
        if !pickable.definition.item.is_empty() && pickable.item.is_none() {
            return Ok(false);
        }
        if let Some(item) = &pickable.item {
            if item.note_type != -1 {
                return Ok(item.readable.is_some());
            }
            if pickable.definition.hold.is_none()
                && !super::super::pickups::inventory_family_available(item, player)
            {
                return Ok(false);
            }
        }
        if let Some(props) = &pickable.definition.hold {
            return Ok(player.loot_hold_available(props.both_hands));
        }
        Ok(true)
    }
    ///691550 calls PhysicsBase118(false)/110 only for the actual source gate.
    pub(in crate::app) fn grapple_catch_filter(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        restore: bool,
    ) -> Result<(), String> {
        if live.get(id).is_none() {
            return Ok(());
        } //6dc560 null.
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("GrappleCatch entity header absent")?;
        if self.grapple_dependency(index, 1, [0x20, 0])?.is_some() {
            return Err(
                "GrappleCatch admitted Navigation requires its actual5f42f0/5fb490 request owner"
                    .into(),
            );
        }
        if let Some((_, pickable)) = self.grapple_pickable(live, id)? {
            if (pickable.owner.flags >> 4 & 3) != 2 || pickable.interaction_flags & 1 != 0 {
                return Ok(());
            }
        }
        let Some(handle) = self.document_rigid(index)? else {
            return Ok(());
        };
        self.publish_catch_filter(index, handle, restore)
    }
    fn publish_catch_filter(
        &mut self,
        index: usize,
        handle: ComponentHandle,
        restore: bool,
    ) -> Result<(), String> {
        let body = self
            .rigid_component(handle)?
            .body()
            .ok_or("GrappleCatch body absent")?;
        let authored = if restore {
            Some(self.rigid_component(handle)?.hold_restore_filter()?)
        } else {
            None
        };
        let mut registry = self
            .registry
            .take()
            .ok_or("GrappleCatch registry borrowed")?;
        let result = (|| {
            let shared = self.world.clone();
            let mut guard = shared
                .write()
                .map_err(|_| "GrappleCatch physics world poisoned")?;
            let world = &mut *guard;
            let filter = if let Some(filter) = authored {
                filter
            } else {
                world
                    .storage
                    .bodies
                    .get(body)
                    .ok_or("GrappleCatch body generation stale")?
                    .filter_info
                    & 0x0820_001f
            };
            scene_publication_events!(self, &mut registry, publication);
            world
                .storage
                .set_body_filter(&mut world.geometry, &mut publication, body, filter)?;
            if !restore {
                self.entity_flags_d4[index] |= 4;
            }
            Ok(())
        })();
        self.registry = Some(registry);
        result
    }
    ///76ec20(false), before native49/50 binds the actual same Pickable.
    pub(in crate::app) fn finish_loot_without_hide(
        &mut self,
        live: &LiveEntities,
        reference: CheckedComponent,
    ) -> Result<(), String> {
        let (index, _) = self.document_pickable(live, reference)?;
        let component = self
            .pickables
            .get_mut(reference.handle.slot)
            .ok_or("held Loot Pickable absent")?;
        if component.interaction_flags & 4 != 0 {
            self.registry
                .as_mut()
                .ok_or("held Loot registry borrowed")?
                .pause_update_flags(reference.handle, &mut component.owner.flags)?;
        }
        component.interaction_flags &= !2;
        self.entity_flags_d4[index] |= 4;
        Ok(())
    }
    pub(in crate::app) fn finish_held_loot(
        &mut self,
        live: &LiveEntities,
        reference: CheckedComponent,
    ) -> Result<(), String> {
        let (index, _) = self.document_pickable(live, reference)?;
        let shadow = self
            .pickables
            .get(reference.handle.slot)
            .ok_or("held Pickable absent")?
            .definition
            .shadow_grapple;
        self.entities[index].entity_flags =
            (self.entities[index].entity_flags & !(1 << 15)) | (u32::from(shadow) << 15);
        self.finish_loot_without_hide(live, reference)
    }
    ///83fe70 leaves a nonreadable holding target in the world.76ec80 restores
    ///its temporary filter without relocating it to a Hold/Drop camera pose.
    pub(in crate::app) fn cancel_held_loot(
        &mut self,
        live: &LiveEntities,
        reference: CheckedComponent,
    ) -> Result<(), String> {
        let (index, _) = self.document_pickable(live, reference)?;
        let classes = self.types.classes();
        let cloth = classes
            .hierarchy()
            .registration_order()
            .iter()
            .copied()
            .find(|&class| {
                classes
                    .metadata(class)
                    .and_then(|m| classes.name_text(m.name))
                    == Some(b"arkCpntCloth".as_slice())
            });
        let cloth = if let Some(class) = cloth {
            let range = classes.hierarchy().type_range(class)?;
            self.entities[index]
                .components
                .find_component(range, [0x28, 0], |h| {
                    Ok(
                        crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                            flags: self.owner(h)?.flags,
                            start_phases: self
                                .declarations
                                .get(&h.packed())
                                .map(|d| d.scheduling.start_phases),
                        },
                    )
                })?
        } else {
            None
        };
        if cloth.is_some() {
            return Err("Loot47 cancel needs actualCloth70b320 callback".into());
        }
        let component = self
            .pickables
            .get(reference.handle.slot)
            .ok_or("cancelled Pickable absent")?;
        if component.interaction_flags & 1 != 0 {
            if let Some(physics) = component.references.as_ref().and_then(|r| r.physics_base) {
                if self.owner(physics.handle).is_ok_and(|o| {
                    o.generation == physics.generation
                        && crate::gameplay::component_state::matches_reference_state(
                            o.flags,
                            [0x28, 0],
                        )
                }) {
                    self.publish_catch_filter(index, physics.handle, true)?;
                    self.pickables
                        .get_mut(reference.handle.slot)
                        .unwrap()
                        .interaction_flags &= !1;
                }
            }
        }
        self.entities[index].entity_flags_d7 |= 1;
        Ok(())
    }
    fn catch_occluded(
        &self,
        live: &LiveEntities,
        ray: &mut crate::gameplay::world_ray_query::WorldRayQuery,
        id: EntityId,
        origin: bevy_math::Vec3,
        layers: u32,
    ) -> Result<bool, String> {
        let target = self
            .entity_frame(live, id)?
            .ok_or("GrappleCatch ray target frame absent")?;
        let guard = self
            .world
            .read()
            .map_err(|_| "GrappleCatch ray world poisoned")?;
        let hit = guard.ray_closest_hit(
            ray,
            &WorldRayRequest {
                origin: origin.to_array(),
                direction: (bevy_math::Vec3::from_array(target.position) - origin).to_array(),
                maximum_fraction: 1.,
                hit: ShapeHitInfo {
                    body_id: None,
                    material_id: u16::MAX,
                    shape_key: u32::MAX,
                    filter_info: 0,
                    user_data: 0,
                },
                filter: Some(CollisionQueryFilter {
                    layers,
                    required_flags: 0x40,
                }),
            },
        )?;
        hit.map(|h| {
            self.collision_entity(live, &h.target)
                .map(|entity| entity != Some(id))
        })
        .unwrap_or(Ok(false))
    }
}


fn drain(
    world: &mut World,
    carry: &mut super::carrying_host::CarryPhysics,
    scene: &mut PhysicsScene,
    live: &LiveEntities,
    player: &mut Player,
    mode: usize,
) -> Result<(), String> {
    while let Some(change) = player.grapple_catch_runtime(mode).front() {
        match change {
            Change::Mask(id) => scene.grapple_catch_filter(live, id, false)?,
            Change::NotifyGrappled(id) => {
                if live.get(id).is_some() {
                    super::super::entity_graphs::grappled(
                        world,
                        live,
                        id,
                        &mut carry.catch_notices[mode],
                    )?;
                }
                carry.catch_notices[mode] = Default::default();
            }
            Change::Restore(id) => scene.grapple_catch_filter(live, id, true)?,
            Change::ReleaseBeam => player.stop_grapple_beam(),
        }
        player.grapple_catch_runtime(mode).acknowledge(change)?;
    }
    Ok(())
}
fn sample(
    world: &mut World,
    scene: &mut PhysicsScene,
    live: &LiveEntities,
    player: &mut Player,
    carry: &mut super::carrying_host::CarryPhysics,
    mode: usize,
) -> Result<(), String> {
    drain(world, carry, scene, live, player, mode)?;
    let Some(mut tick) = player.grapple_catch_runtime(mode).tick else {
        return Ok(());
    };
    let declaration = player
        .grapple_catch_declaration(mode)
        .ok_or("GrappleCatch active declaration absent")?
        .clone();
    let live_target = |id: Option<EntityId>| id.filter(|&id| live.get(id).is_some());
    let targets = Targets {
        grappled: live_target(tick.targets.grappled),
        movable: live_target(tick.targets.movable),
        ragdoll: live_target(tick.targets.ragdoll),
    };
    if tick.phase == 0 {
        let blocked = if let Some(id) = targets.grappled {
            scene.catch_occluded(
                live,
                &mut carry.catch_ray,
                id,
                tick.origin,
                declaration.occlusion_layers,
            )?
        } else {
            false
        };
        player.grapple_catch_runtime(mode).blocked = blocked;
        tick.phase = 1;
        player.grapple_catch_runtime(mode).tick = Some(tick);
    }
    if tick.phase == 1 {
        let run = player.grapple_catch_runtime(mode);
        if run.target.is_some() {
            if run.remaining <= 0. {
                run.release();
            }
            run.remaining -= tick.dt;
        }
        tick.phase = 2;
        player.grapple_catch_runtime(mode).tick = Some(tick);
    }
    drain(world, carry, scene, live, player, mode)?;
    if tick.phase == 2 {
        if targets.ragdoll.is_some() {
            return Err("GrappleCatch ragdoll requires its actual interaction45/NPC owner".into());
        }
        if let Some(id) = targets.movable {
            let run = player.grapple_catch_runtime(mode);
            if !run.blocked && run.action_pressed {
                let frame = scene
                    .entity_frame(live, id)?
                    .ok_or("GrappleCatch movable frame absent")?;
                let bounds = super::super::runtime_entities::bounds_for(world, live, id)?
                    .unwrap_or([frame.position; 2]);
                let cosine = crate::gameplay::player::states::cos_snap(declaration.maximum_angle);
                if player.grapple_catch_runtime(mode).accepts(
                    bounds,
                    tick.origin,
                    tick.forward,
                    declaration.movable_distance,
                    cosine,
                ) {
                    player.grapple_catch_runtime(mode).release();
                    drain(world, carry, scene, live, player, mode)?;
                    super::super::pickups::request(world, scene, live, player, id)?;
                    player.quit_grapple_catch();
                }
            }
        }
        tick.phase = 3;
        player.grapple_catch_runtime(mode).tick = Some(tick);
    }
    if let Some(id) = targets.grappled {
        let frame = scene
            .entity_frame(live, id)?
            .ok_or("GrappleCatch beam target frame absent")?;
        player.retarget_grapple_beam(bevy_math::Vec3::from_array(frame.position))?;
    }
    if targets.grappled.is_none()
        && targets.movable.is_none()
        && targets.ragdoll.is_none()
        && player.grapple_catch_runtime(mode).target.is_none()
    {
        player.quit_grapple_catch();
    }
    player.grapple_catch_runtime(mode).tick = None;
    Ok(())
}

///Called inside the existing CarryPhysics scope; no additional Scene resource
///move, physics owner or per-frame scratch allocation is introduced.
pub(super) fn advance_borrowed(
    world: &mut World,
    scene: &mut PhysicsScene,
    live: &LiveEntities,
    player: &mut Player,
    carry: &mut super::carrying_host::CarryPhysics,
    closing: bool,
) -> Result<(), String> {
    if !player.grapple_catch_has_work() {
        return Ok(());
    }
    if closing {
        player.finish_grapple_catch();
    }
    for mode in 0..2 {
        if closing {
            drain(world, carry, scene, live, player, mode)?;
        } else {
            sample(world, scene, live, player, carry, mode)?;
        }
    }
    Ok(())
}
