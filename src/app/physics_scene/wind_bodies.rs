//!64ecd0/64ecf0 registrations and807680 ->77f050 rigid-body wind response.
use super::*;
use crate::gameplay::{
    aerodynamics::{MotionMass, wind_velocity_change},
    game_clock::GameClock,
    wind::{Wind, WindManager},
};
impl PhysicsScene {
    pub(super) fn advance_rigid_wind(
        &mut self,
        clock: &GameClock,
        manager: Option<&WindManager>,
        world_wind: Option<&Wind>,
        cursor: &mut usize,
    ) -> Result<(), String> {
        while let Some(&(handle, generation)) = self.rigid_wind_bodies.get(*cursor) {
            let component = self.checked_rigid_component(handle, generation)?;
            let index = component
                .owner
                .entity
                .ok_or("Wind registered body entity absent")?;
            let entity = self
                .entities
                .get(index)
                .ok_or("Wind registered header absent")?;
            let delta = clock
                .entity_delta(entity.timespace)
                .ok_or("Wind entity timespace invalid")?;
            let change = if delta > 0. {
                let selected = if component.wind_triggers.is_empty() {
                    world_wind
                } else {
                    let manager =
                        manager.ok_or("Wind registered body has no canonical WindManager")?;
                    let mut selected = None;
                    //7b7af0 returns the first started entry. A nonempty local
                    //list deliberately does not fall back to world Wind.
                    for &(wind, source, _) in &component.wind_triggers {
                        let owner = self.owner(wind)?;
                        if (owner.flags >> 2) & 3 >= 2 {
                            let payload = manager
                                .triggers
                                .get(source)
                                .ok_or("body Wind payload absent")?;
                            let name =
                                &self.entities[owner.entity.ok_or("body Wind entity absent")?].name;
                            if !manager
                                .trigger_entities
                                .get(source)
                                .is_some_and(|entry| entry.eq_ignore_ascii_case(name))
                            {
                                return Err("body Wind payload identity differs".into());
                            }
                            selected = Some(payload);
                            break;
                        }
                    }
                    selected
                };
                if let Some(wind) = selected {
                    let guard = self
                        .world
                        .read()
                        .map_err(|_| "Wind physics world poisoned")?;
                    let body = guard
                        .storage
                        .bodies
                        .get(component.body().ok_or("Wind body absent")?)
                        .ok_or("Wind body generation stale")?;
                    //807680 excludes static/keyframed. c87d70's asynchronous
                    //simulation-world branch is disabled in this host (returns1).
                    if body.flags & 5 != 0 {
                        None
                    } else {
                        let motion = guard
                            .storage
                            .motions
                            .get(body.motion_id)
                            .ok_or("Wind body motion absent")?;
                        let shape = guard
                            .geometry
                            .shapes
                            .get(body.shape)
                            .and_then(Option::as_ref)
                            .ok_or("Wind shape lease absent")?;
                        if let Some(property) = shape.physics_property_if_present()? {
                            let sample = wind
                                .velocity(bevy_math::Vec3::from_array([
                                    motion.pose.center[0],
                                    motion.pose.center[1],
                                    motion.pose.center[2],
                                ]))
                                .to_array();
                            let columns = [body.transform[0], body.transform[1], body.transform[2]];
                            let mass = MotionMass {
                                center: motion.pose.center,
                                orientation: motion.pose.orientation,
                                inverse_mass: motion.inverse_mass,
                            };
                            let linear = component.velocities.requested_linear;
                            let angular = component.velocities.requested_angular;
                            let factor = component
                                .definition()
                                .ok_or("Wind body declaration absent")?
                                .settings
                                .wind_speed_factor
                                .ok_or("Wind speed factor absent")?;
                            wind_velocity_change(
                                &columns,
                                &property.aero_faces,
                                &mass,
                                linear,
                                sample,
                                factor,
                                delta,
                                body.flags & 8 != 0,
                            )
                            .map(|(dl, da)| {
                                (
                                    body.id,
                                    std::array::from_fn(|i| linear[i] + dl[i]),
                                    std::array::from_fn(|i| angular[i] + da[i]),
                                )
                            })
                        } else {
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((body, linear, angular)) = change {
                self.queue_rigid_velocity(body, linear, angular)?;
            }
            *cursor += 1;
        }
        Ok(())
    }
}
