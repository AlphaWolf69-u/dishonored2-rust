//! Live entity binding and Throwable lifecycle adapters,875e80/8762b0.
use super::*;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    reference_frame::{ReferenceFrame, relative_binding},
};
#[derive(Clone, Copy)]
pub(in crate::app) struct BindingPose {
    pub joint: i16,
    pub offset: [f32; 3],
    pub orientation: [f32; 4],
    pub parent_placement: Option<usize>,
    ///752900's explicit offset/orientation world write, already resolved by
    ///the caller's actual parent/joint owner. None preserves the world pose.
    pub initial_pose: Option<ReferenceFrame>,
}
/// Armed only by the normal-input carry diagnostic. The receipt survives a
/// same-frame body removal and retains neither the object nor its resources.
#[derive(Debug)]
pub(in crate::app) struct ThrowObservation {
    pub target: EntityId,
    pub impact: Option<(EntityId, [f32; 3], f32)>,
    pub shattered: bool,
}

impl PhysicsScene {
    ///4c9560 drains each actual Throwable's inline contact queue in LIFO
    ///order. A failed host publication retains both the contact and its stage.
    pub(in crate::app) fn advance_throwable_contacts(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        clock: &crate::gameplay::game_clock_runtime::ClockRuntime,
        player: Option<&crate::gameplay::player::Player>,
    ) -> Result<(), String> {
        use bevy_math::{Mat3, Vec3};
        //The scheduled source prefix owns its callbacks; this does not scan
        //render placements or construct actors for unadmitted declarations.
        let count = usize::try_from(self.throwables.roster.sorted_start)
            .map_err(|_| "negative Throwable update prefix")?;
        for at in 0..count {
            let slot = self.throwables.roster.indices[at];
            let component = self
                .throwables
                .get(slot)
                .ok_or("scheduled Throwable absent")?;
            if component.owner.flags & 0xf != 0xa {
                continue;
            }
            let index = component
                .owner
                .entity
                .ok_or("scheduled Throwable entity absent")?;
            let id = self.entities[index].shared;
            let Some(first) = component.throwable()?.pending_contact() else {
                continue;
            };
            let handle = first.bodies[0].component;
            if handle.class_id != self.classes[12] || handle.slot != slot {
                return Err("scheduled Throwable contact owner differs".into());
            }
            while let Some(contact) = self.rigid_component(handle)?.throwable()?.pending_contact() {
                let state = self.rigid_component(handle)?.throwable()?;
                if state.contact_delivery() == 0 {
                    if contact.damage <= 0.
                        && state.source.settings.sticky
                        && self.entities[index].entity_flags & 0x80 == 0
                        && state.binding.is_none()
                        && !self.throwable_class_is(
                            self.entities[*self
                                .shared_indices
                                .get(&contact.bodies[1].entity)
                                .ok_or("sticky target actor absent")?]
                            .class,
                            b"arkPlayer",
                        )?
                    {
                        let body = {
                            let guard = self
                                .world
                                .read()
                                .map_err(|_| "sticky contact world poisoned")?;
                            let body = guard
                                .storage
                                .bodies
                                .get(contact.bodies[1].body)
                                .ok_or("sticky contact body retired")?;
                            ReferenceFrame {
                                position: body.transform[3][..3].try_into().unwrap(),
                                basis: std::array::from_fn(|i| {
                                    body.transform[i][..3].try_into().unwrap()
                                }),
                            }
                        };
                        //81b460 reprojects the captured contact through the
                        //other body's current transform before4c9140 offsets it.
                        let old = Mat3::from_cols_array_2d(&contact.other_frame.basis);
                        if old.determinant() == 0. {
                            return Err("sticky captured body basis singular".into());
                        }
                        let current = Mat3::from_cols_array_2d(&body.basis);
                        let basis = current * old.inverse();
                        let position = Vec3::from_array(body.position)
                            + basis
                                * (Vec3::from_array(contact.position)
                                    - Vec3::from_array(contact.other_frame.position));
                        let normal = basis * Vec3::from_array(contact.normal);
                        let mut frame = self.entities[index].frame.pose;
                        frame.position =
                            (position + normal * state.source.settings.stick_distance).to_array();
                        self.stick_throwable(
                            live,
                            id,
                            frame,
                            contact.bodies[1].body,
                            u32::MAX,
                            events,
                        )?;
                    } else if contact.damage > 0. {
                        self.deliver_rigid_impact_damage(
                            live, events, clock, player, handle, contact.bodies[1].entity,
                            contact.position, contact.normal, contact.damage, false,
                        )?;
                    }
                    self.rigid_component_mut(handle)?
                        .throwable_mut()?
                        .delivered_contact_stage(1);
                }
                if self
                    .rigid_component(handle)?
                    .throwable()?
                    .contact_delivery()
                    == 1
                {
                    if contact.damage > 0. {
                        self.deliver_rigid_impact_damage(
                            live, events, clock, player, handle, contact.bodies[1].entity,
                            contact.position, contact.normal, contact.damage, true,
                        )?;
                    }
                    self.rigid_component_mut(handle)?
                        .throwable_mut()?
                        .delivered_contact_stage(2);
                }
                if self
                    .rigid_component(handle)?
                    .throwable()?
                    .contact_delivery()
                    == 2
                {
                    self.notify_throwable_first_hit(handle, id, events)?;
                    self.rigid_component_mut(handle)?
                        .throwable_mut()?
                        .delivered_contact_stage(3);
                }
                if self
                    .rigid_component(handle)?
                    .throwable()?
                    .contact_delivery()
                    == 3
                {
                    let other = *self
                        .shared_indices
                        .get(&contact.bodies[1].entity)
                        .ok_or("contact target scene owner absent")?;
                    if !self.rigid_component(handle)?.throwable()?.npc_hit()
                        && self.throwable_class_is(self.entities[other].class, b"arkNPC")?
                        && (self.entities[other].entity_flags >> 8) & 0x28 == 0
                    {
                        events.post(Event {
                            id: 0x132,
                            target: Some(id),
                            payload: Payload::None,
                        });
                        self.rigid_component_mut(handle)?
                            .throwable_mut()?
                            .mark_npc_hit();
                    }
                    self.rigid_component_mut(handle)?
                        .throwable_mut()?
                        .finish_contact()?;
                }
            }
        }
        Ok(())
    }
    /// 8527e0 -> 4c7b30 -> 8098e0. A collision against an ordinary wall
    /// can damage the moving prop even when the wall has no Breakable.
    pub(super) fn deliver_solved_rigid_impact(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        clock: &crate::gameplay::game_clock_runtime::ClockRuntime,
        player: Option<&crate::gameplay::player::Player>,
        contact: &crate::gameplay::physics_world::contacts::SolvedContact,
        side: usize,
        stage: &mut u8,
    ) -> Result<(), String> {
        use crate::gameplay::collision_user_data::CollisionUserData;
        let guard = self
            .world
            .read()
            .map_err(|_| "contact impulse world poisoned")?;
        let storage = &guard.storage;
        let own = storage
            .bodies
            .get(contact.bodies[side])
            .ok_or("contact impulse body absent")?;
        if own.flags & 0x80 == 0 {
            return Ok(());
        }
        let handle = CollisionUserData::decode(own.user_data)?.component;
        let throwable = handle.class_id == self.classes[12];
        if !throwable && handle.class_id != self.classes[8] {
            return Ok(());
        }
        let component = self.rigid_component(handle)?;
        let source = component
            .definition()
            .ok_or("rigid impulse declaration absent")?;
        if !throwable && source.damage.is_none() {
            return Ok(());
        }
        let dropped = throwable && component.throwable()?.dropped();
        let index = component
            .owner
            .entity
            .ok_or("contact impulse owner absent")?;
        let entity = &self.entities[index];
        let id = entity.shared;
        let origin = entity.frame.pose.position;
        let amount = if source.damage.is_some() {
            let motion = storage
                .motions
                .get(own.motion_id)
                .ok_or("contact impulse motion absent")?;
            rigid_impact_amount(
                &source.settings,
                dropped,
                entity.frame.velocities[0],
                motion.linear_velocity[..3].try_into().unwrap(),
                motion,
            )?
        } else {
            None
        };
        let packet = if let Some(amount) = amount {
            let other = storage
                .bodies
                .get(contact.bodies[1 - side])
                .ok_or("impact other body absent")?;
            let other_handle = CollisionUserData::decode(other.user_data)?.component;
            let other_index = self
                .owner(other_handle)?
                .entity
                .ok_or("impact other actor absent")?;
            let normal = contact.normal.map(|v| if side == 0 { v } else { -v });
            // 81aae0 averages points on the recipient's surface within 0.005
            // of the nearest contact. Geometry stores each point on body B.
            let points = &contact.points[..usize::from(contact.count)];
            let nearest = points
                .iter()
                .map(|p| p.distance)
                .fold(f32::INFINITY, f32::min);
            let mut position = [0.; 3];
            let mut selected = 0.;
            for p in points.iter().filter(|p| p.distance <= nearest + 0.005) {
                for (i, total) in position.iter_mut().enumerate() {
                    *total += if side == 0 {
                        p.position[i] + contact.normal[i] * p.distance
                    } else {
                        p.position[i]
                    };
                }
                selected += 1.;
            }
            if selected == 0. {
                return Err("contact impulse has no finite contact point".into());
            }
            let inverse = crate::gameplay::constraint_solver::reciprocal(selected);
            Some((
                self.entities[other_index].shared,
                position.map(|v| v * inverse),
                normal,
                amount,
            ))
        } else {
            None
        };
        drop(guard);
        if let Some((other, position, normal, amount)) = packet {
            if *stage == 0 {
                self.deliver_rigid_impact_damage(
                    live, events, clock, player, handle, other, position, normal, amount, false,
                )?;
                *stage = 1;
            }
            if *stage == 1 {
                self.deliver_rigid_impact_damage(
                    live, events, clock, player, handle, other, position, normal, amount, true,
                )?;
                *stage = 2;
            }
        }
        if !throwable {
            return Ok(());
        }
        if let Some(player) = player {
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .reset_dropped_distance(origin, player.physics.position().to_array());
        }
        self.notify_throwable_first_hit(handle, id, events)
    }

    pub(super) fn throwable_update_membership(
        &mut self, handle: ComponentHandle, scheduled: bool,
    ) -> Result<(), String> {
        if handle.class_id == self.classes[12] {
            self.rigid_component_mut(handle)?.throwable_mut()?.update_membership_changed(scheduled);
        }
        Ok(())
    }

    fn notify_throwable_first_hit(
        &mut self,
        handle: ComponentHandle,
        id: EntityId,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let state = self.rigid_component(handle)?.throwable()?;
        if !state.first_hit() {
            let profile = state.damping_profile;
            let body = self
                .rigid_component(handle)?
                .body()
                .ok_or("contact damping body absent")?;
            if let Some(profile) = profile {
                let dynamic = {
                    let world = self
                        .world
                        .read()
                        .map_err(|_| "contact damping world poisoned")?;
                    world
                        .storage
                        .bodies
                        .get(body)
                        .ok_or("contact damping body stale")?
                        .flags
                        & 5
                        == 0
                };
                if dynamic {
                    self.body_commands
                        .enqueue_motion_properties(body, profile, false)?;
                }
            }
            events.post(Event {
                id: 0x131,
                target: Some(id),
                payload: Payload::None,
            });
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .mark_first_hit();
        }
        Ok(())
    }

    /// 807170, shared by predictive breakthrough and ordinary solved impacts.
    fn deliver_rigid_impact_damage(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        clock: &crate::gameplay::game_clock_runtime::ClockRuntime,
        player: Option<&crate::gameplay::player::Player>,
        handle: ComponentHandle,
        other: EntityId,
        position: [f32; 3],
        normal: [f32; 3],
        amount: f32,
        self_damage: bool,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let source = component
            .definition()
            .ok_or("rigid damage declaration absent")?;
        if self_damage && source.declaration_flags & 0x40 == 0 {
            return Ok(());
        }
        let Some(declaration) = source.damage.clone() else {
            return Ok(());
        };
        let index = component
            .owner
            .entity
            .ok_or("throwable damage owner absent")?;
        let id = self.entities[index].shared;
        let (target, inflictor) = if self_damage {
            (id, other)
        } else {
            (other, id)
        };
        let other_index = *self
            .shared_indices
            .get(&inflictor)
            .ok_or("impact inflictor absent")?;
        let velocity = bevy_math::Vec3::from_array(self.entities[other_index].frame.velocities[0]);
        let mut params = crate::gameplay::damage::Parameters::from_declaration(&declaration);
        params.hit_location = Some(bevy_math::Vec3::from_array(position));
        params.hit_normal = Some(bevy_math::Vec3::from_array(normal));
        params.hit_direction = velocity / velocity.length_squared().max(f32::MIN_POSITIVE).sqrt();
        params.hit_region = 0;
        params.affected_regions = 1;
        params.inflictor = Some(inflictor);
        let delay = crate::platform::console::shared()
            .read()
            .cvars
            .float("ark_physics_impulser_ignore_delay")
            .ok_or("physics instigator delay absent")?;
        let age =
            (clock.entity_time(id).map_err(str::to_owned)? - component.instigator_time) as f32;
        if age < delay {
            params.attacker = component.instigator.filter(|id| live.get(*id).is_some());
        }
        let greater = player
            .filter(|p| p.managed_entity() == params.attacker)
            .filter(|p| p.carrying_greater_throw());
        let amount = amount * greater.map_or(1., |p| p.carrying_throw_damage_multiplier());
        params.set_amount(&declaration, amount.floor(), greater.is_none());
        if self_damage {
            //807480's fallback changes attribution after calculating the damage.
            if params.attacker.is_none() {
                let body = component.body().ok_or("self-damage body absent")?;
                let world = self
                    .world
                    .read()
                    .map_err(|_| "self-damage world poisoned")?;
                if world
                    .storage
                    .bodies
                    .get(body)
                    .ok_or("self-damage body stale")?
                    .flags
                    & 0x80000
                    != 0
                {
                    params.attacker = player.and_then(|p| p.managed_entity());
                }
            }
        } else {
            let marker = clock.clock().map_err(str::to_owned)?.frame_marker();
            let history = self
                .rigid_component_mut(handle)?
                .damage_targets
                .get_or_insert_with(|| Box::new((marker, Vec::new())));
            if history.0 != marker {
                history.0 = marker;
                history.1.clear();
            }
            if history.1.contains(&target) {
                return Ok(());
            }
            history
                .1
                .try_reserve(1)
                .map_err(|e| format!("rigid damage targets: {e}"))?;
        }
        let result = self.damage_entity(live, events, target, &params)?;
        if let (Some(trace), Some(damage)) = (
            self.throw_observation.as_mut().filter(|t| self_damage && t.target == id),
            result.filter(|d| d.health_damage > 0.),
        ) {
            if live.parent(id)?.is_none() {
                trace.impact = Some((other, position, damage.health_damage));
            }
        }
        if !self_damage {
            self.rigid_component_mut(handle)?
                .damage_targets
                .as_mut()
                .ok_or("impact target history absent")?
                .1
                .push(target);
        }
        Ok(())
    }

    pub(super) fn throwable_class_is(&self, mut class: usize, name: &[u8]) -> Result<bool, String> {
        loop {
            let classes = self.types.classes();
            let metadata = classes
                .metadata(class)
                .ok_or("Throwable contact class metadata absent")?;
            if classes.name_text(metadata.name) == Some(name) {
                return Ok(true);
            }
            let Some(base) = classes.hierarchy().node(class).and_then(|node| node.base) else {
                return Ok(false);
            };
            class = base;
        }
    }

    ///4c7c50/81b370/81b2e0/81aec0. The caller supplies the actual manifold
    ///event kind and mutable cache/Jacobian, not guessed gameplay hit flags.
    pub(in crate::app) fn capture_throwable_contact(
        &mut self,
        world: &WorldStorage,
        live: &LiveEntities,
        contact: &crate::gameplay::physics_world::contacts::SolvedContact,
        cache: Option<&mut crate::gameplay::physics_world::contacts::ContactCallbackState>,
        rows: Option<&mut crate::gameplay::constraint_solver::contact::ContactRows>,
        event_kind: u8,
        side: usize,
    ) -> Result<(), String> {
        use crate::gameplay::{
            collision_user_data::CollisionUserData,
            throwable::{Contact, ContactBody},
        };
        if side >= 2 {
            return Err("Throwable contact recipient side invalid".into());
        }
        //cf3460's event8 has a null cache/Jacobian pointer. Retained
        //numeric Rust rows do not make the native callback input present.
        if event_kind == 8 {
            return Ok(());
        }
        let cache = cache.ok_or("Throwable contact cache absent outside event8")?;
        let own = world
            .bodies
            .get(contact.bodies[side])
            .ok_or("Throwable contact body stale")?;
        let handle = CollisionUserData::decode(own.user_data)?.component;
        if handle.class_id != self.classes[12] || own.flags & 0x40 == 0 {
            return Ok(());
        }
        if event_kind == 1 {
            cache.contact_flags = (cache.contact_flags & !0x80) | 0x60;
        }
        //81b420 reads the selected actual stream header's type byte.
        if !cache.stream_live {
            return Ok(());
        }
        let Some(rows) = rows else {
            return Ok(());
        };
        if rows.stream_type != 0 {
            return Ok(());
        }
        let own_component = self.rigid_component(handle)?;
        let index = own_component
            .owner
            .entity
            .ok_or("Throwable contact entity absent")?;
        let other = world
            .bodies
            .get(contact.bodies[1 - side])
            .ok_or("Throwable other body stale")?;
        let other_handle = CollisionUserData::decode(other.user_data)?.component;
        let other_index = self
            .owner(other_handle)?
            .entity
            .ok_or("Throwable other entity absent")?;
        let id = self.entities[index].shared;
        let other_id = self.entities[other_index].shared;
        if live.get(id).is_none() || live.get(other_id).is_none() {
            return Err("Throwable contact managed generation stale".into());
        }
        if self.throwable_class_is(self.entities[index].class, b"arkProjectile")?
            && self.throwable_class_is(self.entities[other_index].class, b"arkProjectile")?
        {
            for &packed in self.entities[other_index].components.entries.values() {
                let other_projectile = ComponentHandle::from_packed(packed)?;
                if other_projectile.class_id != self.classes[13] {
                    continue;
                }
                let runtime = self
                    .projectiles
                    .get(other_projectile.slot)
                    .and_then(|c| c.runtime.as_ref())
                    .ok_or("contact projectile runtime absent")?;
                if !runtime.impact_stopped {
                    if cache.modifier_flags & 0x200000 == 0 {
                        rows.stream_flags |= 8;
                        rows.stream_type = 3;
                    }
                    return Ok(());
                }
            }
        }
        let count = usize::from(contact.count);
        if count == 0 || count > 4 {
            return Err("Throwable contact manifold count invalid".into());
        }
        let minimum = contact.points[..count]
            .iter()
            .map(|p| p.distance)
            .fold(f32::INFINITY, f32::min);
        let cutoff = minimum + f32::from_bits(0x3ba3d70a);
        let motion = world
            .motions
            .get(own.motion_id)
            .ok_or("Throwable contact motion absent")?;
        let angular = motion.world_angular_velocity();
        let normal = contact.normal.map(|v| if side == 0 { v } else { -v });
        let mut position = [0.; 3];
        let mut velocity = [0.; 3];
        let mut selected = 0.;
        for point in &contact.points[..count] {
            if point.distance > cutoff {
                continue;
            }
            let p = std::array::from_fn::<_, 3, _>(|i| {
                point.position[i]
                    + if side == 0 {
                        0.
                    } else {
                        contact.normal[i] * point.distance
                    }
            });
            let r = std::array::from_fn::<_, 3, _>(|i| p[i] - motion.pose.center[i]);
            let v = [
                (angular[1] * r[2] - angular[2] * r[1]) + motion.linear_velocity[0],
                (angular[2] * r[0] - angular[0] * r[2]) + motion.linear_velocity[1],
                (angular[0] * r[1] - angular[1] * r[0]) + motion.linear_velocity[2],
            ];
            for i in 0..3 {
                position[i] += p[i];
                velocity[i] += v[i];
            }
            selected += 1.;
        }
        if selected == 0. {
            return Err("Throwable manifold has no finite nearest point".into());
        }
        let inverse = crate::gameplay::constraint_solver::reciprocal(selected);
        for i in 0..3 {
            position[i] *= inverse;
            velocity[i] *= inverse;
        }
        let origin = self.entities[index].frame.pose.position;
        let difference = std::array::from_fn::<_, 3, _>(|i| position[i] - origin[i]);
        let distance_squared = (difference[1] * difference[1] + difference[0] * difference[0])
            + difference[2] * difference[2];
        let other_frame = ReferenceFrame {
            position: other.transform[3][..3].try_into().unwrap(),
            basis: std::array::from_fn(|i| other.transform[i][..3].try_into().unwrap()),
        };
        let bodies = [
            ContactBody {
                body: own.id,
                component: handle,
                entity: id,
                material: contact.material_ids[side],
            },
            ContactBody {
                body: other.id,
                component: other_handle,
                entity: other_id,
                material: contact.material_ids[1 - side],
            },
        ];
        let state = own_component.throwable()?;
        if own_component.owner.flags & 0x30 >= 0x20 {
            return Ok(());
        }
        let mut damage = 0.;
        if state.flags() & 1 != 0
            && !self.throwable_class_is(other_handle.class_id as usize, b"arkCpntPhysicsWorld")?
            && !self.throwable_class_is(self.entities[other_index].class, b"arkDoor")?
        {
            let settings = &state.source.rigid.settings;
            let speed = self.entities[index].frame.velocities[0];
            if let Some(amount) = rigid_impact_amount(settings, state.dropped(), speed, [0.; 3], motion)? {
                if self.breakable_would_break_amount(live, other_id, amount, true)? {
                    if cache.modifier_flags & 0x200000 == 0 {
                        rows.stream_flags |= 8;
                        rows.stream_type = 3;
                    }
                    damage = amount;
                }
            }
        }
        let queued = damage > 0.
            || (state.source.settings.sticky && self.entities[index].entity_flags & 0x80 == 0)
            || (state.source.settings.notify_hit
                && (state.flags() & 0x10 == 0 || state.flags() & 0x20 == 0));
        if queued {
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .push_contact(Contact {
                    bodies,
                    position,
                    normal,
                    velocity,
                    distance_squared,
                    other_frame,
                    damage,
                })?;
        }
        Ok(())
    }
    /// 4c7020/4c6f90 use the real player's proxy pair manager. The player
    /// reference actor deliberately has no fabricated WorldStorage body.
    pub(in crate::app) fn mark_carried_release(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        player: &crate::gameplay::player::Player,
        thrown: bool,
        _events: &mut EntityEvents,
    ) -> Result<(), String> {
        let player_id = player
            .managed_entity()
            .ok_or("released object player identity absent")?;
        if live.get(player_id).is_none() {
            return Err("released object player generation stale".into());
        }
        let handle = self.throwable_handle(live, id)?;
        let component = self.rigid_component(handle)?;
        let body = component.body().ok_or("released Throwable body absent")?;
        let filter = component
            .throwable()?
            .source
            .rigid
            .settings
            .collision_filter()
            .ok_or("released Throwable collision filter invalid")?;
        let player_filter = crate::gameplay::collision_filter::filter_info(
            crate::gameplay::collision_filter::layer::PLAYER,
        );
        if crate::gameplay::collision_filter::pair_collides(filter, player_filter) {
            player.physics.release_pairs.mark_released(body)?;
        }
        let state = self.rigid_component_mut(handle)?.throwable_mut()?;
        if thrown {
            state.set_instigator(player_id);
        } else {
            state.mark_dropped(player_id);
        }
        Ok(())
    }

    /// 628df0/629590 publish through the same actual actor and body caches
    /// as all other rigid entities; queued 90200 remains the velocity owner.
    pub(in crate::app) fn publish_projectile_motion(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        frame: ReferenceFrame,
        linear: [f32; 3],
    ) -> Result<(), String> {
        if live.get(id).is_none() {
            return Err("finite projectile managed generation stale".into());
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("finite projectile entity absent")?;
        let handle = self.entities[index]
            .components
            .reference_actor
            .ok_or("finite projectile actor absent")?;
        let actor = self
            .actors
            .actors
            .get_mut(handle.slot)
            .ok_or("finite projectile actor retired")?;
        actor.set_transform(frame, false);
        actor.frame.published_velocities = [linear, [0.; 3]];
        let generation = actor.owner.generation;
        self.entities[index].frame.pose = frame;
        self.entities[index].frame.velocities = [linear, [0.; 3]];
        self.request_post_physics_actor(usize::from(handle.slot), generation)?;
        Ok(())
    }

    ///87d1e0/751d00 physical, entity and actor publication share one pose.
    ///Authored model/body offsets stay in the existing ModelPose world delta.
    pub(super) fn publish_entity_pose_in_registry(&mut self,index:usize,frame:ReferenceFrame,registry:&mut ComponentRegistry)->Result<(),String>{
        if let Some(handle)=self.document_rigid(index)?{
            if let Some(body)=self.rigid_component(handle)?.body(){
                let shared=self.world.clone();
                let mut guard=shared.write().map_err(|_|"entity pose world poisoned")?;
                let world=&mut *guard;
                let matrix=[
                    [frame.basis[0][0],frame.basis[0][1],frame.basis[0][2],0.],
                    [frame.basis[1][0],frame.basis[1][1],frame.basis[1][2],0.],
                    [frame.basis[2][0],frame.basis[2][1],frame.basis[2][2],0.],
                    [frame.position[0],frame.position[1],frame.position[2],1.],
                ];
                scene_publication_events!(self,registry,publication);
                world.storage.set_body_transform(&mut world.geometry,&mut publication,body,matrix,0)?;
            }
        }
        self.entities[index].frame.pose=frame;
        if let Some(handle)=self.entities[index].components.reference_actor{
            let actor=self.actors.actors.get_mut(handle.slot).ok_or("entity pose reference actor retired")?;
            actor.set_transform(frame,false);
            let generation=actor.owner.generation;
            self.request_post_physics_actor(usize::from(handle.slot),generation)?;
        }
        Ok(())
    }
    pub(in crate::app) fn publish_rigid_motion(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        frame: ReferenceFrame,
        linear: [f32; 3],
        angular: [f32; 3],
    ) -> Result<(), String> {
        if live.get(id).is_none() {
            return Err("rigid motion managed generation stale".into());
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("rigid motion entity absent")?;
        let handle = self
            .document_rigid(index)?
            .ok_or("rigid motion actual component absent")?;
        let body = self
            .rigid_component(handle)?
            .body()
            .ok_or("projectile body absent")?;
        let mut registry = self.registry.take().ok_or("projectile registry absent")?;
        let result = self.publish_entity_pose_in_registry(index,frame,&mut registry);
        self.registry = Some(registry);
        result?;
        self.queue_rigid_velocity(
            body,
            [linear[0], linear[1], linear[2], 0.],
            [angular[0], angular[1], angular[2], 0.],
        )?;
        Ok(())
    }
    pub(in crate::app) fn bind_entity(
        &mut self,
        live: &mut LiveEntities,
        child: EntityId,
        parent: EntityId,
        pose: BindingPose,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let mut registry = self
            .registry
            .take()
            .ok_or("binding component registry is absent")?;
        let result = self.bind_entity_in_registry(live, child, parent, pose, events, &mut registry);
        self.registry = Some(registry);
        result
    }
    pub(super) fn bind_entity_in_registry(
        &mut self,
        live: &mut LiveEntities,
        child: EntityId,
        parent: EntityId,
        pose: BindingPose,
        events: &mut EntityEvents,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let child = *self
            .shared_indices
            .get(&child)
            .ok_or("bound child has no actual scene owner")?;
        let parent = *self
            .shared_indices
            .get(&parent)
            .ok_or("bound parent has no actual scene owner")?;
        if live.get(self.entities[child].shared).is_none()
            || live.get(self.entities[parent].shared).is_none()
        {
            return Err("bound managed identity is stale".into());
        }
        if let Some(binding) = self
            .document_hierarchy
            .bindings
            .iter()
            .find(|binding| binding.child == child)
        {
            let actor = self.entities[child]
                .components
                .reference_actor
                .and_then(|h| self.actors.actors.get(h.slot))
                .ok_or("bound actor is absent")?;
            if binding.parent == parent
                && actor.binding.joint == pose.joint
                && actor.binding.offset == pose.offset
                && actor.binding.orientation == pose.orientation
            {
                if let Some(frame)=pose.initial_pose{self.publish_entity_pose_in_registry(child,frame,registry)?;}
                return self.refresh_bound_body_mode(live, child, registry);
            }
            //752900 updates the local binding in place when parent identity
            //is unchanged. A new empty tag may replace a previous joint tag.
            if binding.parent == parent {
                if let Some(frame)=pose.initial_pose{self.publish_entity_pose_in_registry(child,frame,registry)?;}
                let handle = self.entities[child]
                    .components
                    .reference_actor
                    .ok_or("bound actor absent")?;
                let actor = self
                    .actors
                    .actors
                    .get_mut(handle.slot)
                    .ok_or("bound actor absent")?;
                actor.binding.joint = pose.joint;
                actor.binding.offset = pose.offset;
                actor.binding.orientation = pose.orientation;
                let generation = actor.owner.generation;
                let binding = self
                    .document_hierarchy
                    .bindings
                    .iter_mut()
                    .find(|binding| binding.child == child)
                    .unwrap();
                binding.joint = pose.joint;
                binding.parent_placement = pose.parent_placement;
                if self.actors.actors[usize::from(handle.slot)].flags & 0x20 != 0 {
                    self.request_pre_physics_actor(usize::from(handle.slot))?;
                } else {
                    self.request_post_physics_actor(usize::from(handle.slot), generation)?;
                }
                return self.refresh_bound_body_mode(live, child, registry);
            }
            //752900 changes parents in this same bind operation:879100
            //removes the old reciprocal child link, then the normal path
            //below links/publishes the new parent. Keep the actual child
            //world frame/body;8762b0's temporary unbound body-mode restore
            //does not occur during a direct reparent in the source.
            live.preflight_reparent(self.entities[parent].shared, self.entities[child].shared)?;
            self.detach_document(live, child)?;
        }
        //752900 writes explicit values through87d1e0 after an old parent is
        //detached and BEFORE the new link/body-mode change. A pickup rotation
        //must not be synthesized as keyframe velocity then inherited by Drop.
        if let Some(frame)=pose.initial_pose{
            self.publish_entity_pose_in_registry(child,frame,registry)?;
        }
        let result: Result<(), String> = (|| {
            let child_handle = self.reference_actor(child, registry)?;
            let parent_handle = self.reference_actor(parent, registry)?;
            let ca = child_handle.slot as usize;
            let pa = parent_handle.slot as usize;
            let cg = self.actors.actors[ca].owner.generation;
            let pg = self.actors.actors[pa].owner.generation;
            self.actors.actors[pa]
                .children
                .try_reserve(1)
                .map_err(|e| format!("actor child binding: {e}"))?;
            self.document_hierarchy
                .bindings
                .try_reserve(1)
                .map_err(|e| format!("entity binding roster: {e}"))?;
            let initial = self.entities[child].frame.pose;
            let parent_frame = self.entities[parent].frame.pose;
            live.link_child(self.entities[parent].shared, self.entities[child].shared)?;
            //878fd0 inherits the actual parent's2e0 through875fc0. Ordinary
            //post-physics poses never substitute a position-based area query.
            let area=live.ai_world_index(self.entities[parent].shared)?;
            live.change_ai_world_index(self.entities[child].shared,area,events)?;
            self.actors.actors[pa].children.push(ca);
            let actor = &mut self.actors.actors[ca];
            actor.parent = Some(pa);
            actor.binding.entity = Some(parent);
            actor.binding.joint = pose.joint;
            actor.binding.offset = pose.offset;
            actor.binding.orientation = pose.orientation;
            actor.flags |= 0x10;
            self.entities[child].entity_flags |= 0x80;
            self.document_hierarchy.record_runtime_binding(
                child,
                parent,
                pose.parent_placement,
                pose.joint,
                (child_handle, cg),
                (parent_handle, pg),
                initial,
                parent_frame,
            );
            self.actors.propagate_body_flag(pa, true)?;
            events.post(Event {
                id: 0x125,
                target: Some(self.entities[child].shared),
                payload: Payload::Entity(self.entities[parent].shared),
            });
            //751c80 chooses752260 for a real alternate-body hierarchy;
            //POST-only admission updates its mesh but never drives the bound
            //keyframed body through7545a0's desired-velocity calculation.
            if self.actors.actors[ca].flags & 0x20 != 0 {
                self.request_pre_physics_actor(ca)?;
            } else {
                self.request_post_physics_actor(ca, cg)?;
            }
            Ok(())
        })();
        result?;
        self.refresh_bound_body_mode(live, child, registry)
    }

    ///727a00/80af20 refresh the same body's movement type after topology
    ///changes. No new actor/body is created for a carry or sticky attachment.
    fn refresh_bound_body_mode(
        &mut self,
        live: &LiveEntities,
        index: usize,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let Some(handle) = self.document_rigid(index)? else {
            return Ok(());
        };
        let component = self.rigid_component(handle)?;
        if component.owner.flags & 3 < 2 {
            return Ok(());
        }
        let body = component.body().ok_or("bound rigid body absent")?;
        let authored = component
            .definition()
            .and_then(|definition| definition.settings.physics_behavior.as_deref())
            .and_then(entities::rigid_body_behavior_kind)
            .ok_or("bound rigid declared behavior absent")?;
        let behavior = self.bound_document_behavior(live, index, authored)?;
        let result = (|| {
            let shared = self.world.clone();
            let mut guard = shared
                .write()
                .map_err(|_| "bound body mode world poisoned")?;
            let world = &mut *guard;
            scene_publication_events!(self, registry, events);
            match behavior {
                0 => world
                    .storage
                    .static_single_body(&mut world.geometry, &mut events, body),
                1 => world
                    .storage
                    .keyframe_single_body(&mut world.geometry, &mut events, body),
                2 => world
                    .storage
                    .dynamic_single_body(&mut world.geometry, &mut events, body),
                _ => Err("bound body mode invalid".into()),
            }
        })();
        result
    }
    pub(in crate::app) fn bind_to_player(
        &mut self,
        live: &mut LiveEntities,
        child: EntityId,
        player: &crate::gameplay::player::Player,
        tag: &str,
        offset: [f32; 3],
        orientation: [f32; 4],
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let parent = player
            .managed_entity()
            .ok_or("held object's real player identity is absent")?;
        let (joint, tag_offset, tag_orientation) = player.reference_tag_binding(tag)?;
        let index = *self
            .shared_indices
            .get(&parent)
            .ok_or("held object's real player reference actor is absent")?;
        if joint >= 0 {
            self.document_hierarchy
                .external_joints
                .insert((index, joint), player.reference_local_joint(joint)?);
        }
        //752900 adds explicit offset to a real model tag and multiplies its
        //rotation (110ad0). Bare bones/empty tags expose zero/identity here.
        let offset = std::array::from_fn(|i| offset[i] + tag_offset[i]);
        let orientation = crate::gameplay::reference_frame::multiply_binding_quaternion(
            tag_orientation,
            orientation,
        );
        self.bind_entity(
            live,
            child,
            parent,
            BindingPose {
                joint,
                offset,
                orientation,
                parent_placement: None,
                initial_pose: Some(crate::gameplay::reference_frame::bind(
                    if joint>=0 {crate::gameplay::reference_frame::compose(self.entities[index].frame.pose,player.reference_local_joint(joint)?)}else{self.entities[index].frame.pose},
                    offset,orientation)),
            },
            events,
        )
    }
    pub(in crate::app) fn unbind_entity(
        &mut self,
        live: &mut LiveEntities,
        child: EntityId,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&child)
            .ok_or("unbound child has no actual scene owner")?;
        if live.parent(child)?.is_none() {
            return Ok(());
        }
        //8762d0/727450 restores the actual body mode before topology disappears.
        let mut registry = self.registry.take().ok_or("unbind registry is absent")?;
        let restore = (|| {
            self.entities[index].entity_flags &= !0x80;
            if let Some(handle) = self.document_rigid(index)? {
                if self.owner(handle)?.flags & 3 > 1 {
                    let body = self
                        .rigid_component(handle)?
                        .body()
                        .ok_or("bound body absent")?;
                    let shared = self.world.clone();
                    let mut guard = shared.write().map_err(|_| "unbind world poisoned")?;
                    let world = &mut *guard;
                    scene_publication_events!(self, &mut registry, publication);
                    // 8762d0 clears d0 bit7 before 727450(NULL) calls
                    // vtable+188 (727880) with movement type 2. This is the
                    // release request even when the spawn declaration was static.
                    world.storage.dynamic_single_body(&mut world.geometry, &mut publication, body)?;
                }
            }
            self.detach_document(live, index)
        })();
        self.registry = Some(registry);
        restore?;
        events.post(Event {
            id: 0x125,
            target: Some(child),
            payload: Payload::None,
        });
        Ok(())
    }
}

impl PhysicsScene {
    pub(in crate::app) fn throwable_handle(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<ComponentHandle, String> {
        if live.get(id).is_none() {
            return Err("Throwable entity generation is stale".into());
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Throwable entity has no actual scene owner")?;
        for &packed in self.entities[index].components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            if handle.class_id == self.classes[12] {
                let component = self.rigid_component(handle)?;
                if component.owner.entity != Some(index) || !component.throwable()?.initialized {
                    return Err("Throwable actual component is not initialized".into());
                }
                return Ok(handle);
            }
        }
        Err("entity has no actual Throwable component".into())
    }
    /// The base808ea0 must finish first, including its real disabled body.
    pub(super) fn initialize_throwable_component(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let index = component
            .owner
            .entity
            .ok_or("Throwable component is unlinked")?;
        let timespace = self.entities[index].timespace as usize;
        //4c7930 checks actual Breakable or WhaleOilTank component identity.
        let receiver = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .any(|&packed| {
                ComponentHandle::from_packed(packed).is_ok_and(|h| h.class_id == self.classes[11]||h.class_id==self.classes[60])
            })
            //Host loading stages the real tank/pusher Init behind shared
            //prototype readiness. Its typed declaration is already admitted
            //here; preserve4c7930's predicate before that later Init drain.
            ||self.prepared_whale_oil_tanks.iter().any(|plan|plan.actor==self.entities[index].shared);
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "Throwable world owner is poisoned")?;
        let world = &mut *guard;
        super::super::throwable::initialize(
            self.rigid_component_mut(handle)?,
            &mut world.storage,
            &mut world.geometry,
            timespace,
            receiver,
        )
    }
    ///4c7fc0 runs after809d80. Fresh bound instances apply their actual
    /// attached filter before enabling quality/contact events and125.
    pub(super) fn start_throwable_component(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut EntityEvents,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let index = component
            .owner
            .entity
            .ok_or("Throwable component is unlinked")?;
        let id = self.entities[index].shared;
        let restoring = component.owner.flags & 0x4000_0000 != 0;
        let enabled = !restoring || component.throwable()?.flags() & 2 != 0;
        let attached = !restoring && self.entities[index].entity_flags & 0x80 != 0;
        let filter = if attached {
            let state = component.throwable()?;
            let mut settings = state.source.rigid.settings.clone();
            settings.collision = Some(state.source.settings.attached_collision);
            Some(
                settings
                    .collision_filter()
                    .ok_or("Throwable attached filter is invalid")?,
            )
        } else {
            None
        };
        let body = component
            .body()
            .ok_or("Throwable Start has no actual body")?;
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "Throwable world owner is poisoned")?;
        let world = &mut *guard;
        if let Some(filter) = filter {
            scene_publication_events!(self, registry, publication, &mut *entity_events);
            world
                .storage
                .set_body_filter(&mut world.geometry, &mut publication, body, filter)?;
        }
        {
            let quality = self.rigid_component(handle)?.throwable()?.quality(enabled);
            scene_publication_events!(self, registry, publication, &mut *entity_events);
            if let Some(quality) = quality {
                world.storage.set_body_quality(
                    &mut world.geometry,
                    &mut publication,
                    body,
                    quality,
                    0,
                )?;
            }
        }
        self.rigid_component_mut(handle)?
            .throwable_mut()?
            .quality_applied(enabled);
        let flags = world
            .storage
            .bodies
            .get(body)
            .ok_or("Throwable body retired during Start")?
            .flags;
        let mode = if flags & 1 != 0 {
            0
        } else if flags & 4 != 0 {
            1
        } else {
            2
        };
        let contact = self
            .rigid_component(handle)?
            .throwable()?
            .contact_events(mode);
        {
            scene_publication_events!(self, registry, publication, &mut *entity_events);
            world.storage.set_body_user_flags(
                &mut world.geometry,
                &mut publication,
                body,
                0x40,
                contact,
                0,
            )?;
        }
        drop(guard);
        let owner = self.owner_id;
        super::super::throwable::start_listener(
            self.rigid_component_mut(handle)?,
            entity_events,
            owner,
            handle,
            id,
        )
    }
    pub(super) fn stop_throwable_component(
        &mut self,
        handle: ComponentHandle,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        super::super::throwable::stop_listener(self.rigid_component_mut(handle)?, events)
    }
    pub(super) fn throwable_binding_changed(
        &mut self,
        handle: ComponentHandle,
        bound: bool,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let state = component.throwable()?;
        if !state.stuck() && !state.collision_update_pending() {
            return Ok(());
        }
        let mut settings = state.source.rigid.settings.clone();
        if bound {
            settings.collision = Some(state.source.settings.attached_collision);
        }
        let filter = settings
            .collision_filter()
            .ok_or("Throwable binding filter is invalid")?;
        let body = component
            .body()
            .ok_or("Throwable binding has no actual body")?;
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "Throwable world owner is poisoned")?;
        let world = &mut *guard;
        {
            scene_publication_events!(self, registry, publication);
            world
                .storage
                .set_body_filter(&mut world.geometry, &mut publication, body, filter)?;
        }
        if !bound {
            //4c7570 resolves the same Pickable's checked Loot before its
            //detachment callback settles; otherwise the old owner keeps the
            //projectile interaction disabled and can later remove it.
            let index=self.rigid_component(handle)?.owner.entity
                .ok_or("detached Throwable owner is absent")?;
            self.detach_stuck_loot_member(index)?;
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .detached();
        }
        Ok(())
    }
    pub(in crate::app) fn start_throwable(
        &mut self,
        live: &mut LiveEntities,
        id: EntityId,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let handle = self.throwable_handle(live, id)?;
        let mut registry = self
            .registry
            .take()
            .ok_or("Throwable Start registry is absent")?;
        let result = (|| {
            // Vtable141b64b68+38 is0998b0, the source queued-start no-op.
            registry.request_start(
                &mut self.rigid_component_mut(handle)?.owner,
                handle.packed() as usize,
                false,
                |_, _| Ok(()),
            )?;
            registry.initialize_and_start(&mut interaction_components::Startup { world: None,
                scene: self,
                live,
                events,
            })?;
            registry.drain_update_changes(&mut interaction_components::Startup { world: None,
                scene: self,
                live,
                events,
            })
        })();
        self.registry = Some(registry);
        result
    }
    ///4c6b70's non-skeletal branch: the actual collision payload identifies
    ///the target entity. The body/shape witness is retained across callbacks.
    pub(in crate::app) fn stick_throwable(
        &mut self,
        live: &mut LiveEntities,
        id: EntityId,
        frame: ReferenceFrame,
        target_body: u32,
        shape_key: u32,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        use crate::gameplay::throwable::{Binding, PendingStick, StickPhase};
        let handle = self.throwable_handle(live, id)?;
        let index = self
            .rigid_component(handle)?
            .owner
            .entity
            .ok_or("sticking Throwable is unlinked")?;
        let (parent, dynamic_target) = {
            let world = self.world.read().map_err(|_| "stick world poisoned")?;
            let body = world
                .storage
                .bodies
                .get(target_body)
                .ok_or("stick target body generation stale")?;
            let target =
                crate::gameplay::collision_user_data::CollisionUserData::decode(body.user_data)?
                    .component;
            let entity = self
                .owner(target)?
                .entity
                .ok_or("stick target collision payload is unlinked")?;
            (self.entities[entity].shared, body.flags & 1 == 0)
        };
        if live.get(parent).is_none() {
            return Err("stick target entity generation stale".into());
        }
        let request = Binding {
            parent,
            body: target_body,
            shape_key,
            frame,
        };
        let state = self.rigid_component_mut(handle)?.throwable_mut()?;
        if let Some(pending) = state.pending_stick {
            let old = pending.request;
            if old.parent != parent
                || old.body != target_body
                || old.shape_key != shape_key
                || old.frame.position != frame.position
                || old.frame.basis != frame.basis
            {
                return Err("sticking request differs from suspended source request".into());
            }
        } else {
            state.pending_stick = Some(PendingStick {
                request,
                phase: StickPhase::Quality,
                dynamic_target,
            });
        }
        let mut registry = self.registry.take().ok_or("stick registry absent")?;
        let result: Result<(), String> = (|| {
            let phase = self
                .rigid_component(handle)?
                .throwable()?
                .pending_stick
                .unwrap()
                .phase;
            if phase == StickPhase::Quality {
                if dynamic_target {
                    let quality = self.rigid_component(handle)?.throwable()?.quality(false);
                    if let Some(quality) = quality {
                        let body = self
                            .rigid_component(handle)?
                            .body()
                            .ok_or("sticking body absent")?;
                        let shared = self.world.clone();
                        let mut guard = shared.write().map_err(|_| "stick world poisoned")?;
                        let world = &mut *guard;
                        scene_publication_events!(self, &mut registry, publication);
                        world.storage.set_body_quality(
                            &mut world.geometry,
                            &mut publication,
                            body,
                            quality,
                            0,
                        )?;
                    }
                    self.rigid_component_mut(handle)?
                        .throwable_mut()?
                        .quality_applied(false);
                }
                self.rigid_component_mut(handle)?
                    .throwable_mut()?
                    .pending_stick
                    .as_mut()
                    .unwrap()
                    .phase = StickPhase::Frame;
            }
            if self
                .rigid_component(handle)?
                .throwable()?
                .pending_stick
                .unwrap()
                .phase
                == StickPhase::Frame
            {
                let body = self
                    .rigid_component(handle)?
                    .body()
                    .ok_or("sticking body absent")?;
                let matrix = std::array::from_fn(|row| {
                    if row == 3 {
                        [frame.position[0], frame.position[1], frame.position[2], 1.]
                    } else {
                        [
                            frame.basis[row][0],
                            frame.basis[row][1],
                            frame.basis[row][2],
                            0.,
                        ]
                    }
                });
                let shared = self.world.clone();
                let mut guard = shared.write().map_err(|_| "stick world poisoned")?;
                let world = &mut *guard;
                {
                    scene_publication_events!(self, &mut registry, publication);
                    world.storage.set_body_transform(
                        &mut world.geometry,
                        &mut publication,
                        body,
                        matrix,
                        0,
                    )?;
                }
                self.entities[index].frame.pose = frame;
                if let Some(actor) = self.entities[index].components.reference_actor {
                    self.actors.actors[actor.slot as usize].set_transform(frame, false);
                }
                self.rigid_component_mut(handle)?
                    .throwable_mut()?
                    .pending_stick
                    .as_mut()
                    .unwrap()
                    .phase = StickPhase::Bind;
            }
            Ok(())
        })();
        self.registry = Some(registry);
        result?;
        if self
            .rigid_component(handle)?
            .throwable()?
            .pending_stick
            .unwrap()
            .phase
            == StickPhase::Bind
        {
            let parent_index = *self
                .shared_indices
                .get(&parent)
                .ok_or("stick parent scene owner absent")?;
            let (offset, orientation) =
                relative_binding(self.entities[parent_index].frame.pose, frame);
            self.bind_entity(
                live,
                id,
                parent,
                BindingPose {
                    joint: -1,
                    offset,
                    orientation,
                    parent_placement: None,
                    initial_pose: None, //The actual stick frame was published before this preserve-world bind.
                },
                events,
            )?;
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .pending_stick
                .as_mut()
                .unwrap()
                .phase = StickPhase::Notify;
        }
        if self
            .rigid_component(handle)?
            .throwable()?
            .pending_stick
            .unwrap()
            .phase
            == StickPhase::Notify
        {
            events.post(Event {
                id: 0x184,
                target: Some(parent),
                payload: Payload::Entity(id),
            });
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .mark_stuck(request);
        }
        Ok(())
    }
}

/// 807a20 uses the signed change along the incoming velocity. Predictive
/// breakthrough supplies zero outgoing velocity; 8098e0 reads the solved body.
fn rigid_impact_amount(
    settings: &crate::entities::RigidBodySettings,
    dropped: bool,
    incoming: [f32; 3],
    outgoing: [f32; 3],
    motion: &crate::gameplay::motion::Motion,
) -> Result<Option<f32>, String> {
    let dot = |a: [f32; 3], b: [f32; 3]| (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2];
    let speed2 = dot(incoming, incoming);
    let threshold = if dropped {
        settings.velocity_damage_threshold_when_dropped
    } else {
        settings.velocity_damage_threshold
    }
    .ok_or("Throwable velocity damage threshold absent")?;
    if speed2 <= threshold * threshold {
        return Ok(None);
    }
    let speed = speed2.sqrt();
    let axis = incoming.map(|v| v / speed2.max(f32::MIN_POSITIVE).sqrt());
    let delta = dot(axis, outgoing) - speed;
    if delta >= 0. {
        return Ok(None);
    }
    let inverse_mass = f32::from_bits(u32::from(motion.inverse_mass[3]) << 16);
    let mass = settings
        .mass_override
        .filter(|&value| value != 0.)
        .unwrap_or_else(|| {
            if inverse_mass == 0. {
                0.
            } else {
                1. / inverse_mass
            }
        });
    let energy = delta * delta * mass * 0.5;
    let minimum = settings
        .min_contact_impulse_for_damage
        .ok_or("Throwable minimum impulse absent")?;
    if energy <= minimum {
        return Ok(None);
    }
    let maximum = settings
        .max_contact_impulse_for_damage
        .ok_or("Throwable maximum impulse absent")?;
    let maximum_damage = settings
        .contact_impulse_max_damage
        .ok_or("Throwable maximum damage absent")?;
    let factor = if maximum > minimum {
        ((energy - minimum) / (maximum - minimum)).min(1.)
    } else {
        1.
    };
    Ok(Some((factor * maximum_damage).max(1.)))
}
