//! Opt-in observation of the placed EotW lamp and turbine joints; no synthetic forces or state.
use super::*;
const ENDPOINTS: [&str; 2] = [
    "env_interiors_lamp_hanging_cable_01",
    "env_interiors_lamp_hanging_cable_01_71",
];
pub(super) struct Observation {
    first_step: Option<u32>,
    last_step: Option<u32>,
    observed_steps: u32,
    maximum_pivot_separation_squared: f32,
    minimum_cone_axis_dot: Option<f32>,
    prepared_samples: [bool; 2],
    rotor_active_samples: u32,
    rotor_frozen_samples: u32,
    rotor_spinning_samples: u32,
    rotor_max_hub_distance: f32,
    rotor_previous_render_basis: Option<[[f32; 3]; 3]>,
    rotor_render_changes: u32,
}
impl Observation {
    pub(super) fn begin(_world: &World) -> Result<Self, String> {
        eprintln!("ACTION_SMOKE_CONSTRAINT_BEGIN source=actual_EotW_passive_lamp_constraints observation_steps=120 no_impulse_spawn_pose_clock_or_authored_state_override=true");
        Ok(Self {
            first_step: None,
            last_step: None,
            observed_steps: 0,
            maximum_pivot_separation_squared: 0.,
            minimum_cone_axis_dot: None,
            prepared_samples: [false; 2],
            rotor_active_samples: 0,
            rotor_frozen_samples: 0,
            rotor_spinning_samples: 0,
            rotor_max_hub_distance: 0.,
            rotor_previous_render_basis: None,
            rotor_render_changes: 0,
        })
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        let scene = world.resource::<super::super::physics_scene::PhysicsScene>();
        let live = world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        let owner = scene.point_queries()?;
        let guard = owner
            .read()
            .map_err(|_| "constraint observation world poisoned")?;
        let storage = &guard.storage;
        let step = storage.step_index;
        let first = *self.first_step.get_or_insert(step);
        if self.last_step == Some(step) {
            return Ok(false);
        }
        self.last_step = Some(step);
        self.observed_steps += 1;
        let complete = step.wrapping_sub(first) >= 120;
        let report = self.observed_steps == 1 || complete;
        const ROTOR: &str = "script_constraint_rotor_3_1_1";
        const BLADES: &str = "script_wind_turbine_propeller_01_2_1_1";
        const HEAD: &str = "script_wind_turbine_top_01_1_1_1";
        let machinery = world.resource::<super::super::machinery::AmbientMachinery>();
        let placements = world.resource::<super::super::RenderSourcePlacements>();
        let mut rotor_found = false;
        let mut found = [false; 2];
        let mut prepared = [false; 2];
        scene.visit_constraints(live,|row|{
            let endpoints=row.definition.endpoints();
            if row.name == ROTOR {
                if rotor_found || endpoints != [Some(BLADES), Some(HEAD)] {
                    return Err(format!("captured rotor joint is duplicated or has wrong endpoints: {endpoints:?}"));
                }
                let joint = row.joint.ok_or("captured rotor lost its actual joint")?;
                let actual = storage.observe_joint(joint)?;
                let (sources, levels, frozen, ambient) = machinery.observe_motor(joint).ok_or("captured rotor has no machinery consumer")?;
                if actual.bodies != row.bodies || !actual.linked.iter().all(|&linked| linked)
                    || !row.started || !actual.enabled || ambient != [0; 2] {
                    return Err(format!("captured rotor has conflicting physics owners: {actual:?} ambient={ambient:?}"));
                }
                let blade = storage.bodies.get(row.bodies[0]).ok_or("captured blades body stale")?;
                let rendered = placements.0.get(sources[0]).filter(|p| p.id == BLADES)
                    .ok_or("captured rotor render placement differs from constrained blades")?;
                if placements.0.get(sources[1]).is_none_or(|p| p.id != HEAD) {
                    return Err("captured rotor head render placement differs from constraint".into());
                }
                let name = live.find(BLADES).ok_or("captured blades managed generation absent")?;
                let actor = scene.published_entity_frame(live, name)?.pose;
                let hub = Vec3::from(row.owner_frame.position);
                let center = Vec3::new(blade.transform[3][0], blade.transform[3][1], blade.transform[3][2]);
                // Installed entity script_wind_turbine_propeller_01_2_1_1 spawnPosition.
                // The placement itself is the *live* render publication and cannot be its own baseline.
                let spawn = Vec3::new(-43.8608589172, 119.6921081543, 24.4499740601);
                let spawn_radius = (spawn - hub).length();
                let radius = (center - hub).length();
                let render_radius = (Vec3::from(rendered.position) - hub).length();
                let actor_radius = (Vec3::from(actor.position) - hub).length();
                for (owner, value) in [("body", radius), ("actor", actor_radius), ("render", render_radius)] {
                    if !value.is_finite() || value > spawn_radius + storage.info.collision_tolerance {
                        return Err(format!("captured rotor {owner} drifted from its fixed hub: body={center:?} actor={:?} render={:?} hub={hub:?} radius={value} authored_radius={spawn_radius} collision_tolerance={}", actor.position, rendered.position, storage.info.collision_tolerance));
                    }
                }
                self.rotor_max_hub_distance = self.rotor_max_hub_distance.max(radius);
                let motor = storage.motor_parameters(joint)?.ok_or("captured hinge has no motor")?;
                let angular = actual.angular[0];
                let previous = self.rotor_previous_render_basis.replace(rendered.rotation);
                if frozen { self.rotor_frozen_samples += 1; }
                else if motor.0 && motor.2 != 0. && actual.prepared {
                    self.rotor_active_samples += 1;
                    if previous.is_some_and(|basis| basis != rendered.rotation) { self.rotor_render_changes += 1; }
                    if angular[..3].iter().any(|&value| value != 0.) { self.rotor_spinning_samples += 1; }
                }
                if report {
                    eprintln!("ACTION_CAPTURED_ROTOR step={step} name={ROTOR} ends={endpoints:?} bodies={:?} source_placements={sources:?} source_center={spawn:?} render_center={:?} actual_center={center:?} actor_center={:?} hub={hub:?} offset={:?} authored_radius={spawn_radius} joint_separation_squared={} angular={angular:?} motor={motor:?} levels={levels:?} frozen={frozen} ambient_dynamic_copies={ambient:?}",
                        actual.bodies, rendered.position, actor.position, center-hub,actual.pivot_separation_squared);
                }
                rotor_found = true;
                return Ok(());
            }
            let Some(fixture)=ENDPOINTS.iter().position(|expected|endpoints.iter().flatten().any(|actual|actual==expected))else{return Ok(())};
            if found[fixture]{return Err("passive lamp fixture has multiple admitted constraints; inspect actual source owners".into())}
            found[fixture]=true;
            let Some(id)=row.joint.or(row.retiring_joint)else{
                if report{eprintln!("ACTION_SMOKE_CONSTRAINT_PENDING endpoint={} name={} checked={:?} initialized={} started={} pending_enable={}",ENDPOINTS[fixture],row.name,row.checked,row.initialized,row.started,row.pending_enable);}
                return Ok(())
            };
            let actual=storage.observe_joint(id)?;
            if actual.bodies!=row.bodies{return Err("passive lamp component endpoints differ from actual numeric joint generations".into())}
            if !actual.linked.iter().all(|&linked|linked){return Err("passive lamp numeric joint is absent from actual endpoint adjacency".into())}
            prepared[fixture]=(actual.enabled||actual.factor==0.)&&!actual.transition_pending&&row.started&&!row.pending_enable;
            self.prepared_samples[fixture]|=actual.prepared;
            self.maximum_pivot_separation_squared=self.maximum_pivot_separation_squared.max(actual.pivot_separation_squared);
            if let Some(dot)=actual.cone_axis_dot{
                self.minimum_cone_axis_dot=Some(self.minimum_cone_axis_dot.map_or(dot,|old|old.min(dot)));
            }
            if report{
                eprintln!("ACTION_SMOKE_CONSTRAINT_STATE endpoint={} name={} checked={:?} initialized={} started={} pending_enable={} retiring={:?} owner_frame={:?} authored_linear={} authored_angular={} authored_axis={} actual_numeric={actual:?}",
                    ENDPOINTS[fixture],row.name,row.checked,row.initialized,row.started,row.pending_enable,row.retiring_joint,row.owner_frame,
                    row.definition.settings().linear_constraint,row.definition.settings().angular_constraint,row.definition.settings().angular_axis);
            }
            Ok(())
        })?;
        if !rotor_found { return Err("captured rotor constraint never reached the gameplay world".into()); }
        if !found.iter().all(|&value| value) {
            return Err(format!(
                "actual EotW lamp constraint fixtures unavailable: {found:?}"
            ));
        }
        if complete {
            if self.rotor_active_samples > 0 && self.rotor_spinning_samples == 0 {
                return Err("running rotor produced no angular velocity in active solver samples".into());
            }
            if self.rotor_spinning_samples > 1 && self.rotor_render_changes == 0 {
                return Err("spinning rotor never published a changed render orientation".into());
            }
            eprintln!("ACTION_SMOKE_PASS check=captured_rotor_one_owner_and_hub maximum_center_to_hub={} active_samples={} spinning_samples={} rendered_rotation_changes={} frozen_samples={} steps={}", self.rotor_max_hub_distance, self.rotor_active_samples, self.rotor_spinning_samples, self.rotor_render_changes, self.rotor_frozen_samples, self.observed_steps);
            // Independent of sword selection, steering and attack callbacks.
            // The existing level has advanced 120 real physics steps; this
            // only borrows published state around the submitted window.
            let query_center = [16.222336, 90.63119, 17.512545];
            let mut window = String::new();
            scene.write_static_usable_diagnostics(world, query_center, &mut window);
            eprintln!("ACTION_CAPTURED_WINDOW context=actual_passive_constraint_observation first_physics_step={first} last_physics_step={step} sampled_advanced_steps={} read_only_query_center={query_center:?} no_player_movement_or_state_override=true{window}", self.observed_steps);
            if !prepared.iter().all(|&value| value) {
                return Err(format!("actual lamp constraints did not reach source enabled/frozen ownership: {prepared:?}"));
            }
            eprintln!("ACTION_SMOKE_PASS check=actual_passive_lamp_owners_and_links first_physics_step={first} last_physics_step={step} sampled_advanced_steps={} maximum_pivot_separation_squared={} minimum_cone_axis_dot={:?} actual_joint_slots_live_free={:?} actual_prepared_row_samples={:?} numerical_values_not_arbitrary_tolerance_or_visual_proof=true Stop_retirement_observed_by_existing_ordinary_exit_lifecycle=true",
                self.observed_steps,self.maximum_pivot_separation_squared,self.minimum_cone_axis_dot,storage.constraint_counts(),self.prepared_samples);
            if !self.prepared_samples.iter().all(|&value| value) {
                eprintln!("ACTION_SMOKE_NOT_COVERED check=active_passive_lamp_solver reason=no_prepared_rows_observed_inspect_recorded_actual_body_activity no_synthetic_wake_or_impulse=true");
            }
        }
        Ok(complete)
    }
}
