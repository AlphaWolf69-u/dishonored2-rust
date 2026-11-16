//! Concrete DamageZone lifecycle and actual FX/query consumers.
use super::*;
use crate::gameplay::{
    damage::Parameters,
    damage_zone::{self, Target},
    entity_lifetime::{EntityId, LiveEntities},
};
use bevy::prelude::World;
#[derive(Clone)]
pub(super) struct FxState {
    key: Arc<str>,
    ticket: super::super::usable_effects::EffectTicket,
    stopping: bool,
}
fn random(world: &mut World, min: f32, max: f32) -> Result<f32, String> {
    let mut state = world
        .get_resource_mut::<super::super::player::PlayerState>()
        .ok_or("DamageZone shared GameLocal RNG owner absent")?;
    let player = state
        .player
        .as_mut()
        .ok_or("DamageZone shared GameLocal RNG player absent")?;
    Ok(player.inventory_random() * (max - min) + min)
}
impl PhysicsScene {
    pub(in crate::app) fn damage_zone_callbacks_pending(&self, gather: bool, marker: i32) -> bool {
        if self.retirement.is_some() {
            return false;
        }
        if gather && self.occluded_queries_pending() {
            return true;
        }
        self.damage_zones.roster.indices[..self.damage_zones.roster.sorted_start as usize]
            .iter()
            .filter_map(|&slot| self.damage_zones.get(slot))
            .any(|zone| {
                zone.started
                    && if gather {
                        zone.gather_marker != Some(marker) && zone.active() && zone.interval <= 0.
                    } else {
                        zone.update_marker != Some(marker)
                    }
            })
    }

    fn zone_key(&self, handle: ComponentHandle) -> Result<(u64, u32), String> {
        if handle.class_id != self.classes[18] {
            return Err("DamageZone class differs".into());
        }
        Ok((
            handle.packed(),
            self.damage_zones
                .get(handle.slot)
                .ok_or("DamageZone payload absent")?
                .owner
                .generation,
        ))
    }
    pub(in crate::app) fn initialize_damage_zone(
        &mut self,
        handle: ComponentHandle,
        live: &LiveEntities,
    ) -> Result<(), String> {
        self.zone_key(handle)?;
        let zone = self
            .damage_zones
            .get_mut(handle.slot)
            .ok_or("DamageZone payload absent")?;
        if zone.initialized {
            return Ok(());
        }
        //39ce10 begins with common Initialize7e7400. The shared registry
        //tests these actual owner bits after the callback, not our sensor flag.
        zone.owner.mark_initialized();
        let index = zone.owner.entity.ok_or("DamageZone entity absent")?;
        let id = self
            .entities
            .get(index)
            .ok_or("DamageZone header absent")?
            .shared;
        if live.get(id).is_none() {
            return Err("DamageZone entity generation stale".into());
        }
        //39ce10 +64 is sensor ignored managed ID, NOT a damage attacker.
        let ignored = if zone.definition.can_damage_spawner {
            None
        } else {
            live.owner(id)?.filter(|&owner| live.get(owner).is_some())
        };
        zone.sensor.initialize(
            zone.definition.radius,
            zone.definition.layers,
            zone.definition.occlusion_layers,
            false,
            ignored,
        )?;
        zone.initialized = true;
        Ok(())
    }
    pub(in crate::app) fn start_damage_zone(
        &mut self,
        world: &mut World,
        live: &LiveEntities,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.zone_key(handle)?;
        let zone = self
            .damage_zones
            .get(handle.slot)
            .ok_or("DamageZone payload absent")?;
        if zone.started {
            return Ok(());
        }
        if !zone.initialized {
            return Err("DamageZone Start precedes real Initialize".into());
        }
        let index = zone.owner.entity.ok_or("DamageZone entity absent")?;
        let id = self.entities[index].shared;
        if live.get(id).is_none() {
            return Err("DamageZone Start entity generation stale".into());
        }
        if let Some(path) = zone.definition.fx.clone() {
            if self.damage_zone_effects.contains_key(&key) {
                return Err("DamageZone restarts before prior FX retirement".into());
            }
            if !world.contains_resource::<super::super::usable_effects::EffectRequests>() {
                return Err("DamageZone real FX request owner absent".into());
            }
            //39ce80 draws one GameLocal angle in[0,2pi) and10e6a0 axis2.
            let angle = random(world, 0., f32::from_bits(0x40c90fdb))?;
            let (sines, cosines) = crate::gameplay::shape_transform::rotation::sin_cos([angle; 4]);
            let (sin, cos) = (sines[0], cosines[0]);
            let frame = crate::gameplay::reference_frame::ReferenceFrame {
                position: self.entities[index].frame.pose.position,
                basis: [[cos, sin, 0.], [-sin, cos, 0.], [0., 0., 1.]],
            };
            let scope: Arc<str> = Arc::from(format!("damage-zone:{:016x}:{}", key.0, key.1));
            let ticket = world
                .resource_mut::<super::super::usable_effects::EffectRequests>()
                .start_world_once(scope.clone(), &path, frame);
            self.damage_zone_effects.insert(
                key,
                FxState {
                    key: scope,
                    ticket,
                    stopping: false,
                },
            );
        }
        self.damage_zones
            .get_mut(handle.slot)
            .ok_or("DamageZone Start payload vanished")?
            .started = true;
        self.entity_flags_d4[index] |= 4;
        Ok(())
    }
    pub(in crate::app) fn stop_damage_zone(
        &mut self,
        world: Option<&mut World>,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.zone_key(handle)?;
        if let Some(state) = self.damage_zone_effects.get_mut(&key) {
            if !state.stopping {
                let world = world.ok_or("live DamageZone FX Stop requires its published owner")?;
                world
                    .get_resource_mut::<super::super::usable_effects::EffectRequests>()
                    .ok_or("DamageZone FX Stop request owner absent")?
                    .stop_entity(state.key.clone());
                state.stopping = true;
            }
        }
        self.damage_zones
            .get_mut(handle.slot)
            .ok_or("DamageZone Stop payload absent")?
            .started = false;
        Ok(())
    }
    pub(in crate::app) fn damage_zone_stop_complete(
        &mut self,
        world: Option<&World>,
        handle: ComponentHandle,
    ) -> Result<bool, String> {
        let key = self.zone_key(handle)?;
        if self
            .damage_zones
            .get(handle.slot)
            .ok_or("DamageZone Stop payload absent")?
            .started
        {
            return Ok(false);
        }
        if let Some(state) = self.damage_zone_effects.get(&key) {
            if !state.stopping {
                return Ok(false);
            }
            let world =
                world.ok_or("live DamageZone FX retirement requires its published owner")?;
            if !super::super::usable_effects::entity_retired(world, &state.ticket) {
                return Ok(false);
            }
        }
        self.damage_zone_effects.remove(&key);
        Ok(true)
    }
    pub(in crate::app) fn deinitialize_damage_zone(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.zone_key(handle)?;
        if self.damage_zone_effects.contains_key(&key) {
            return Err("DamageZone sensor deinit precedes FX acknowledgement".into());
        }
        let mut sensor = std::mem::take(
            &mut self
                .damage_zones
                .get_mut(handle.slot)
                .ok_or("DamageZone sensor payload absent")?
                .sensor,
        );
        //Transport the same unique Sensor across the Scene host borrow; restore
        //on every outcome. No copied tickets, second shape or substitute owner.
        let result = self.retire_occluded_sensor_queries(&mut sensor);
        let zone = self
            .damage_zones
            .get_mut(handle.slot)
            .ok_or("DamageZone sensor payload vanished")?;
        zone.sensor = sensor;
        result?;
        zone.initialized = false;
        Ok(())
    }
    pub(in crate::app) fn update_damage_zones(
        &mut self,
        world: &mut World,
        live: &LiveEntities,
        marker: i32,
    ) -> Result<(), String> {
        for position in 0..self.damage_zones.roster.sorted_start as usize {
            let handle = ComponentHandle {
                class_id: self.classes[18],
                slot: self.damage_zones.roster.indices[position],
            };
            let key = self.zone_key(handle)?;
            let zone = self
                .damage_zones
                .get(handle.slot)
                .ok_or("DamageZone update payload absent")?;
            if !zone.started || zone.update_marker == Some(marker) {
                continue;
            }
            //39d6e1's166830 is a returning diagnostic stub. Assembly sets
            //BPL=1, still advances/submits normally, then posts ordinary Remove
            //at39d7ca/39d7cf. It neither panics nor skips the accepted frame.
            let fx_missing = self.damage_zone_effects.get(&key).is_some_and(|state| {
                !state.stopping && super::super::usable_effects::effect_completed(&state.ticket)
            });
            let index = zone.owner.entity.ok_or("DamageZone update entity absent")?;
            let id = self.entities[index].shared;
            let delta = world
                .resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                .entity_delta(id)?;
            let zone = self
                .damage_zones
                .get_mut(handle.slot)
                .ok_or("DamageZone update payload vanished")?;
            zone.advance(delta);
            zone.update_marker = Some(marker);
            if zone.expired() {
                //39d680 posts ordinary reflected Remove; the manager owns FIFO.
                super::super::entity_functions::enqueue_removal(world, id, 0.)?;
            } else if zone.active() && zone.interval <= 0. {
                zone.sensor
                    .set_origin(self.entities[index].frame.pose.position);
                let mut sensor = std::mem::take(&mut zone.sensor);
                let result = self.submit_occluded_sensor(live, &mut sensor);
                self.damage_zones
                    .get_mut(handle.slot)
                    .ok_or("DamageZone sensor payload vanished")?
                    .sensor = sensor;
                result?;
            }
            if fx_missing
                && !self
                    .damage_zones
                    .get(handle.slot)
                    .ok_or("DamageZone removal payload vanished")?
                    .expired()
            {
                super::super::entity_functions::enqueue_removal(world, id, 0.)?;
            }
        }
        Ok(())
    }
    pub(in crate::app) fn gather_damage_zones(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut crate::gameplay::entity_events::EntityEvents,
        marker: i32,
    ) -> Result<(), String> {
        self.execute_occluded_queries()?;
        for position in 0..self.damage_zones.roster.sorted_start as usize {
            let handle = ComponentHandle {
                class_id: self.classes[18],
                slot: self.damage_zones.roster.indices[position],
            };
            let zone = self
                .damage_zones
                .get_mut(handle.slot)
                .ok_or("DamageZone gather payload absent")?;
            if !zone.started
                || zone.gather_marker == Some(marker)
                || !zone.active()
                || zone.interval > 0.
            {
                continue;
            }
            let mut sensor = std::mem::take(&mut zone.sensor);
            let result = self.gather_occluded_sensor(live, &mut sensor);
            self.damage_zones
                .get_mut(handle.slot)
                .ok_or("DamageZone gather payload vanished")?
                .sensor = sensor;
            result?;
            let sensor = std::mem::take(
                &mut self
                    .damage_zones
                    .get_mut(handle.slot)
                    .ok_or("DamageZone result payload absent")?
                    .sensor,
            );
            let selected_result = (|| {
                if sensor.results().iter().any(|target| target.actor.is_some()) {
                    return Err(
                        "DamageZone swarm target requires actual SwarmManager weak-agent owner"
                            .into(),
                    );
                }
                let player = self.damage_player.map(|player| player.id);
                let targets = sensor
                    .results()
                    .iter()
                    .filter_map(|target| target.entity.filter(|&id| live.get(id).is_some()))
                    .map(|id| Target {
                        key: id,
                        entity: Some(id),
                        is_player: Some(id) == player,
                        eligible: true,
                    });
                self.damage_zones
                    .get_mut(handle.slot)
                    .ok_or("DamageZone selection payload absent")?
                    .select(targets, |min, max| random(world, min, max))
            })();
            self.damage_zones
                .get_mut(handle.slot)
                .ok_or("DamageZone result payload vanished")?
                .sensor = sensor;
            let selected = selected_result?;
            if let Some(selection) = selected {
                let target = selection.key;
                let zone = self
                    .damage_zones
                    .get(handle.slot)
                    .ok_or("DamageZone selected payload absent")?;
                let player = self.damage_player.is_some_and(|owner| owner.id == target);
                let declaration = if player {
                    zone.definition.player_damage.as_ref()
                } else {
                    zone.definition.damage.as_ref()
                };
                if let Some(declaration) = declaration {
                    let mut params = Parameters::from_declaration(declaration);
                    params.hit_region = 1;
                    params.affected_regions |= 2;
                    params.hit_location = Some(bevy_math::Vec3::from_array(
                        self.published_entity_frame(live, target)?.pose.position,
                    ));
                    //39cc30 targets reflected Damage with amount/region setup,
                    //without inventing an attacker or an inflictor.
                    if player {
                        let mut state = world
                            .get_resource_mut::<super::super::player::PlayerState>()
                            .ok_or("DamageZone Player damage owner absent")?;
                        let player = state
                            .player
                            .as_mut()
                            .ok_or("DamageZone Player runtime absent")?;
                        if player.managed_entity() != Some(target) {
                            return Err("DamageZone Player damage target differs".into());
                        }
                        player.receive_damage(&params);
                        super::super::player_damage::flush_events(player, live, events, target,self)?;
                    } else {
                        self.damage_entity(live, events, target, &params)?;
                    }
                }
                self.damage_zones
                    .get_mut(handle.slot)
                    .ok_or("DamageZone interval payload absent")?
                    .finish_selection(selection.available_count, |min, max| {
                        random(world, min, max)
                    })?;
            }
            self.damage_zones
                .get_mut(handle.slot)
                .ok_or("DamageZone gather payload vanished")?
                .gather_marker = Some(marker);
        }
        Ok(())
    }
}
