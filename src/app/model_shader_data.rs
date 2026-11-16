//! Per-model shader fields written by87ce70/87d050 and234ba0/236550.
//! Resources and SH storage remain with ModelProbeLighting and actual models.
use super::*;
use crate::gameplay::{
    entity_lifetime::{EntityId, LiveEntities},
    game_clock_runtime::GameplayViewClock,
};
use bevy::prelude::*;
#[derive(Clone, Copy, Default)]
struct State {
    clock: Option<(u8, f32)>,
    flags: u32,
}
#[derive(Resource, Default)]
pub(in crate::app) struct ModelShaderData {
    models: HashMap<ModelKey, State>,
}
impl ModelShaderData {
    pub(in crate::app) fn remove(&mut self, key: ModelKey) {
        self.models.remove(&key);
    }
    pub(in crate::app) fn flags(&self, key: ModelKey) -> u32 {
        self.models.get(&key).map_or(0, |state| state.flags)
    }
    pub(in crate::app) fn clock(&self, key: ModelKey) -> (u8, f32) {
        self.models
            .get(&key)
            .and_then(|state| state.clock)
            .unwrap_or((0, 0.))
    }
}
fn keys(world: &World, live: &LiveEntities, id: EntityId) -> Result<Vec<ModelKey>, String> {
    let (name, _) = live
        .get(id)
        .ok_or("shader fields targeted a stale entity generation")?;
    let mut keys = world
        .get_resource::<super::super::RenderSourcePlacements>()
        .map(|placements| {
            placements
                .0
                .iter()
                .enumerate()
                .filter_map(|(index, placement)| {
                    placement
                        .id
                        .eq_ignore_ascii_case(name)
                        .then_some(ModelKey::Placed(index))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if super::super::world_models::frame(world, id).is_some()
        || world
            .get_resource::<super::super::player_arms::PlayerArms>()
            .is_some_and(|arms| arms.has_managed_model(id))
    {
        keys.push(ModelKey::Actor(id));
    }
    Ok(keys)
}
pub(in crate::app) fn set_shader_clock(
    world: &mut World,
    id: EntityId,
    group: u8,
    offset: f32,
) -> Result<(), String> {
    if group >= 5 || !offset.is_finite() {
        return Err("invalid source model shader clock".into());
    }
    let keys = keys(
        world,
        world
            .get_resource::<LiveEntities>()
            .ok_or("model shader live owner absent")?,
        id,
    )?;
    // The source skips actors without a render-model pointer.
    if !keys.is_empty() {
        let mut data = world.resource_mut::<ModelShaderData>();
        for key in keys {
            data.models.entry(key).or_default().clock = Some((group, offset));
        }
    }
    Ok(())
}
pub(in crate::app) fn set_in_water(
    world: &mut World,
    live: &LiveEntities,
    root: EntityId,
    entered: bool,
) -> Result<(), String> {
    let mut stack = vec![root];
    let mut visited = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            return Err("cyclic model water-state binding".into());
        }
        let keys = keys(world, live, id)?;
        let mut data = world.resource_mut::<ModelShaderData>();
        for key in keys {
            let flags = &mut data.models.entry(key).or_default().flags;
            if entered {
                crate::render_model::request_flags(flags, 2);
            } else {
                crate::render_model::clear_flags(flags, 2);
            }
        }
        drop(data);
        stack.extend_from_slice(live.children(id)?);
    }
    Ok(())
}
pub(in crate::app) fn publish_shader_clocks(
    mut data: ResMut<ModelShaderData>,
    view: Res<GameplayViewClock>,
    queue: Option<Res<super::super::SceneRenderQueue>>,
    worlds: Option<Res<super::super::world_models::WorldModels>>,
    bank: Option<ResMut<ModelProbeLighting>>,
    sources: Option<Res<super::super::ShadowModelBoundsSources>>,
    equipment: Option<Res<super::super::player_arms::PlayerArms>>,
    live: Option<Res<LiveEntities>>,
    mut patches: ResMut<ShaderBufferPatches>,
    mut commands: Commands,
) {
    let Some(mut bank) = bank else { return };
    if let Some(live) = live.as_deref() {
        if data
            .models
            .keys()
            .any(|key| matches!(key,ModelKey::Actor(id) if live.get(*id).is_none()))
        {
            data.models
                .retain(|key, _| !matches!(key,ModelKey::Actor(id) if live.get(*id).is_none()));
        }
    }
    let result = (|| -> Result<(), String> {
        bank.publish_default_clock(&mut patches, view.model_local_time(0., 0)?);
        if let (Some(sources), Some(queue)) = (sources.as_deref(), queue.as_deref()) {
            if data
                .models
                .iter()
                .any(|(key, state)| matches!(key, ModelKey::Placed(_)) && state.clock.is_some())
            {
                for source in &sources.0 {
                    let Some((group, offset)) = data
                        .models
                        .get(&ModelKey::Placed(source.placement))
                        .and_then(|state| state.clock)
                    else {
                        continue;
                    };
                    let time = view.model_local_time(offset, group << 4)?;
                    for instance in queue
                        .instances
                        .get(source.instances.clone())
                        .ok_or("model clock source draw range invalid")?
                    {
                        if instance.probe_index & 0x80000000 == 0 {
                            bank.publish_clock(&mut patches, instance.probe_index, time);
                        }
                    }
                }
            }
        }
        for (&key, state) in &data.models {
            let (ModelKey::Actor(id), Some((group, offset))) = (key, state.clock) else {
                continue;
            };
            let time = view.model_local_time(offset, group << 4)?;
            if let Some(worlds) = worlds.as_deref() {
                worlds.publish_clock(id, &mut bank, &mut patches, time);
            }
            if let Some(equipment) = equipment.as_deref() {
                equipment.publish_shader_clock(id, &mut bank, &mut patches, time);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        error!("Model shader clock publication failed: {error}");
        commands.write_message(AppExit::error());
    }
}
