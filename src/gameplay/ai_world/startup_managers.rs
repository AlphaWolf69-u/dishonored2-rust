//! Typed AIWorld startup payloads. No absent-NPC result replaces Init effects.
use crate::{
    assets::ai_world::startup_managers::Definition,
    gameplay::{
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
        sound::bark_history::BarkRecord,
    },
};
use std::sync::Arc;

pub struct CuePool {
    pub children: Vec<EntityId>,
    pub used: usize,
    ///bevy_backend: resume synchronous Init after a fallible host callback.
    pub configured:usize,
}
pub struct Cues {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub pools: [CuePool; 25],
    pub initialized: bool,
    pub started: bool,
    ///bevy_backend: acknowledgement cursor within synchronous458b30.
    pub retire_cursor:[usize;2],
    pub retire_phase:u8,
}
impl Cues {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Cues(_)) {
            return Err("Cues payload received another declaration class".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            pools: std::array::from_fn(|_| CuePool {
                children: Vec::new(),
                used: 0,
                configured:0,
            }),
            initialized: false,
            started: false,
            retire_cursor:[0;2],
            retire_phase:0,
        })
    }
    pub fn validate_children(&self) -> Result<(), String> {
        let Definition::Cues(definition) = &*self.definition else {
            unreachable!()
        };
        if self
            .pools
            .iter()
            .zip(&definition.pools)
            .any(|(children, pool)| children.children.len() != pool.maximum)
        {
            return Err("Cues Initialize requires actual prepared child actors".into());
        }
        Ok(())
    }
}
pub struct CrewDispatch {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub children: Vec<EntityId>,
    ///bevy_backend: the child actor exists before configuration can fail.
    pub configured:usize,
    ///bevy_backend: retain source5780e0's reflected-removal progress.
    pub retire_cursor:usize,
    pub requester: Option<EntityId>,
    pub request_next: i64,
    pub request_active: bool,
    pub initialized: bool,
    pub started: bool,
}
impl CrewDispatch {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Crew(_)) {
            return Err("Crew payload received another declaration class".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            children: Vec::new(),
            configured:0,
            retire_cursor:0,
            requester: None,
            request_next: -1,
            request_active: false,
            initialized: false,
            started: false,
        })
    }
}
/// Actual admitted graph lease supplied by the navigation-resource owner.
/// The initializer must not interpret an unavailable graph as an empty graph.
pub struct Dsm {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub(crate) graph: Option<Arc<crate::assets::physworld::navmesh::Graph>>,
    pub cell_access: std::collections::HashMap<[u16; 3], u16>,
    pub search_values: Vec<u8>,
    pub static_values: Vec<u8>,
    pub chase_values: Vec<u8>,
    pub valid: Vec<u32>,
    pub touched: Vec<u32>,
    pub sensor_values: [Vec<u8>; 8],
    pub areas: Vec<Option<EntityId>>,
    pub last_search_time: f64,
    pub last_regeneration_time: f64,
    pub sensor_times: [f64; 8],
    pub navigation_listener: bool,
    pub initialized: bool,
    pub started: bool,
}
impl Dsm {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Dsm(_)) {
            return Err("DSM payload received another declaration class".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            graph: None,
            cell_access: std::collections::HashMap::new(),
            search_values: Vec::new(),
            static_values: Vec::new(),
            chase_values: Vec::new(),
            valid: Vec::new(),
            touched: Vec::new(),
            sensor_values: std::array::from_fn(|_| Vec::new()),
            areas: Vec::new(),
            last_search_time: -1.,
            last_regeneration_time: -1.,
            sensor_times: [-1.; 8],
            navigation_listener: false,
            initialized: false,
            started: false,
        })
    }
}
/// Host composition of the actual admitted hkaiWorld instance and its DSM
/// listener array(df6e80/df6ee0). The source listener's callbacks are allnoops;
/// its retained registration/lifetime still belong to the navigation owner.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub(crate) struct NavigationInstances {
    metadata: Arc<crate::assets::physworld::navmesh::Metadata>,
    listeners: Vec<crate::gameplay::pickable_component::CheckedComponent>,
}
impl NavigationInstances {
    pub(crate) fn new(metadata: Arc<crate::assets::physworld::navmesh::Metadata>) -> Self {
        Self {
            metadata,
            listeners: Vec::new(),
        }
    }
    pub(crate) fn graph(
        &self,
        area: u16,
    ) -> Result<Arc<crate::assets::physworld::navmesh::Graph>, String> {
        self.metadata.dsm_graph_for(0, area)
    }
    pub(crate) fn register(
        &mut self,
        value: crate::gameplay::pickable_component::CheckedComponent,
    ) -> Result<(), String> {
        if self.listeners.contains(&value) {
            return Err("DSM navigation listener registered twice".into());
        }
        self.listeners.push(value);
        Ok(())
    }
    pub(crate) fn unregister(
        &mut self,
        value: crate::gameplay::pickable_component::CheckedComponent,
    ) -> Result<(), String> {
        let index = self
            .listeners
            .iter()
            .position(|candidate| *candidate == value)
            .ok_or("DSM navigation listener registration disappeared")?;
        self.listeners.remove(index);
        Ok(())
    }
    pub(crate) fn is_clear(&self) -> bool {
        self.listeners.is_empty()
    }
}
#[derive(Clone, Copy)]
pub struct FleeEvent {
    pub entity: EntityId,
    pub position: [f32; 3],
    pub time: f64,
    pub time_domain: i32,
    pub valid: bool,
}
pub struct Flee {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub elapsed: f32,
    pub events: Vec<FleeEvent>,
    pub initialized: bool,
    pub started: bool,
}
impl Flee {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Flee(_)) {
            return Err("Flee payload received another declaration class".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            elapsed: f32::from_bits(0x3e99999a),
            events: Vec::new(),
            initialized: false,
            started: false,
        })
    }
}
///417740's32-byte row: cue managed identity, NPC managed-ID list and word+18.
pub struct CuePerception {pub cue:EntityId,pub perceivers:Vec<EntityId>,pub midnight_handle:u32}
pub struct Coordination {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub barks: Vec<BarkRecord>,
    pub bark_written: bool,
    pub cue_perception:Vec<CuePerception>,
    pub cue_perception_dirty:bool,
    pub initialized: bool,
    pub started: bool,
}
impl Coordination {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Coordination(_)) {
            return Err("Coordination payload received another declaration class".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            barks: Vec::new(),
            bark_written: false,
            cue_perception:Vec::new(),
            cue_perception_dirty:false,
            initialized: false,
            started: false,
        })
    }
    ///417140 resets an uncommitted list before the writer receives it.
    pub fn begin_bark_update(&mut self) -> &mut Vec<BarkRecord> {
        if !self.bark_written {
            self.barks.clear()
        }
        &mut self.barks
    }
    ///4178a0 commits the actual BB+120 flag before releasing its write lock.
    pub fn finish_bark_update(&mut self) {
        self.bark_written = !self.barks.is_empty();
    }
    pub fn reset(&mut self) {
        self.cue_perception=Vec::new();
        self.cue_perception_dirty=false;
        self.barks.clear();
        self.bark_written = false;
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservableRef {
    pub entity: EntityId,
    pub component: crate::gameplay::pickable_component::CheckedComponent,
}
pub struct VisionBudget {
    ///607570's three actual words at+0/+4/+8. Evaluation owns their use.
    pub counters: [u32;3],
    pub enabled: bool,
}
pub struct Vision {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub rosters: [Vec<ObservableRef>; 8],
    pub budgets: [VisionBudget; 8],
    pub initialized: bool,
    pub started: bool,
}
impl Vision {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Vision(_)) {
            return Err("Vision payload received another declaration class".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            rosters: std::array::from_fn(|_| Vec::new()),
            budgets: std::array::from_fn(|_| VisionBudget {
                counters: [0,30,0],
                enabled: true,
            }),
            initialized: false,
            started: false,
        })
    }
}
macro_rules! component {($($name:ty),*)=>{$(impl ComponentInstance for $name{
    fn owner(&self)->&ComponentOwner{&self.owner}
    fn owner_mut(&mut self)->&mut ComponentOwner{&mut self.owner}
})*}}
component!(Cues, CrewDispatch, Dsm, Flee, Coordination, Vision);
