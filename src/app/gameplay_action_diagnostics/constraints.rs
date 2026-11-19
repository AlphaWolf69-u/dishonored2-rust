//! Opt-in observation of real passive EotW joints; no synthetic forces or state.
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
        let mut found = [false; 2];
        let mut prepared = [false; 2];
        scene.visit_constraints(live,|row|{
            let endpoints=row.definition.endpoints();
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
        if !found.iter().all(|&value| value) {
            return Err(format!(
                "actual EotW lamp constraint fixtures unavailable: {found:?}"
            ));
        }
        if complete {
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
