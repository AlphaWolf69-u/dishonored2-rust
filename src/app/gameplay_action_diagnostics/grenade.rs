//! Opt-in real-input grenade hold→world gauge ACK→throw→Sphere→explosion.
//! No inventory, source, target, time, body or renderer state is substituted.
use super::super as app;
use super::{Input, input_for, player, send};
use crate::gameplay::entity_lifetime::{EntityId, LiveEntities};
use bevy::prelude::*;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Resource, Default)]
struct Receipt {
    target: Option<EntityId>,
    exploded: bool,
}
///Called only after the actual490840 dispatch finishes all accepted phases.
pub(in crate::app) fn exploded(world: &mut World, entity: EntityId) {
    if let Some(mut receipt) = world.get_resource_mut::<Receipt>() {
        if receipt.target == Some(entity) {
            receipt.exploded = true;
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Select,
    Reselect,
    Hold,
    Throw,
    Explosion,
    Done,
}
#[derive(Clone, Copy)]
struct Tick {
    wall: Instant,
    app: Option<u32>,
    game: Option<i32>,
    game_time: Option<f64>,
}
#[derive(Default)]
struct Progress {
    first: Option<Tick>,
    last: Option<Tick>,
    call_finished: Option<Tick>,
    longest_gap: Option<(Tick, Tick)>,
    polls: u64,
    signature: Option<(bool, Option<crate::gameplay::entity_lifetime::Stage>, Option<(Option<u8>, bool, Option<bool>)>)>,
}
pub(super) struct Observation {
    stage: Stage,
    since: Instant,
    release: Option<Input>,
    use_input: Input,
    selection: Input,
    request_uid: Option<u32>,
    repeats: bool,
    held: Option<EntityId>,
    initial_progress: Option<f32>,
    before: HashSet<EntityId>,
    thrown: Option<EntityId>,
    hold_observation: Option<(u16, Option<EntityId>, Option<EntityId>)>,
    progress: Progress,
}
impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        let inventory = world
            .get_resource::<app::interaction_focus::DiscoveredDocuments>()
            .ok_or("Grenade diagnostic actual inventory absent")?;
        let p = player(world)?;
        let (index, item, archetype) = (0..10).find_map(|index| {
            let item = inventory.wheel_shortcut_pair(index).0?;
            let archetype = p.equipment_target_metadata(item)?.0.archetype;
            (archetype == 4).then(|| (index, item.to_owned(), archetype))
        }).ok_or("Grenade diagnostic requires an actual owned grenade shortcut in this level/loadout")?;
        let selection = input_for(25 + index as i32)?;
        let use_input = input_for(20)?;
        let scene = world
            .get_resource::<app::physics_scene::PhysicsScene>()
            .ok_or("Grenade diagnostic Scene absent")?;
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("Grenade diagnostic identities absent")?;
        let before = scene
            .debug_grenades(live)?
            .into_iter()
            .map(|entry| entry.entity)
            .collect();
        world.insert_resource(Receipt::default());
        eprintln!(
            "ACTION_SMOKE_GRENADE_BEGIN owned_item={item} actual_archetype={archetype} selection={selection:?} use_input={use_input:?} no_time_override=true no_fake_ACK=true"
        );
        send(world, selection, true)?;
        Ok(Self {
            stage: Stage::Select,
            since: Instant::now(),
            release: Some(selection),
            use_input,
            selection,
            request_uid: None,
            repeats: std::env::var_os("DH2_DEBUG_EXHAUST_GRENADES").is_some(),
            held: None,
            initial_progress: None,
            before,
            thrown: None,
            hold_observation: None,
            progress: Progress::default(),
        })
    }
    fn stage(&mut self, stage: Stage) {
        eprintln!(
            "ACTION_SMOKE_GRENADE_STAGE from={:?} next={stage:?} elapsed={} held={:?} thrown={:?}",
            self.stage,
            self.since.elapsed().as_secs_f64(),
            self.held,
            self.thrown
        );
        self.stage = stage;
        self.since = Instant::now();
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        if self.stage == Stage::Explosion { self.observe_progress(world)?; }
        if let Some(error) = app::runtime_materials::equipment_admission::startup_error(world) {
            return Err(format!("actual equipment publication failed during {:?}: {error}", self.stage));
        }
        //Inspect the causal constructor failure before any next global retry
        //can replace it with an entered-component readiness diagnostic.
        if let Some(error) = app::weapon_throwing::startup_error(world) {
            return Err(format!(
                "actual grenade throw failed during {:?}: {error}",
                self.stage
            ));
        }
        if let Some(error) = app::grenade_cooking::startup_error(world) {
            return Err(format!("actual grenade gauge failed: {error}"));
        }
        if let Some(input) = self.release.take() {
            send(world, input, false)?;
            return Ok(false);
        }
        if self.since.elapsed() > Duration::from_secs(90) {
            let mut report = String::new();
            player(world)?.write_combat_diagnostics(&mut report);
            app::flash_surfaces::write_diagnostics(world, &mut report);
            return Err(format!(
                "Grenade actual-input timeout stage={:?} held={:?} thrown={:?}\n{}\n{report}",
                self.stage,
                self.held,
                self.thrown,
                app::weapon_throwing::debug_startup_pending(world)
            ));
        }
        let scene = world
            .get_resource::<app::physics_scene::PhysicsScene>()
            .ok_or("Grenade diagnostic Scene absent")?;
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("Grenade diagnostic identities absent")?;
        let grenades = scene.debug_grenades(live)?;
        if self.stage == Stage::Hold {
            //Capture transitions while their owners still exist, before exit
            //correctly retires them. This is bounded by predicate changes,
            //not a per-frame history or a new gameplay/state/time input.
            let held = grenades.iter().find(|entry| Some(entry.entity) == self.held);
            let runtime = player(world)?.throwable.as_ref().map(|weapon|
                (weapon.entity, weapon.cooking_duration, weapon.cooking_stage, weapon.ready, weapon.total_ammo));
            let parent = self.held.map(|id| world.resource::<LiveEntities>().parent(id)).transpose()?.flatten();
            let arm = parent.is_some_and(|parent| player(world).is_ok_and(|player|
                player.body.arms.iter().any(|arm| arm.managed_entity == Some(parent))));
            let gauge = app::grenade_cooking::debug_gauge(world)?;
            let lead = app::grenade_cooking::debug_lead(world);
            let flags = u16::from(runtime.is_some())
                | (u16::from(runtime.is_some_and(|value| value.1 > 0.)) << 1)
                | (u16::from(held.is_some_and(|value| value.armed)) << 2)
                | (u16::from(held.is_some_and(|value| value.exploded)) << 3)
                | (u16::from(gauge.is_some_and(|value| value.1)) << 4)
                | (u16::from(gauge.is_some_and(|value| value.2)) << 5)
                | (u16::from(lead.is_some_and(|value| value.2)) << 6)
                | (u16::from(lead.is_some_and(|value| value.3)) << 7)
                | (u16::from(arm) << 8);
            let signature = (flags, parent, gauge.and_then(|value| value.3));
            if self.hold_observation != Some(signature) {
                eprintln!("ACTION_SMOKE_GRENADE_HOLD runtime={runtime:?} actual_parent={parent:?} parent_is_actual_arm={arm} fuse={held:?} gauge={gauge:?} lead={lead:?}");
                self.hold_observation = Some(signature);
            }
            if held.is_some_and(|value| value.exploded) {
                return Err(format!("actual held grenade exploded before gauge/throw readiness; held={held:?} runtime={runtime:?} parent={parent:?} parent_is_arm={arm} gauge={gauge:?} lead={lead:?}"));
            }
        }
        match self.stage {
            Stage::Select => {
                let p = player(world)?;
                let ready = p
                    .equipment_action_status(0)
                    .is_some_and(|(_, equipped, changing)| equipped && !changing);
                if let Some(weapon) = p.throwable.as_ref().filter(|weapon| ready && weapon.ready) {
                    if weapon.total_ammo == 0 || weapon.source.is_none() {
                        return Err("owned grenade has no actual ammunition/prepared firing source; fixture is unavailable".into());
                    }
                    eprintln!(
                        "ACTION_SMOKE_GRENADE_SOURCE entity={:?} actual_ammo={} ammo_index={} fire_scene={} detonation_source=canonical_Grenade",
                        weapon.entity,
                        weapon.total_ammo,
                        weapon.ammunition_index,
                        weapon.active_scene
                    );
                    self.held = Some(weapon.entity);
                    self.before.insert(weapon.entity);
                    self.request_uid = p.equipment_request_uid(0);
                    send(world, self.selection, true)?;
                    self.release = Some(self.selection);
                    self.stage(Stage::Reselect);
                }
            }
            Stage::Reselect => {
                let p = player(world)?;
                if self.request_uid != p.equipment_request_uid(0) || p.throwable.as_ref().map(|weapon| weapon.entity) != self.held {
                    return Err("reselecting equipped grenade started a new equipment request".into());
                }
                if p.equipment_action_status(0).is_some_and(|(_, equipped, changing)| equipped && !changing) {
                    eprintln!("ACTION_SMOKE_PASS check=repeat_shortcut_preserved_equipment actual_request_uid={:?} owner={:?}", self.request_uid, self.held);
                    send(world, self.use_input, true)?;
                    self.stage(Stage::Hold);
                }
            }
            Stage::Hold => {
                if let Some(grenade) = grenades
                    .iter()
                    .find(|entry| Some(entry.entity) == self.held && entry.armed && !entry.exploded)
                {
                    let first = *self.initial_progress.get_or_insert(grenade.progress);
                    let gauge = app::grenade_cooking::debug_gauge(world)?;
                    let actual_arm = world.resource::<LiveEntities>().parent(grenade.entity)?;
                    if grenade.progress > first
                        && gauge.is_some_and(|(_, initialized, actual_ack, parent)| {
                            initialized && actual_ack && parent.is_some() && parent == actual_arm
                        })
                    {
                        eprintln!(
                            "ACTION_SMOKE_PASS check=grenade_real_fuse_and_gauge_Init_render_ACK held={:?} total={} initial_progress={first} current_progress={} gauge={gauge:?}",
                            grenade.entity, grenade.total, grenade.progress
                        );
                        send(world, self.use_input, false)?;
                        self.stage(Stage::Throw);
                    }
                }
            }
            Stage::Throw => {
                if let Some(grenade) = grenades
                    .iter()
                    .find(|entry| !self.before.contains(&entry.entity) && entry.armed)
                {
                    let Some((body, shape_type)) = grenade.body_shape else {
                        return Ok(false);
                    };
                    if shape_type != 2 {
                        return Err(format!(
                            "actual grenade body {body:#x} admitted shape type {shape_type}, expected installed hknpSphereShape2"
                        ));
                    }
                    self.thrown = Some(grenade.entity);
                    world.resource_mut::<Receipt>().target = self.thrown;
                    eprintln!(
                        "ACTION_SMOKE_PASS check=grenade_real_throw_and_Sphere_body full_entity={:?} body={body:#x} actual_shape_type={shape_type} actual_progress={}",
                        grenade.entity, grenade.progress
                    );
                    self.stage(Stage::Explosion);
                    self.observe_progress(world)?;
                }
            }
            Stage::Explosion => {
                let receipt = world.resource::<Receipt>();
                let gone = self
                    .thrown
                    .is_some_and(|entity| world.resource::<LiveEntities>().get(entity).is_none());
                if receipt.exploded && gone {
                    self.write_progress();
                    eprintln!(
                        "ACTION_SMOKE_PASS check=grenade_actual_490840_completion_and_entity_retirement full_entity={:?} exit=ordinary_ExitDrain",
                        self.thrown
                    );
                    if self.repeats {
                        let p = player(world)?;
                        let weapon = p.throwable.as_ref().ok_or("empty throwable retired before unequip")?;
                        if weapon.total_ammo > 0 {
                            self.before.extend(self.thrown.take());
                            self.initial_progress = None;
                            self.hold_observation = None;
                            self.progress = Progress::default();
                            *world.resource_mut::<Receipt>() = Receipt::default();
                            self.stage(Stage::Select);
                            return Ok(false);
                        }
                        let pose = p.body.arms.first().map(|arm| arm.equipment());
                        if pose != Some("Empty") || p.body.equipment[0].as_ref().is_none_or(|tree| tree.visible) {
                            return Err(format!("empty grenade retained incorrect arm/model presentation: pose={pose:?}"));
                        }
                        eprintln!("ACTION_SMOKE_PASS check=last_grenade_empty_hand owner={:?} ammo={} magazine={} pose={pose:?} held_model_visible=false", weapon.entity, weapon.total_ammo, weapon.magazine);
                    }
                    self.stage(Stage::Done);
                }
            }
            Stage::Done => {
                world.remove_resource::<Receipt>();
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn observe_progress(&mut self, world: &World) -> Result<(), String> {
        let clock = world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .and_then(|owner| owner.clock().ok());
        let current = Tick { wall: Instant::now(),
            app: world.get_resource::<bevy::diagnostic::FrameCount>().map(|frame| frame.0),
            game: clock.map(|clock| clock.frame_marker()),
            game_time: clock.and_then(|clock| clock.time(0)),
        };
        let progress = &mut self.progress;
        if let Some(previous) = progress.last {
            let gap = current.wall.duration_since(previous.wall);
            if progress.longest_gap.is_none_or(|(before, after)| gap > after.wall.duration_since(before.wall)) {
                progress.longest_gap = Some((previous, current));
            }
        }
        progress.first.get_or_insert(current);
        progress.last = Some(current);
        progress.polls += 1;
        let live = world.resource::<LiveEntities>();
        let stage = self.thrown.and_then(|id| live.stage(id));
        let tail = self.thrown.map(|id| world.resource::<app::physics_scene::PhysicsScene>()
            .debug_grenade_tail(live, id)).transpose()?.flatten();
        let signature = (world.resource::<Receipt>().exploded, stage, tail);
        if signature.0 { progress.call_finished.get_or_insert(current); }
        if progress.signature != Some(signature) {
            eprintln!("ACTION_SMOKE_GRENADE_PROGRESS stage={:?} app_frame={:?} game_frame={:?} game_time={:?} actual_490840_finished={} entity_stage={stage:?} tail_call_phase_remove_on_FX_end_effect_completed={tail:?} polls={}",
                self.stage, current.app, current.game, current.game_time, signature.0, progress.polls);
            progress.signature = Some(signature);
        }
        Ok(())
    }
    fn write_progress(&self) {
        let progress = &self.progress;
        let Some((first, last)) = progress.first.zip(progress.last) else { return };
        let app_delta = first.app.zip(last.app).map(|(a, b)| b.wrapping_sub(a));
        let game_delta = first.game.zip(last.game).map(|(a, b)| b.wrapping_sub(a));
        let game_seconds = first.game_time.zip(last.game_time).map(|(a, b)| b - a);
        let gap = progress.longest_gap.map(|(a, b)| (
            b.wall.duration_since(a.wall).as_secs_f64() * 1000., a.app, b.app, a.game, b.game));
        let tail = progress.call_finished.map(|first| (
            last.wall.duration_since(first.wall).as_secs_f64(),
            first.app.zip(last.app).map(|(a,b)| b.wrapping_sub(a)),
            first.game.zip(last.game).map(|(a,b)| b.wrapping_sub(a)),
            first.game_time.zip(last.game_time).map(|(a,b)| b-a)));
        eprintln!("ACTION_SMOKE_GRENADE_FRAME_GAPS polls={} wall_seconds={} app_frames={app_delta:?} game_frames={game_delta:?} game_seconds={game_seconds:?} longest_poll_gap_ms_app_before_after_game_before_after={gap:?} retirement_is_not_frame_time=true",
            progress.polls, last.wall.duration_since(first.wall).as_secs_f64());
        eprintln!("ACTION_SMOKE_GRENADE_FX_END_WAIT observed_490840_completion_to_retirement_wall_seconds_app_frames_game_frames_game_seconds={tail:?}");
    }
}
