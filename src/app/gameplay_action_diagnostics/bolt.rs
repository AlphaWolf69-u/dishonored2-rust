//! Actual regular-bolt contact/recovery, including the stopped class13 phase.
//! Physical input selects the existing nearby WORLD support fixture.
use super::super as app;
use super::{input_for, player, send, Input};
use crate::gameplay::{
    component_state::entity::ComponentHandle,
    entity_lifetime::{EntityId, LiveEntities},
    player::{states::StateId, weapons::RangedFamily},
};
use bevy::prelude::*;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

///A single exact producer receipt, allocated only by bolt-bottle opt-in.
#[derive(Resource)]
struct SupportContact {
    target: EntityId,
    before_health: f32,
    handoff: Option<(EntityId, Option<EntityId>)>,
    expected: Option<(EntityId, EntityId, usize)>,
    fired: Option<EntityId>,
    hit: Option<(
        EntityId,
        Option<EntityId>,
        crate::gameplay::projectiles::Response,
        crate::gameplay::shape_ray_query::ShapeRayContact,
    )>,
}
pub(super) fn record_birth(
    world: &mut World,
    id: EntityId,
    attacker: Option<EntityId>,
    equipment: Option<EntityId>,
    definition: usize,
) {
    let Some(mut receipt) = world.get_resource_mut::<SupportContact>() else {
        return;
    };
    let Some((expected_attacker, expected_equipment, expected_definition)) = receipt.expected
    else {
        return;
    };
    if receipt.fired.is_none()
        && attacker == Some(expected_attacker)
        && equipment == Some(expected_equipment)
        && definition == expected_definition
    {
        receipt.fired = Some(id);
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_EXACT_BIRTH fired={id:?} attacker={attacker:?} equipment={equipment:?} declaration_identity={definition} armed_by_actual_fire_input=true");
    }
}
pub(super) fn record_contact(
    world: &mut World,
    step: &app::physics_scene::projectiles::ProjectileStep,
) {
    let Some(witness) = step.witness else {
        return;
    };
    let Some(mut receipt) = world.get_resource_mut::<SupportContact>() else {
        return;
    };
    if receipt.fired == Some(step.entity) && receipt.hit.is_none() {
        receipt.hit = Some((step.entity, step.contact_entity, step.response, witness));
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_CONTACT fired={:?} actual_target={:?} expected_target={:?} response={:?} witness={witness:?}",step.entity,step.contact_entity,receipt.target,step.response);
    }
}

pub(super) fn record_attachment(world: &mut World, id: EntityId) {
    if !world
        .get_resource::<SupportContact>()
        .is_some_and(|receipt| receipt.hit.is_some_and(|hit| hit.0 == id))
    {
        return;
    }
    let parent = world.resource::<LiveEntities>().parent(id);
    match parent {
        Ok(parent) => {
            world.resource_mut::<SupportContact>().handoff = Some((id, parent));
            eprintln!("ACTION_SMOKE_BOLT_BOTTLE_HANDOFF actual_fired={id:?} actual_parent_after_handoff={parent:?}");
        }
        Err(error) => {
            eprintln!("ACTION_SMOKE_BOLT_BOTTLE_HANDOFF actual_fired={id:?} topology_error={error}")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Select,
    Aim,
    Flight,
    Retry,
    BackAway,
    Recover,
    Pickup,
    Retirement,
    Done,
}
pub(super) struct Observation {
    stage: Stage,
    since: Instant,
    release: Option<Input>,
    movement: Option<Input>,
    crouch: Option<Input>,
    projectile: Option<(ComponentHandle, EntityId)>,
    batch: Vec<(ComponentHandle, EntityId)>,
    existing: HashSet<EntityId>,
    expected: Option<(EntityId, String)>,
    ammo: String,
    initial: u32,
    after_shot: Option<u32>,
    picked_up: bool,
    pending_seen: bool,
    focused_seen: bool,
    input_sent: bool,
    last_gate: Option<(bool, bool)>,
    support: Option<EntityId>,
    support_destroyed: bool,
    support_attempts:u32,
    support_misses:u32,
    tank:bool,
}
impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        let mut batch = Vec::new();
        scene.diagnostic_projectile_batch(&mut batch)?;
        let existing = batch.iter().map(|(_, id)| *id).collect();
        let input = input_for(25)?;
        send(world, input, true)?;
        eprintln!("ACTION_SMOKE_BOLT_BEGIN source=actual_fixture+physical_shortcut1+mouse no_spawn_override=true no_pose_override=true no_range_override=true no_fake_ACK=true");
        Ok(Self {
            stage: Stage::Select,
            since: Instant::now(),
            release: Some(input),
            movement: None,
            crouch: None,
            projectile: None,
            batch,
            existing,
            expected: None,
            ammo: String::new(),
            initial: 0,
            after_shot: None,
            picked_up: false,
            pending_seen: false,
            focused_seen: false,
            input_sent: false,
            last_gate: None,
            support: None,
            support_destroyed: false,
            support_attempts:0,
            support_misses:0,
            tank:false,
        })
    }
    pub(super) fn begin_bottle(world: &mut World) -> Result<Self, String> {
        Self::begin_support(world,"env_mulberry_mead_01_18",false)
    }
    pub(super) fn begin_tank(world:&mut World)->Result<Self,String>{
        //The same captured, physically occupied feet used by wall_thrust.
        //Only setup restores position; aim/fire use ordinary inputs.
        let feet=Vec3::new(13.866425,87.776184,21.560005);
        let forward=Vec3::new(0.5445311,-0.82580554,0.14673439);
        world.resource_mut::<PlayerState>().player.as_mut().ok_or("tank setup Player absent")?
            .teleport(feet,forward.y.atan2(forward.x));
        eprintln!("ACTION_SMOKE_TANK_SETUP source=collision-1795084974980 feet={feet:?} target=script_tank_station_wol setup_restore_only=true LOS_not_assumed=true");
        Self::begin_support(world,"script_tank_station_wol",true)
    }
    fn begin_support(world:&mut World,name:&str,tank:bool)->Result<Self,String>{
        let live = world.resource::<LiveEntities>();
        let target = live
            .find(name)
            .ok_or("existing reachable bottle fixture is absent on this level")?;
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        let health = scene
            .entity_health(live, target)?
            .ok_or("actual bottle fixture lacks Health")?
            .0;
        if health <= 0. {
            return Err("actual bottle fixture is already destroyed".into());
        }
        if scene.breakable_component(live, target)?.is_none() {
            return Err("actual bottle fixture lacks started Breakable".into());
        }
        if tank{
            let snapshot=scene.whale_oil_tank_snapshot(live,target)?.ok_or("exact tank lacks its actual component")?;
            eprintln!("ACTION_SMOKE_TANK_OWNER target={target:?} actual_component_energy_timer_thrown_started_updating_pusher={snapshot:?}");
        }
        let mut observation = Self::begin(world)?;
        observation.support = Some(target);
        observation.tank=tank;
        world.insert_resource(SupportContact {
            target,
            before_health: health,
            handoff: None,
            expected: None,
            fired: None,
            hit: None,
        });
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_BEGIN actual_target={target:?} before_health={health} fixture={name} tank={tank} physical_approach_look_fire_only=true no_damage_pose_or_RNG_override=true");
        Ok(observation)
    }
    fn aim_bottle(&mut self, world: &mut World) -> Result<(), String> {
        let target = self.support.ok_or("bottle observation target absent")?;
        let center = Self::center(world, target)?;
        if !Self::steer(world, center)? {
            self.stop(world)?;
            return Ok(());
        }
        if !self.tank&&!Self::focused(world, target) {
            self.move_with(world, 2)?;
            return Ok(());
        }
        self.stop(world)?;
        let p = player(world)?;
        //Installed rangedtarget enables DEBRIS alongside constructor WORLD
        //bits. This fixture requires its actual hit, not a forced target.
        if p.ranged_target_entity()!=Some(target){
            if self.tank&&self.last_gate!=Some((true,false)){
                eprintln!("ACTION_SMOKE_TANK_LOS actual_target={target:?} ranged_target={:?} ranged_hit={:?} fired=false no_navigation_or_ray_override=true",p.ranged_target_entity(),p.ranged_target_hit());
                self.last_gate=Some((true,false));
            }
            return Ok(());
        }
        let weapon = p
            .weapon
            .as_ref()
            .ok_or("bottle shot has no selected weapon")?;
        let source = weapon
            .fire_source
            .as_ref()
            .ok_or("bottle shot has no real fire source")?;
        if weapon.settings.family != RangedFamily::Crossbow
            || weapon.ammunition_index() != 0
            || source.prepared.declaration.sleep_damage.is_some()
        {
            return Err("bolt-bottle requires actual initial regular crossbow ammo".into());
        }
        let expected_birth = (
            source.attacker,
            source.equipment,
            std::sync::Arc::as_ptr(&source.prepared.declaration) as usize,
        );
        self.expected = Some((source.attacker, source.prepared.declaration.entity.clone()));
        self.ammo = weapon.ammunition().item.clone();
        self.initial = world
            .resource::<app::interaction_focus::DiscoveredDocuments>()
            .quantity(&self.ammo);
        if self.initial == 0 {
            return Err("bolt-bottle has no actual regular ammunition".into());
        }
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_AIM actual_target={target:?} center={center:?} actual_unlocked_focus=true initial_quantity={} no_ray_or_range_override=true",self.initial);
        //Arm only immediately before this physical fire. Actual spawn callback
        //correlates managed shooter/equipment AND prepared declaration identity.
        let input = input_for(20)?;
        self.after_shot=None;
        self.support_attempts+=1;
        let mut receipt=world.resource_mut::<SupportContact>();
        receipt.expected=Some(expected_birth);receipt.fired=None;receipt.hit=None;receipt.handoff=None;
        drop(receipt);
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_ATTEMPT attempt={} prior_misses={} target={target:?} actual_ranged_target={:?} actual_dispersion_ratio={} before_quantity={} no_settle_timer_or_velocity_override=true",self.support_attempts,self.support_misses,player(world)?.ranged_target_entity(),player(world)?.weapon.as_ref().unwrap().dispersion_ratio(),self.initial);
        send(world, input, true)?;
        self.release = Some(input);
        self.stage(Stage::Flight);
        Ok(())
    }
    fn retry_bottle(&mut self,world:&mut World,id:EntityId,reason:&str)->Result<(),String>{
        let target=self.support.ok_or("bottle retry lost actual target")?;
        let live=world.resource::<LiveEntities>();
        let scene=world.resource::<app::physics_scene::PhysicsScene>();
        let alive=live.get(target).is_some()
            &&scene.diagnostic_damage_flags(live,target)?.is_some_and(|flags|flags&0x800==0)
            &&scene.entity_health(live,target)?.is_some_and(|health|health.0>0.);
        self.explain(world,reason)?;
        if !alive{
            eprintln!("ACTION_SMOKE_NOT_COVERED check=destroyed_support_bolt_recovery reason=same_bottle_not_alive_for_physical_retry attempts={} misses={} no_recovery_claim=true",self.support_attempts,self.support_misses);
            self.stage(Stage::Done);return Ok(());
        }
        //Retain the missed actor/body and its native removal task untouched.
        //Only exclude its exact generation from this next attempt's observer.
        self.existing.insert(id);self.projectile=None;self.expected=None;
        let mut receipt=world.resource_mut::<SupportContact>();
        receipt.expected=None;receipt.fired=None;receipt.hit=None;receipt.handoff=None;
        drop(receipt);
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_RETRY previous_fired={id:?} reason={reason} same_target={target:?} attempts={} misses={} no_bolt_removed=true no_ammo_refund=true no_RNG_override=true",self.support_attempts,self.support_misses);
        self.stage(Stage::Retry);Ok(())
    }
    fn report_support(&self, world: &World, reason: &str) -> Result<(), String> {
        let Some(target) = self.support else {
            return Ok(());
        };
        let live = world.resource::<LiveEntities>();
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        let receipt = world.resource::<SupportContact>();
        if self.tank{
            eprintln!("ACTION_SMOKE_TANK_OWNER reason={reason} target={target:?} actual_component_energy_timer_thrown_started_updating_pusher={:?}",scene.whale_oil_tank_snapshot(live,target)?);
        }
        let health = if live.get(target).is_some() {
            scene.entity_health(live, target)?
        } else {
            None
        };
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_DAMAGE_STATE reason={reason} target={target:?} actual_header_flags={:?} death_bit=0x800 Health_mode2_absence_not_death_proof=true",scene.diagnostic_damage_flags(live,target)?);
        eprintln!("ACTION_SMOKE_BOLT_BOTTLE_SUPPORT reason={reason} target={target:?} generation_live={} health={health:?} before_health={} hit={:?} handoff={:?} destroyed_observed={} children={:?}",live.get(target).is_some(),receipt.before_health,receipt.hit,receipt.handoff,self.support_destroyed,if live.get(target).is_some(){Some(live.children(target)?)}else{None});
        if live.get(target).is_some() {
            eprintln!(
                "ACTION_SMOKE_BOLT_BOTTLE_TARGET {}",
                app::interactions::describe_carry_fixture(world, target)?
            );
        }
        Ok(())
    }
    fn stage(&mut self, next: Stage) {
        eprintln!(
            "ACTION_SMOKE_BOLT_STAGE previous={:?} next={next:?} elapsed={} actual_projectile={:?}",
            self.stage,
            self.since.elapsed().as_secs_f64(),
            self.projectile.map(|(_, id)| id)
        );
        self.stage = next;
        self.since = Instant::now();
        self.last_gate = None;
    }
    fn stop(&mut self, world: &mut World) -> Result<(), String> {
        if let Some(input) = self.movement.take() {
            send(world, input, false)?;
        }
        Ok(())
    }
    fn move_with(&mut self, world: &mut World, action: i32) -> Result<(), String> {
        let input = input_for(action)?;
        if self.movement.is_none() {
            send(world, input, true)?;
            self.movement = Some(input);
        }
        Ok(())
    }
    fn focused(world: &World, id: EntityId) -> bool {
        let Some(live) = world.get_resource::<LiveEntities>() else {
            return false;
        };
        let Some((name, _)) = live.get(id) else {
            return false;
        };
        world
            .get_resource::<app::interaction_focus::FocusedInteraction>()
            .and_then(|focus| focus.current.as_ref())
            .is_some_and(|prompt| !prompt.locked && prompt.entity.eq_ignore_ascii_case(name))
            && world.get_resource::<app::interactions::Interactions>()
                .is_some_and(|owner|owner.selected_interaction_target(live)==Some(id))
    }
    fn center(world: &World, id: EntityId) -> Result<Vec3, String> {
        let live = world.resource::<LiveEntities>();
        if let Some(bounds) = app::world_models::entity_bounds(world, live, id)? {
            return Ok((Vec3::from(bounds[0]) + Vec3::from(bounds[1])) * 0.5);
        }
        world
            .resource::<app::physics_scene::PhysicsScene>()
            .entity_frame(live, id)?
            .map(|frame| Vec3::from(frame.position))
            .ok_or_else(|| "bolt diagnostic actual target frame absent".into())
    }
    fn steer(world: &mut World, center: Vec3) -> Result<bool, String> {
        let transform = *world
            .query_filtered::<&Transform, With<Camera3d>>()
            .single(world)
            .map_err(|error| error.to_string())?;
        let eye = Vec3::from(app::view_to_game(transform.translation));
        let forward = Vec3::from(app::view_to_game(transform.forward().as_vec3()));
        let to = center - eye;
        let yaw = to.y.atan2(to.x) - forward.y.atan2(forward.x);
        let yaw = yaw.sin().atan2(yaw.cos());
        let pitch =
            -to.z.atan2(to.truncate().length()) + forward.z.atan2(forward.truncate().length());
        //Controller increments guide input only; actual physics/focus admit actions.
        world.write_message(bevy::input::mouse::MouseMotion {
            delta: Vec2::new((-yaw * 100.).clamp(-8., 8.), (pitch * 100.).clamp(-8., 8.)),
        });
        Ok(yaw.abs() < 0.05 && pitch.abs() < 0.05)
    }
    fn discover(&mut self, world: &World) -> Result<(), String> {
        if self.projectile.is_some() {
            return Ok(());
        }
        let Some((attacker, source)) = &self.expected else {
            return Ok(());
        };
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        scene.diagnostic_projectile_batch(&mut self.batch)?;
        for &(handle, id) in &self.batch {
            if self.existing.contains(&id) {
                continue;
            }
            if self.support.is_some() && world.resource::<SupportContact>().fired != Some(id) {
                continue;
            }
            let runtime = scene.projectile_runtime(handle)?;
            if runtime.identity == id
                && runtime.attacker == Some(*attacker)
                && runtime.declaration.entity == *source
            {
                self.projectile = Some((handle, id));
                eprintln!("ACTION_SMOKE_BOLT_BIRTH id={id:?} source={source} origin={:?} position={:?} stopped={} picked_up={} observation_includes_stopped_components=true",runtime.origin,runtime.position,runtime.has_hit_and_stopped,runtime.picked_up);
                break;
            }
        }
        Ok(())
    }
    fn explain(&self, world: &World, reason: &str) -> Result<(), String> {
        self.report_support(world, reason)?;
        let Some((handle, id)) = self.projectile else {
            return Ok(());
        };
        let live = world.resource::<LiveEntities>();
        if live.get(id).is_none() {
            eprintln!("ACTION_SMOKE_BOLT_QUERY reason={reason} id={id:?} generation_retired=true");
            return Ok(());
        }
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        //A native stopped/released component may disappear before its header.
        //Never read a reused slot as this old projectile or invent its flags.
        let runtime=match scene.projectile_runtime(handle){
            Ok(runtime)if runtime.identity==id=>Some(runtime),
            Ok(runtime)=>{eprintln!("ACTION_SMOKE_BOLT_COMPONENT reason={reason} id={id:?} old_handle={handle:?} slot_reused_by={:?}",runtime.identity);None},
            Err(error)=>{eprintln!("ACTION_SMOKE_BOLT_COMPONENT reason={reason} id={id:?} old_handle={handle:?} actual_component_unavailable={error}");None},
        };
        let (eye, axis) = player(world)?.published_camera_frame();
        let sensor = world.resource::<app::interactions::Interactions>().sensor;
        let model = world.get_resource::<app::world_models::WorldModels>();
        eprintln!("ACTION_SMOKE_BOLT_QUERY reason={reason} id={id:?} eye={eye:?} actual_entity_frame={:?} runtime_snapshot={:?} throwable_started={:?} actual_recoverable={:?} model_publication={:?} source_visibility={:?}\n{}",
            scene.entity_frame(live,id),runtime.map(|runtime|(runtime.position,runtime.has_hit_and_stopped,runtime.impact_stopped,runtime.picked_up)),
            runtime.map(|_|scene.projectile_throwable_started(handle)),scene.projectile_recoverable(id),model.and_then(|model|model.publication_pose(id)),model.and_then(|model|model.publication_visibility(id)),
            scene.describe_interaction_fixture(live,id,eye,axis.x_axis,sensor).unwrap_or_else(|error|format!("actual interaction diagnostic unavailable: {error}")));
        if let Some(model) = model {
            let query = world
                .get_resource::<app::SceneRenderQueue>()
                .and_then(|queue| queue.shadow_ordinary_query.as_ref())
                .and_then(|query| query.state.as_ref());
            let pixels = world
                .get_resource::<app::SceneRenderQueue>()
                .map_or(0, |queue| queue.texture_viewport_pixels);
            if let Some(reason) = model.query_diagnostic(id, query, pixels)? {
                eprintln!("ACTION_SMOKE_BOLT_CULL id={id:?} actual_query=true {reason}");
            }
            if let Some(query) = query {
                if let Some(reason) = model.query_diagnostic(id, Some(query), 1366 * 768)? {
                    eprintln!("ACTION_SMOKE_BOLT_CULL_1366X768 id={id:?} diagnostic_only=true actual_rows_and_depth_unchanged=true not_GPU_visibility_proof=true {reason}");
                }
            }
        }
        eprintln!("ACTION_SMOKE_BOLT_INTERACTION reason={reason}\n{}",app::interactions::describe_carry_fixture(world,id).unwrap_or_else(|error|format!("actual carry diagnostic unavailable: {error}")));
        Ok(())
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        self.discover(world)?;
        if let Some(input) = self.release.take() {
            send(world, input, false)?;
            return Ok(false);
        }
        if self.since.elapsed() > Duration::from_secs(30) {
            self.stop(world)?;
            if let Some(input) = self.crouch.take() {
                send(world, input, false)?;
            }
            self.explain(world, "timeout")?;
            if self.tank&&matches!(self.stage,Stage::Aim)&&self.projectile.is_none(){
                eprintln!("ACTION_SMOKE_NOT_COVERED check=actual_whale_oil_tank_firearm_detonation reason=captured_start_did_not_produce_exact_target_native_ranged_LOS target={:?} actual_ranged_target={:?} actual_ranged_hit={:?} fired=false no_navigation_or_target_override=true",self.support,player(world)?.ranged_target_entity(),player(world)?.ranged_target_hit());
                self.stage(Stage::Done);
                return Ok(false);
            }
            return Err(format!("actual bolt diagnostic timeout {:?}; focused={} pickup_input={} pending_seen={} picked_up={} after_shot={:?}",self.stage,self.focused_seen,self.input_sent,self.pending_seen,self.picked_up,self.after_shot));
        }
        match self.stage {
            Stage::Select => {
                if super::Observation::ranged_ready(world, RangedFamily::Crossbow)? {
                    if self.support.is_none() {
                        let input = input_for(player(world)?.controller.sneak_action)?;
                        send(world, input, true)?;
                        self.crouch = Some(input);
                    }
                    self.stage(Stage::Aim);
                }
            }
            Stage::Aim => {
                if !super::Observation::ranged_ready(world, RangedFamily::Crossbow)? {
                    return Ok(false);
                }
                if self.support.is_some() {
                    self.aim_bottle(world)?;
                    return Ok(false);
                }
                if let Some(hit) = super::Observation::world_fixture(world)? {
                    let p = player(world)?;
                    let weapon = p
                        .weapon
                        .as_ref()
                        .ok_or("bolt fixture crossbow disappeared")?;
                    if weapon.settings.family != RangedFamily::Crossbow {
                        return Err("bolt fixture selected a different ranged family".into());
                    }
                    let source = weapon
                        .fire_source
                        .as_ref()
                        .ok_or("bolt fixture actual fire source absent")?;
                    if weapon.ammunition_index() != 0
                        || source.prepared.declaration.sleep_damage.is_some()
                    {
                        return Err("bolt scenario requires the installed initial regular ammunition; no ammo state is overridden".into());
                    }
                    let world_owner = world
                        .resource::<app::physics_scene::PhysicsScene>()
                        .point_queries()?;
                    let query = world_owner
                        .read()
                        .map_err(|_| "bolt WORLD fixture material owner poisoned")?;
                    if hit.material_id == u16::MAX {
                        return Err("actual WORLD fixture lacks a material; recoverable Stick cannot be certified".into());
                    }
                    let material = query.storage.materials.material_for_id(hit.material_id)?;
                    eprintln!("ACTION_SMOKE_BOLT_WORLD_FIXTURE hit={hit:?} actual_material_can_stick={} actual_material_can_break={} source_required_stick_angle={} source_can_break={} source_break_chance_stickable={} source_may_still_Break=true no_collision_or_RNG_override=true",
                        material.projectile_can_stick,material.projectile_can_break,source.prepared.declaration.required_stick_angle,
                        source.prepared.declaration.can_break,source.prepared.declaration.break_chance_stickable);
                    if !material.projectile_can_stick {
                        eprintln!("ACTION_SMOKE_NOT_COVERED check=actual_bolt_focus_pickup reason=source624300_WORLD_support_material_forbids_Stick no_gameplay_failure_claim=true");
                        self.stage(Stage::Done);
                        return Ok(false);
                    }
                    drop(query);
                    self.expected =
                        Some((source.attacker, source.prepared.declaration.entity.clone()));
                    self.ammo = weapon.ammunition().item.clone();
                    self.initial = world
                        .resource::<app::interaction_focus::DiscoveredDocuments>()
                        .quantity(&self.ammo);
                    self.after_shot = None;
                    if self.initial == 0 {
                        return Err("bolt fixture has no actual regular ammunition".into());
                    }
                    let input = input_for(20)?;
                    send(world, input, true)?;
                    self.release = Some(input);
                    self.stage(Stage::Flight);
                } else {
                    //Reuse the WORLD scenario's physical downward aiming; the
                    //same nearest support ray and authored focus admit the fixture.
                    world.write_message(bevy::input::mouse::MouseMotion {
                        delta: Vec2::new(0., 8.),
                    });
                }
            }
            Stage::Flight => {
                let quantity = world
                    .resource::<app::interaction_focus::DiscoveredDocuments>()
                    .quantity(&self.ammo);
                if quantity < self.initial {
                    self.after_shot = Some(quantity);
                }
                if let Some((handle, id)) = self.projectile {
                    if let Some(target) = self.support {
                        let Some((fired, actual, response, _)) =
                            world.resource::<SupportContact>().hit
                        else {
                            return Ok(false);
                        };
                        if fired!=id{return Err(format!("bolt-bottle exact birth identity differs: receipt={fired:?} observed={id:?}"));}
                        if actual!=Some(target){
                            self.support_misses+=1;
                            self.retry_bottle(world,id,"actual_fired_bolt_missed_living_bottle")?;
                            return Ok(false);
                        }
                        let live = world.resource::<LiveEntities>();
                        let scene=world.resource::<app::physics_scene::PhysicsScene>();
                        let damage_flags=scene.diagnostic_damage_flags(live,target)?;
                        //damage_target commits0x800 on actual death beforeA5/A2;
                        //later componentStop hides Health from its mode2 lookup.
                        //Missing Health by itself is never accepted as death.
                        self.support_destroyed=live.get(target).is_none()
                            ||damage_flags.is_some_and(|flags|flags&0x800!=0)
                            ||scene.entity_health(live,target)?.is_some_and(|health|health.0<=0.);
                        if self.tank&&self.support_destroyed{
                            self.report_support(world,"actual-tank-shot-death-committed")?;
                            eprintln!("ACTION_SMOKE_PASS check=actual_whale_oil_tank_firearm_detonation exact_fired={id:?} exact_target={target:?} source_health_death_observed=true recovery_not_claimed=true");
                            self.stage(Stage::Done);
                            return Ok(false);
                        }
                        if !self.support_destroyed {
                            if super::Observation::ranged_ready(world, RangedFamily::Crossbow)? {
                                self.retry_bottle(world,id,"actual_contact_did_not_destroy_living_bottle")?;
                            }
                            return Ok(false);
                        }
                        if live.get(id).is_none() {
                            self.report_support(world, "source-retired-bolt-after-contact")?;
                            eprintln!("ACTION_SMOKE_NOT_COVERED check=destroyed_support_surviving_bolt_recovery reason=actual_bolt_retired response={response:?} destroyed_support=true no_recovery_claim=true");
                            self.stage(Stage::Done);
                            return Ok(false);
                        }
                    }
                    let live = world.resource::<LiveEntities>();
                    if live.get(id).is_none() {
                        return Err("actual regular bolt retired on contact before a recoverable stick; no recovery claim".into());
                    }
                    let scene = world.resource::<app::physics_scene::PhysicsScene>();
                    let runtime = scene.projectile_runtime(handle)?;
                    let owner_hidden = world
                        .get_resource::<app::world_models::WorldModels>()
                        .and_then(|models| models.publication_visibility(id))
                        .is_some_and(|(requested, _, _)| !requested);
                    if runtime.has_hit_and_stopped && owner_hidden {
                        self.explain(world, "actual-hidden-Break-or-Uncanny-response")?;
                        if self.support.is_some() {
                            eprintln!("ACTION_SMOKE_NOT_COVERED check=destroyed_support_surviving_bolt_recovery reason=actual_source_Break_or_Uncanny destroyed_support={} no_recovery_claim=true",self.support_destroyed);
                            self.stage(Stage::Done);
                            return Ok(false);
                        }
                        eprintln!("ACTION_SMOKE_BOLT_RETRY previous={id:?} reason=actual_source_Break_or_Uncanny_response current_quantity={quantity} before_shot={} physical_next_shot_only=true no_RNG_override=true",self.initial);
                        self.existing.insert(id);
                        self.projectile = None;
                        self.expected = None;
                        self.stage(Stage::Retry);
                        return Ok(false);
                    }
                    //Native Bounce transfers finite movement to the same real
                    //Throwable without setting the Stick/Break stopped bit.
                    let actual_bounce_handoff=self.support.is_some()
                        &&!runtime.movement_enabled
                        &&world.resource::<SupportContact>().hit.is_some_and(|hit|hit.0==id&&hit.2==crate::gameplay::projectiles::Response::Bounce);
                    if (runtime.has_hit_and_stopped||actual_bounce_handoff)
                        &&scene.projectile_throwable_started(handle)? {
                        self.explain(world, "actual-impact-after-Throwable-Start")?;
                        if !scene.projectile_recoverable(id)? {
                            if self.support.is_some() {
                                eprintln!("ACTION_SMOKE_NOT_COVERED check=destroyed_support_bolt_recovery reason=source625e80_disabled_Pickable no_recovery_claim=true");
                                self.stage(Stage::Done);
                                return Ok(false);
                            }
                            return Err("actual contact disabled Pickable via source625e80; recovery not admitted".into());
                        }
                        self.stage(Stage::BackAway);
                    }
                }
            }
            Stage::Retry => {
                let quantity = world
                    .resource::<app::interaction_focus::DiscoveredDocuments>()
                    .quantity(&self.ammo);
                if quantity >= self.initial {
                    return Err("actual prior shot did not consume inventory ammunition; retry would not be bounded by the real inventory".into());
                }
                if quantity == 0 {
                    eprintln!("ACTION_SMOKE_NOT_COVERED check={} reason=actual_regular_ammunition_exhausted_after_native_contacts attempts={} misses={} no_recovery_claim=true no_inventory_or_RNG_override=true",if self.support.is_some(){"destroyed_support_bolt_recovery"}else{"actual_bolt_focus_pickup"},self.support_attempts,self.support_misses);
                    self.stage(Stage::Done);
                } else if super::Observation::ranged_ready(world, RangedFamily::Crossbow)? {
                    //The next Aim samples its own actual pre-shot quantity;
                    //recovering it must not refund any earlier broken bolt.
                    self.stage(Stage::Aim);
                }
            }
            Stage::BackAway | Stage::Recover => {
                let id = self
                    .projectile
                    .ok_or("bolt recovery lost actual identity")?
                    .1;
                let center = Self::center(world, id)?;
                let (eye, _) = player(world)?.published_camera_frame();
                if self.stage == Stage::BackAway {
                    //Observation position beyond the user's reported ~2m cutoff.
                    //This only steers ordinary movement; culling stays unchanged.
                    if eye.distance(center) >= 3. {
                        self.stop(world)?;
                        self.explain(world, "physical-distance-beyond-user-two-metre-report")?;
                        self.stage(Stage::Recover);
                    } else if Self::steer(world, center)? {
                        self.move_with(world, 3)?;
                    } else {
                        self.stop(world)?;
                    }
                } else if Self::focused(world, id) {
                    self.stop(world)?;
                    self.focused_seen = true;
                    self.explain(world, "actual-unlocked-bolt-focus")?;
                    let input = input_for(22)?;
                    send(world, input, true)?;
                    self.release = Some(input);
                    self.input_sent = true;
                    self.stage(Stage::Pickup);
                } else {
                    if Self::steer(world, center)? {
                        self.move_with(world, 2)?;
                    } else {
                        self.stop(world)?;
                    }
                }
            }
            Stage::Pickup => {
                let (handle, id) = self.projectile.unwrap();
                let live = world.resource::<LiveEntities>();
                self.pending_seen |= world
                    .resource::<app::interactions::Interactions>()
                    .pending_pickup
                    .as_ref()
                    .is_some_and(|pending| pending.entity == id);
                if live.get(id).is_some() {
                    let scene = world.resource::<app::physics_scene::PhysicsScene>();
                    let runtime = scene.projectile_runtime(handle)?;
                    if runtime.identity != id {
                        return Err("bolt pickup component generation changed".into());
                    }
                    self.picked_up |= runtime.picked_up;
                }
                let quantity = world
                    .resource::<app::interaction_focus::DiscoveredDocuments>()
                    .quantity(&self.ammo);
                let pending = world
                    .resource::<app::interactions::Interactions>()
                    .pending_pickup
                    .as_ref()
                    .is_some_and(|pending| pending.entity == id);
                let hidden = world
                    .get_resource::<app::world_models::WorldModels>()
                    .and_then(|models| models.publication_visibility(id))
                    .is_some_and(|(enabled, published, _)| !enabled && !published)
                    || live.get(id).is_none();
                let gate = (self.picked_up, hidden);
                if self.last_gate != Some(gate) {
                    self.last_gate = Some(gate);
                    eprintln!("ACTION_SMOKE_BOLT_PICKUP id={id:?} actual_source6274d0={} source_hidden={hidden} pending={pending} state47={} quantity={quantity} initial={} after_shot={:?}",self.picked_up,player(world)?.machine.is_active(StateId::LOOT_OBJECT),self.initial,self.after_shot);
                }
                if self.picked_up
                    && hidden
                    && !pending
                    && quantity == self.initial
                    && self.after_shot.is_some_and(|after| quantity > after)
                {
                    eprintln!("ACTION_SMOKE_PASS check=actual_fired_bolt_focus_pickup_ammo id={id:?} actual_state47_transaction_sampled={} source6274d0=true source76ec20_hidden=true ammunition_restored=true",self.pending_seen);
                    if self.support.is_some() {
                        self.report_support(world, "exact-fired-ID-picked-up-after-destruction")?;
                        eprintln!("ACTION_SMOKE_PASS check=actual_destroyed_bottle_surviving_bolt_recovery fired={id:?} destroyed_support={} exact_focus_pickup_ammo=true",self.support_destroyed);
                    }
                    self.stage(Stage::Retirement);
                }
            }
            Stage::Retirement => {
                let id = self.projectile.unwrap().1;
                if world.resource::<LiveEntities>().get(id).is_none() {
                    eprintln!("ACTION_SMOKE_PASS check=actual_recovered_bolt_header_retired id={id:?} original_generation_absent=true");
                    self.stage(Stage::Done);
                } else {
                    let task = app::entity_functions::diagnostic_hidden_removal(world, id)?;
                    eprintln!("ACTION_SMOKE_BOLT_RETIREMENT id={id:?} source=existing_883f60_clock2_RemovalWhenHidden actual_pending_task={task:?} actual_header_retired=false completion_not_claimed=true no_time_or_visibility_override=true");
                    eprintln!("ACTION_SMOKE_NOT_COVERED check=recovered_bolt_header_deletion reason=original_delayed_removal_not_accelerated final_level_teardown_checked_separately=true");
                    self.stage(Stage::Done);
                }
            }
            Stage::Done => {
                self.stop(world)?;
                if let Some(input) = self.crouch.take() {
                    send(world, input, false)?;
                }
                if self.support.is_some() {
                    world.remove_resource::<SupportContact>();
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
}
