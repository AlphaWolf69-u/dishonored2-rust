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
    Hold,
    Throw,
    Explosion,
    Done,
}
pub(super) struct Observation {
    stage: Stage,
    since: Instant,
    release: Option<Input>,
    use_input: Input,
    held: Option<EntityId>,
    initial_progress: Option<f32>,
    before: HashSet<EntityId>,
    thrown: Option<EntityId>,
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
            matches!(archetype, 4 | 5).then(|| (index, item.to_owned(), archetype))
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
            held: None,
            initial_progress: None,
            before,
            thrown: None,
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
                    if grenade.progress > first
                        && gauge
                            .is_some_and(|(_, initialized, actual_ack)| initialized && actual_ack)
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
                }
            }
            Stage::Explosion => {
                let receipt = world.resource::<Receipt>();
                let gone = self
                    .thrown
                    .is_some_and(|entity| world.resource::<LiveEntities>().get(entity).is_none());
                if receipt.exploded && gone {
                    eprintln!(
                        "ACTION_SMOKE_PASS check=grenade_actual_490840_completion_and_entity_retirement full_entity={:?} exit=ordinary_ExitDrain",
                        self.thrown
                    );
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
}
