//! World FX resources shared with the renderer. Graph playback owns activation;
//! prepared level declarations alone never create visible particle instances.
use crate::gameplay::reference_frame::{self, ReferenceFrame};
use crate::rendering::particles::world_sources::WorldFxSources;
use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Resource)]
pub struct WorldEffectGraphs(pub Vec<crate::assets::entities::effects::FxGraph>);

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldEffectSources(pub Arc<WorldFxSources>);

struct EntityFrame {
    spawn: ReferenceFrame,
    published: Option<ReferenceFrame>,
    parent: Option<FrameBinding>,
}

struct FrameBinding {
    entity: String,
    tag: Option<String>,
    local: Option<([f32; 3], [f32; 4])>,
}

/// Pose publication includes non-rendered FX targets. This is not a second
/// physics world: moving owners publish the same pose used by their geometry.
#[derive(Resource, Default)]
pub struct WorldEffectFrames {
    entities: HashMap<String, EntityFrame>,
}

impl WorldEffectFrames {
    pub fn load(document: Option<&crate::assets::entities::Document>) -> Result<Self, String> {
        let Some(document) = document else {
            return Ok(Self::default());
        };
        let mut entities = document
            .definitions
            .iter()
            .filter(|d| d.kind == "entity")
            .map(|entity| {
                Ok((
                    entity.id.clone(),
                    EntityFrame {
                        spawn: crate::assets::entities::spawn_frame(entity)?,
                        published: None,
                        parent: None,
                    },
                ))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        for binding in crate::assets::entities::entity_binds(document) {
            let parent = entities.get(&binding.parent).map(|entry| entry.spawn);
            if let Some(child) = entities.get_mut(&binding.entity_id) {
                child.parent = Some(FrameBinding {
                    entity: binding.parent,
                    tag: binding.tag,
                    local: parent
                        .map(|parent| reference_frame::relative_binding(parent, child.spawn)),
                });
            }
        }
        Ok(Self { entities })
    }

    pub fn publish(&mut self, entity: &str, frame: ReferenceFrame) {
        if let Some(entry) = self.entities.get_mut(entity) {
            entry.published = Some(frame);
        }
    }

    pub fn frame(&self, entity: &str) -> Result<Option<ReferenceFrame>, String> {
        let mut current = entity;
        let mut visited = HashSet::new();
        let mut chain = Vec::new();
        let mut frame = loop {
            let Some(entry) = self.entities.get(current) else {
                return Ok(None);
            };
            if !visited.insert(current) {
                return Err(format!("FX reference-frame cycle at {current}"));
            }
            if let Some(frame) = entry.published {
                break frame;
            }
            let Some(binding) = &entry.parent else {
                break entry.spawn;
            };
            if let Some(tag) = binding.tag.as_deref().filter(|tag| !tag.is_empty()) {
                return Err(format!(
                    "FX reference {current} requires actual parent tag {}/{tag}",
                    binding.entity
                ));
            }
            chain.push(binding.local.ok_or_else(|| {
                format!(
                    "FX reference {current} has unresolved parent {}",
                    binding.entity
                )
            })?);
            current = &binding.entity;
        };
        for (offset, orientation) in chain.into_iter().rev() {
            frame = reference_frame::bind(frame, offset, orientation);
        }
        Ok(Some(frame))
    }
}
