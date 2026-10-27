//! World FX resources shared with the renderer. Graph playback owns activation;
//! prepared level declarations alone never create visible particle instances.
use crate::gameplay::reference_frame::{self, ReferenceFrame};
use crate::rendering::particles::world_sources::WorldFxSources;
use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldEffectDefinitions {
    pub graphs: Arc<Vec<crate::assets::entities::effects::FxGraph>>,
    pub actors: Arc<Vec<crate::assets::entities::effects::actors::ActorDefinition>>,
    pub seed: u64,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldEffectSources(pub Arc<WorldFxSources>);

struct EntityFrame {
    spawn: ReferenceFrame,
    parent: Option<FrameBinding>,
}

struct FrameBinding {
    entity: String,
    tag: Option<String>,
    local: Option<([f32; 3], [f32; 4])>,
}

/// Pose publication includes non-rendered FX targets. This is not a second
/// physics world: moving owners publish the same pose used by their geometry.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldEffectFrames {
    entities: Arc<HashMap<String, EntityFrame>>,
    published: HashMap<String, ReferenceFrame>,
    pub clock: Option<WorldFxClock>,
    player: Option<(ReferenceFrame, Arc<HashMap<String, ReferenceFrame>>)>,
}

#[derive(Clone, Copy)]
pub struct WorldFxClock {
    pub time: f64,
    pub delta: f32,
    pub scale: f32,
    /// Reference actors use the entity's clock, independently of FX group2.
    pub entity_delta: f32,
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
        Ok(Self {
            entities: Arc::new(entities),
            published: HashMap::new(),
            clock: None,
            player: None,
        })
    }

    pub fn publish_player(&mut self, frame: Option<(ReferenceFrame, HashMap<String, ReferenceFrame>)>) {
        self.player = frame.map(|(root,tags)|(root,Arc::new(tags)));
    }

    pub fn resolve(&self, entity: &crate::gameplay::event_graph::EntityReference, tag: &str) -> Result<Option<ReferenceFrame>, String> {
        use crate::gameplay::event_graph::EntityReference;
        match entity {
            EntityReference::Invalid => Ok(None),
            EntityReference::Player => match &self.player {
                None => Ok(None),
                Some((root, _)) if tag.is_empty() => Ok(Some(*root)),
                Some((_, tags)) => tags.get(tag).copied().map(Some)
                    .ok_or_else(|| format!("player FX tag/joint {tag} is absent")),
            },
            EntityReference::Named(name) if tag.is_empty() => self.frame(name),
            EntityReference::Named(name) => Err(format!("FX tag/joint {name}/{tag} awaits its animated model pose")),
        }
    }

    pub fn publish_delta(&mut self, entity: &str, delta: [[f32; 4]; 3]) {
        if let Some(entry) = self.entities.get(entity) {
            // A blueprint mesh can have an authored offset from its entity.
            // Apply the body's rigid delta to the ENTITY spawn frame, not to
            // whichever render placement last published this entity name.
            let delta = ReferenceFrame {
                position: std::array::from_fn(|axis| delta[axis][3]),
                basis: std::array::from_fn(|axis| {
                    std::array::from_fn(|component| delta[component][axis])
                }),
            };
            self.published.insert(
                entity.to_owned(),
                reference_frame::compose(delta, entry.spawn),
            );
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
            if let Some(frame) = self.published.get(current) {
                break *frame;
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
