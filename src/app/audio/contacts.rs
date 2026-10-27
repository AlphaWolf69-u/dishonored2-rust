//! Host scheduling for the sound portion of material-contact FX. Definitions,
//! ancestry and event IDs are read from the installation and cached by name.
use super::{AudioTarget, SoundEvent};
use crate::{archive::Catalog, assets::{entities::{contacts::{material_chain, ContactDatabase}, player::resolved_decl, Value}, fx},
    gameplay::{effects::{groups::sample_offsets, sound::{sample_position, SoundEffect, SoundEvent as TimedEvent}, timing::EffectTimingRanges},
        game_clock_runtime::{ClockRuntime, GamePaused}, sound::{Contact, Emitter, Request}}};
use bevy::prelude::*;
use std::{collections::{HashMap, HashSet}, sync::Arc};

#[derive(Message)]
pub(in crate::app) struct ContactSound(pub Contact);

struct Sound {
    position: [Vec3; 2],
    start: TimedEvent,
}
struct Group {
    location: fx::EffectGroupLocation,
    timing: EffectTimingRanges,
    sounds: Vec<Sound>,
}
struct Definitions {
    catalog: Catalog,
    database: ContactDatabase,
    chains: HashMap<String, Vec<String>>,
    effects: HashMap<String, Arc<Vec<Group>>>,
}

fn text(catalog: &Catalog, path: &str) -> Result<Option<String>, String> {
    catalog.read(path).map(|bytes| bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned())).map_err(|error| error.to_string())
}

impl Definitions {
    fn load() -> Result<Self, String> {
        let base = crate::assets::installation::game_base().map_err(|error| error.to_string())?;
        let catalog = Catalog::open(&base).map_err(|error| error.to_string())?;
        let source = text(&catalog, "generated/decls/contactsystemdb/default_db.decl")?.ok_or("contact database is absent")?;
        let database = ContactDatabase::parse(&source)?;
        Ok(Self { catalog, database, chains: HashMap::new(), effects: HashMap::new() })
    }

    fn event(&self, asset: &fx::SoundEventAsset) -> Result<TimedEvent, String> {
        let event = asset.resolve(&self.catalog)?.map(|event| event.id);
        Ok(TimedEvent { delay: asset.delay, event })
    }

    fn resolve(&mut self, contact: &Contact) -> Result<Arc<Vec<Group>>, String> {
        use crate::{entities::scene_motion::child, gameplay::options::decl::list_items};
        for name in [&contact.source, &contact.target] {
            if !self.chains.contains_key(name) {
                self.chains.insert(name.clone(), material_chain(name, |path| text(&self.catalog, path))?);
            }
        }
        let Some(effect) = self.database.effect(&self.chains[&contact.source], &self.chains[&contact.target]) else {
            return Ok(Arc::new(Vec::new()));
        };
        if let Some(groups) = self.effects.get(effect) { return Ok(groups.clone()); }
        let effect = effect.to_owned();
        let fields = resolved_decl("fx", &effect, |path| text(&self.catalog, path))?;
        let mut groups = Vec::new();
        for item in list_items(child(&fields, "m_groups")) {
            let Value::Scope(fields) = item else { return Err(format!("{effect}: invalid FX group")) };
            let group = fx::effect_group(fields)?;
            if !group.enabled { continue; }
            let mut sounds = Vec::new();
            for sound in group.sounds {
                if !sound.enabled || (sound.start.event.is_none() && sound.stop.event.is_none()) { continue; }
                if sound.stop.event.is_some() {
                    return Err(format!("{effect}: contact FX requires an owner for its authored stop event"));
                }
                sounds.push(Sound { position: sound.position, start: self.event(&sound.start)? });
            }
            groups.push(Group { location: group.location, timing: group.timings, sounds });
        }
        let groups = Arc::new(groups);
        self.effects.insert(effect, groups.clone());
        Ok(groups)
    }
}

struct Active {
    sound: SoundEffect,
    emitter: Emitter,
    age: f32,
    origin: Vec3,
    axis: Mat3,
    parameter: (u32, f32),
}

#[derive(Resource)]
pub(super) struct ContactSounds {
    definitions: Result<Definitions, String>,
    active: Vec<Active>,
    seed: u32,
    reported: HashSet<String>,
}
impl FromWorld for ContactSounds {
    fn from_world(_: &mut World) -> Self {
        Self { definitions: Definitions::load(), active: Vec::new(), seed: 0, reported: HashSet::new() }
    }
}

pub(super) fn update(mut state: ResMut<ContactSounds>, mut contacts: MessageReader<ContactSound>,
    objects: Res<super::objects::EffectObjects>,
    mix: Res<super::AudioMix>,
    mut output: MessageWriter<SoundEvent>, clock: Res<ClockRuntime>, paused: Res<GamePaused>, playing: Query<&AudioTarget>,
    mut physics: Option<ResMut<crate::app::physics_scene::PhysicsScene>>)
{
    if paused.0 || !clock.advanced() { return; }
    let Some(energy) = mix.physics_scale else { return };
    let Ok(clock) = clock.clock() else { return };
    let dt = clock.delta(3).unwrap_or(0.0);
    let state = &mut *state;
    let emitters = playing.iter().map(|target| target.emitter).collect::<HashSet<_>>();
    state.active.retain(|active| active.sound.is_alive(emitters.contains(&active.emitter)));
    for active in &mut state.active { active.age += dt; }
    for ContactSound(contact) in contacts.read() {
        let prepared = (|| {
            let definitions = state.definitions.as_mut().map_err(|error| error.clone())?;
            let groups = definitions.resolve(contact)?;
            let value = energy.clamp(contact.speed * contact.speed * contact.mass);
            Ok::<_, String>((groups, (energy.id, value)))
        })();
        match prepared {
            Ok((groups, parameter)) => {
                let mut in_water = None;
                for group in groups.iter() {
                    if group.location != fx::EffectGroupLocation::Everywhere {
                        let water = in_water.get_or_insert_with(|| physics.as_mut()
                            .ok_or_else(|| "contact FX water query has no physics world".to_owned())
                            .and_then(|physics| physics.water_at(contact.position.to_array())));
                        match water {
                            Ok(water) if crate::gameplay::effects::groups::location_allowed(group.location, *water) => {},
                            Ok(_) => continue,
                            Err(error) => {
                                if state.reported.insert(error.clone()) { warn!("Contact sound unavailable: {error}"); }
                                continue;
                            }
                        }
                    }
                    let offsets = sample_offsets(&group.timing, 0.0, &mut state.seed);
                    for sound in &group.sounds {
                        let position = sample_position(sound.position, &mut state.seed);
                        let runtime = SoundEffect::new(position, sound.start, TimedEvent { delay: [0.0; 2], event: None }, offsets[0], &mut state.seed);
                        let emitter = match objects.allocate() {
                            Ok(emitter) => emitter,
                            Err(error) => { error!("Contact sound allocation failed: {error}"); continue; }
                        };
                        state.active.push(Active { sound: runtime, emitter, age: 0.0,
                            origin: contact.position, axis: crate::gameplay::reference_frame::direction_basis(contact.normal), parameter });
                    }
                }
            }
            Err(error) if state.reported.insert(error.clone()) => warn!("Contact sound unavailable: {error}"),
            Err(_) => {}
        }
    }
    for active in &mut state.active {
        let update = active.sound.update(active.age, active.origin, active.axis);
        if let Some(event) = update.event {
            output.write(SoundEvent(Request::PostAt { event, emitter: active.emitter, position: update.position,
                front: None, parameter: Some(active.parameter) }));
        }
    }
}
