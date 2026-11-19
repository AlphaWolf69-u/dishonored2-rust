//!Placed Pickable76e750 begin-pickup sound and Hold49/50 throw-FX demand.
//!Declarations stay in their component/player owners; preparation borrows the
//!same canonical Wwise/FX owners used by runtime prototypes and effects.
use super::carrying_host::CarryPhysics;
use super::*;
use crate::assets::wwise::SoundEventDeclaration;
use crate::gameplay::pickable_component::CheckedComponent;
use bevy::prelude::World;
struct SoundScope {
    player_owned: bool,
    key: Arc<str>,
    declaration: Option<Arc<SoundEventDeclaration>>,
    members: Vec<CheckedComponent>,
    reserved: bool,
}
#[derive(Default)]
pub(super) struct Resources {
    initialized: bool,
    ready: bool,
    closed: bool,
    sounds: Vec<SoundScope>,
    fx_key: Option<Arc<str>>,
    fx_paths: Option<Vec<String>>,
    fx_reserved: bool,
    error: Option<String>,
}
impl PhysicsScene {
    fn carrying_sound_demand(&self) -> Vec<SoundScope> {
        let mut sounds: Vec<SoundScope> = Vec::new();
        let mut indices = HashMap::new();
        for (class_id, pool) in [(self.classes[6], &self.pickables), (self.classes[61], &self.charm_pickables)] {
            for slot in 0..pool.roster.indices.len() {
                let Some(component) = pool.get_at(slot) else {
                    continue;
                };
                if component.owner.entity.is_none() {
                    continue;
                }
                let Some(sound) = component.definition.begin_pickup_sound.as_ref() else {
                    continue;
                };
                let identity = Arc::as_ptr(sound) as usize;
                let index = *indices.entry(identity).or_insert_with(|| {
                    let index = sounds.len();
                    sounds.push(SoundScope {
                        player_owned: false,
                        key: Arc::from(format!("carry_pickup:{identity:x}")),
                        declaration: Some(sound.clone()),
                        members: Vec::new(),
                        reserved: false,
                    });
                    index
                });
                sounds[index].members.push(CheckedComponent {
                    handle: ComponentHandle {
                        class_id,
                        slot: slot as u16,
                    },
                    generation: component.owner.generation,
                });
            }
        }
        sounds
    }
}
fn advance(world: &mut World, state: &mut Resources) -> Result<(), String> {
    if state.closed || state.ready {
        return Ok(());
    }
    if !state.initialized {
        let Some(player) = world.get_resource::<super::super::player::PlayerState>() else {
            return Ok(());
        };
        let Some(scene) = world.get_resource::<PhysicsScene>() else {
            return Ok(());
        };
        state.sounds = scene.carrying_sound_demand();
        if player.player.is_some() {
            let Some(actions) = world.get_resource::<super::super::player_actions::PlayerActions>()
            else {
                return Ok(());
            };
            for (mode, sound) in actions.loot_full_sounds.iter().enumerate() {
                if let Some(sound) = sound {
                    state.sounds.push(SoundScope {
                        player_owned: true,
                        key: Arc::from(format!("carry_inventory_full:{mode}")),
                        declaration: Some(sound.clone()),
                        members: Vec::new(),
                        reserved: false,
                    });
                }
            }
        }

        if let Some(player) = player.player.as_ref() {
            let paths = player.carrying_throw_fx_paths();
            if !paths.is_empty() {
                state.fx_key = Some(Arc::from("carry_throw_fx"));
                state.fx_paths = Some(paths);
            }
        }
        state.initialized = true;
    }
    for scope in &mut state.sounds {
        if !scope.reserved {
            let declaration = scope
                .declaration
                .as_ref()
                .ok_or("carry sound declaration lease absent")?
                .clone();
            super::super::audio::reserve_events(
                world,
                scope.key.clone(),
                std::iter::once(declaration),
            )?;
            scope.reserved = true;
            scope.declaration = None; //Canonical reservation owns the actual Arc.
        }
    }
    if let Some(key) = state.fx_key.as_ref() {
        if !state.fx_reserved {
            let mut requests = world
                .get_resource_mut::<super::super::usable_effects::EffectRequests>()
                .ok_or("carry canonical FX request owner absent")?;
            let paths = state.fx_paths.take().ok_or("carry FX demand absent")?;
            requests.reserve(key.clone(), paths);
            state.fx_reserved = true;
        }
        if !super::super::usable_effects::reservation_ready(world, key)? {
            return Ok(());
        }
    }
    for scope in &state.sounds {
        if !super::super::audio::events_ready(world, &scope.key)? {
            return Ok(());
        }
    }
    state.ready = true;
    Ok(())
}
pub(in crate::app) fn prepare(world: &mut World) {
    if world.get_resource::<CarryPhysics>().is_none_or(|carry| {
        carry.resources.ready || carry.resources.closed || carry.resources.error.is_some()
    }) {
        return;
    }
    let Some(mut carry) = world.remove_resource::<CarryPhysics>() else {
        return;
    };
    if carry.resources.error.is_none() {
        if let Err(error) = advance(world, &mut carry.resources) {
            carry.resources.error = Some(error);
        }
    }
    world.insert_resource(carry);
}
pub(in crate::app) fn startup_pending(world: &World) -> bool {
    world
        .get_resource::<CarryPhysics>()
        .is_some_and(|carry| !carry.resources.ready && !carry.resources.closed)
}
pub(in crate::app) fn startup_error(world: &World) -> Option<String> {
    world
        .get_resource::<CarryPhysics>()
        .and_then(|carry| carry.resources.error.clone())
}
///Called by the actual placed Pickable Deinitialize, before its generation
///or declaration drops. Other components sharing that declaration keep demand.
pub(in crate::app) fn release_pickable(
    world: &mut World,
    handle: ComponentHandle,
    generation: u32,
) -> Result<(), String> {
    let Some(mut carry) = world.remove_resource::<CarryPhysics>() else {
        return Ok(());
    };
    let result = (|| {
        let state = &mut carry.resources;
        let member = CheckedComponent { handle, generation };
        let mut cursor = 0;
        while cursor < state.sounds.len() {
            let scope = &mut state.sounds[cursor];
            scope.members.retain(|entry| *entry != member);
            if scope.members.is_empty() && !scope.player_owned {
                if scope.reserved {
                    super::super::audio::release_events(world, &scope.key)?;
                }
                state.sounds.swap_remove(cursor);
            } else {
                cursor += 1;
            }
        }
        Ok(())
    })();
    world.insert_resource(carry);
    result
}
///ProducerStop precedes this call. Live FX/voices retain their own tails;
///reservation withdrawal neither stops them early nor owns a second cache.
pub(in crate::app) fn retire(world: &mut World) -> Result<(), String> {
    let Some(mut carry) = world.remove_resource::<CarryPhysics>() else {
        return Ok(());
    };
    carry.resources.closed = true;
    let result = (|| {
        for scope in &mut carry.resources.sounds {
            if scope.reserved {
                super::super::audio::release_events(world, &scope.key)?;
                scope.reserved = false;
            }
        }
        carry.resources.sounds.clear();
        if carry.resources.fx_reserved {
            let key = carry
                .resources
                .fx_key
                .as_ref()
                .ok_or("retiring carry FX key absent")?
                .clone();
            world
                .get_resource_mut::<super::super::usable_effects::EffectRequests>()
                .ok_or("retiring carry FX owner absent")?
                .release_reservation(key);
            carry.resources.fx_reserved = false;
        }
        carry.resources.fx_paths = None;
        Ok(())
    })();
    world.insert_resource(carry);
    result
}
