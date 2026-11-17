//! World FX resources shared with the renderer. Graph playback owns activation;
//! prepared level declarations alone never create visible particle instances.
use crate::gameplay::reference_frame::{self, ReferenceFrame};
use crate::rendering::particles::world_sources::WorldFxSources;
use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use hashbrown::{HashMap, HashSet};
use std::sync::Arc;

pub(crate) fn trace_lifecycle() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("DH2_DEBUG_FX_LIFECYCLE").as_deref() == Ok("1"))
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldEffectDefinitions {
    pub graphs: Arc<Vec<Arc<crate::assets::entities::effects::FxGraph>>>,
    pub actors: Arc<Vec<crate::assets::entities::effects::actors::ActorDefinition>>,
    pub seed: u64,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct WorldEffectSources(pub Arc<WorldFxSources>);

/// Runtime-owned effects published as a snapshot. Each revision represents a
/// start event; removing an entry requests a stop but does not destroy its
/// still-live particle or sound tails.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct DynamicEffects(pub Arc<Vec<DynamicEffect>>);

///7ee610's instance metadata, owned through the actual FX tail lifetime.
#[derive(Clone, Copy)]
pub struct EntityEffectContext {
    pub owner: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub culprit: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub cannot_incapacitate_player_and_allies: bool,
    pub player_impact: bool,
    pub velocities: [[f32; 3]; 2],
    ///60a480/60a8b0 request+f0/f4 -> FXparams+b8/bc.22fd00 uses
    ///speed squared times mass for the existing physics-energy sound RTPC.
    pub contact_speed:f32,
    pub contact_mass:f32,
    ///60a480 receives8520d0's SOURCE body flag19 to install the
    ///439480 contact-stimulus completion callback; it is not a clock.
    pub contact_stimulus_enabled:Option<bool>,
}
#[derive(Clone)]
pub struct DynamicEffect {
    pub entity_context: Option<EntityEffectContext>,
    pub owner: String,
    pub revision: u64,
    pub effect: String,
    pub target: crate::gameplay::event_graph::EntityReference,
    pub tag: String,
    pub local: ReferenceFrame,
    pub attached: bool,
    /// Detached explicit-world starts retain their supplied pose through
    /// asynchronous preparation, without resolving a moving parent later.
    pub initial_world: Option<ReferenceFrame>,
    pub explicit_target: Option<crate::gameplay::particles::beam::BeamTarget>,
    /// Keeps sub-effect dependencies alive through stopped particle tails.
    pub sources: Arc<WorldFxSources>,
    /// The component's acknowledgement remains live through real render,
    /// particle and sound tails; source caches never retain this lease.
    pub retirement: Arc<DynamicEffectLifetime>,
}

/// A one-shot acknowledges completion only after the real instance releases
/// all particle, sound, decal and sub-effect chains. Persistent actors keep
/// their existing dormant/restart semantics.
pub struct DynamicEffectLifetime {
    pub once: bool,
    pub completed: std::sync::atomic::AtomicBool,
}
impl DynamicEffectLifetime {
    pub fn new(once: bool) -> Self { Self { once, completed: std::sync::atomic::AtomicBool::new(false) } }
}

/// Request-scoped GPU material preparation. A completion acknowledges only
/// the matching generation, so removed/replaced requests cannot start later.
#[derive(Resource, Clone, ExtractResource)]
pub struct DynamicMaterialPrewarm {
    pub generation: u64,
    pub retirement: u64,
    pub sources: Arc<WorldFxSources>,
    pub completed: Arc<std::sync::atomic::AtomicU64>,
    pub failed: Arc<std::sync::atomic::AtomicU64>,
    pub retired_signal: Arc<std::sync::atomic::AtomicU64>,
}
impl Default for DynamicMaterialPrewarm {
    fn default() -> Self {
        Self { generation: 0, retirement: 0, sources: Arc::default(),
            completed: Arc::default(), failed: Arc::default(), retired_signal: Arc::default() }
    }
}

struct EntityFrame {
    spawn: ReferenceFrame,
    parent: Option<FrameBinding>,
}

struct FrameBinding {
    entity: String,
    tag: Option<String>,
    local: Option<([f32; 3], [f32; 4])>,
}

#[derive(Clone, Default)]
struct EphemeralFrame {
    root: Option<(ReferenceFrame, u64)>,
    tags: HashMap<String, (ReferenceFrame, u64)>,
    revision: u64,
}

/// Pose publication includes non-rendered FX targets. This is not a second
/// physics world: moving owners publish the same pose used by their geometry.
#[derive(Resource, Default, ExtractResource)]
pub struct WorldEffectFrames {
    entities: Arc<HashMap<String, EntityFrame>>,
    published: HashMap<String, ReferenceFrame>,
    ephemeral: HashMap<String, EphemeralFrame>,
    ephemeral_revision: u64,
    pub clock: Option<WorldFxClock>,
    player: Option<(ReferenceFrame, Arc<HashMap<String, ReferenceFrame>>)>,
}

impl Clone for WorldEffectFrames {
    fn clone(&self) -> Self {
        let trace = trace_lifecycle();
        let cycles = if trace { crate::loading::thread_cycles() } else { None };
        let copy = Self {
            entities: self.entities.clone(),
            published: self.published.clone(),
            ephemeral: self.ephemeral.clone(),
            ephemeral_revision: self.ephemeral_revision,
            clock: self.clock,
            player: self.player.clone(),
        };
        if trace {
            let elapsed = cycles.zip(crate::loading::thread_cycles())
                .map(|(start, end)| end.saturating_sub(start));
            let names_bytes: usize = self.published.keys().map(String::len).sum();
            let entries_bytes = self.published.len() * std::mem::size_of::<(String, ReferenceFrame)>();
            eprintln!("FX_LIFECYCLE frame_clone poses={} entry_payload_bytes={entries_bytes} name_bytes={names_bytes} cpu_cycles={elapsed:?} pose_signature={:016x}",
                self.published.len(), self.debug_pose_signature());
        }
        copy
    }
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
    /// Order-independent diagnostic of the published moving-entity poses.
    pub(crate) fn debug_pose_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        self.published.iter().fold(0, |signature, (name, pose)| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            name.hash(&mut hasher);
            for value in pose.position.iter().chain(pose.basis.iter().flatten()) {
                value.to_bits().hash(&mut hasher);
            }
            signature ^ hasher.finish()
        })
    }
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
        for binding in crate::assets::entities::resolved_entity_binds(document) {
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
            ephemeral: HashMap::new(),
            ephemeral_revision: 0,
            clock: None,
            player: None,
        })
    }

    pub fn publish_player(&mut self, frame: Option<(ReferenceFrame, HashMap<String, ReferenceFrame>)>) {
        self.player = frame.map(|(root,tags)|(root,Arc::new(tags)));
    }

    /// Component-owned temporary entities publish their actual live poses.
    /// Reuse names and storage while active; the component's retirement
    /// removes its publication without altering installed entity frames.
    pub fn publish_ephemeral<'a>(&mut self, frames: impl Clone + Iterator<Item = (&'a str, ReferenceFrame)>) {
        self.begin_ephemeral();
        for (name, frame) in frames {
            self.publish_ephemeral_frame(name, "", frame);
        }
        self.finish_ephemeral();
    }

    pub fn begin_ephemeral(&mut self) { self.ephemeral_revision = self.ephemeral_revision.wrapping_add(1); }

    /// Publish only actual demanded entity/tag poses into the existing host
    /// snapshot. Rigs, palettes, definitions and component identities stay with
    /// their canonical owners; unchanged names/tag keys allocate nothing.
    pub fn publish_ephemeral_frame(&mut self, name: &str, tag: &str, frame: ReferenceFrame) {
        if !self.ephemeral.contains_key(name) { self.ephemeral.insert(name.to_owned(), EphemeralFrame::default()); }
        let entry = self.ephemeral.get_mut(name).expect("inserted FX frame");
        entry.revision = self.ephemeral_revision;
        if tag.is_empty() { entry.root = Some((frame, self.ephemeral_revision)); }
        else if let Some(value) = entry.tags.get_mut(tag) { *value = (frame, self.ephemeral_revision); }
        else { entry.tags.insert(tag.to_owned(), (frame, self.ephemeral_revision)); }
    }

    pub fn finish_ephemeral(&mut self) {
        let revision = self.ephemeral_revision;
        self.ephemeral.retain(|_, entry| {
            if entry.revision != revision { return false; }
            if entry.root.is_some_and(|(_, previous)| previous != revision) { entry.root = None; }
            entry.tags.retain(|_, (_, previous)| *previous == revision);
            true
        });
    }

    pub fn has_authored_entity(&self, name: &str) -> bool { self.entities.contains_key(name) }

    pub fn ephemeral_current(&self, name: &str, tag: &str) -> bool {
        let Some(entry) = self.ephemeral.get(name) else { return false; };
        if tag.is_empty() { entry.root.is_some_and(|(_, revision)| revision == self.ephemeral_revision) }
        else { entry.tags.get(tag).is_some_and(|(_, revision)| *revision == self.ephemeral_revision) }
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
            EntityReference::Named(name) => match self.ephemeral.get(name) {
                Some(entry) => entry.tags.get(tag).map(|(frame, _)| Some(*frame))
                    .ok_or_else(|| format!("FX tag/joint {name}/{tag} is absent from its actual published model")),
                None if !self.entities.contains_key(name) => Ok(None),
                None => Err(format!("FX tag/joint {name}/{tag} awaits its animated model pose")),
            },
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

    /// Borrow the ordinary unbound/published pose without allocating a bind
    /// traversal. Bound entities still use the one existing resolver below.
    pub fn published_frame(&self, entity:&str)->Result<Option<ReferenceFrame>,String>{
        if let Some(frame)=self.published.get(entity){return Ok(Some(*frame));}
        if let Some(frame)=self.ephemeral.get(entity).and_then(|entry| entry.root){return Ok(Some(frame.0));}
        let Some(entry)=self.entities.get(entity) else{return Ok(None);};
        if entry.parent.is_none(){return Ok(Some(entry.spawn));}
        self.frame(entity)
    }
    pub fn frame(&self, entity: &str) -> Result<Option<ReferenceFrame>, String> {
        if !self.entities.contains_key(entity) { return Ok(self.ephemeral.get(entity).and_then(|entry| entry.root.map(|value| value.0))); }
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
