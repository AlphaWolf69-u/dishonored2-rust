//! Real module constructors584970/5865e0/587020/584f30/3eb8f0/406120.
//! Numeric rows are allocated only when a real role/spot is admitted.
use crate::{
    assets::ai_world::crew_modules::Definition,
    gameplay::{
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
        pickable_component::CheckedComponent,
    },
};
use std::sync::Arc;
pub struct Assignment {
    pub member: EntityId,
    pub spot: Option<CheckedComponent>,
}
pub struct Spots {
    pub available: Vec<CheckedComponent>,
    pub assigned: Vec<Assignment>,
    pub archived: Vec<Assignment>,
}
impl Spots {
    fn new() -> Self {
        Self {
            available: Vec::new(),
            assigned: Vec::new(),
            archived: Vec::new(),
        }
    }
    pub fn stop(&mut self) {
        self.available.clear();
        self.available.shrink_to_fit();
        self.assigned.clear();
        self.assigned.shrink_to_fit();
        self.archived.clear();
        self.archived.shrink_to_fit();
    }
}
pub enum State {
    Chase {
        herd: Option<CheckedComponent>,
        candidates: Vec<EntityId>,
        previous: Vec<EntityId>,
        running: bool,
    },
    Melee {
        primary: Option<EntityId>,
        members: Vec<EntityId>,
    },
    Search,
    Link(Spots),
    Ranged,
    CombatFake,
    Flank {
        members: Vec<EntityId>,
    },
    Hideout {
        spots: Spots,
        distances: Vec<(EntityId, f32)>,
        selected: i32,
        target: Option<EntityId>,
    },
    Tether {
        members: Vec<EntityId>,
        maximum: i32,
    },
    Alarm {
        target: Option<EntityId>,
        binding: Option<CheckedComponent>,
        phase: i32,
        assignments: Vec<Assignment>,
    },
}
pub struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub core: Option<CheckedComponent>,
    pub coordination: Option<CheckedComponent>,
    pub knowledge: Option<CheckedComponent>,
    pub flags: u32,
    pub state: State,
    pub initialized: bool,
    pub started: bool,
}
impl Component {
    pub fn new(definition: Arc<Definition>) -> Self {
        let mut owner = ComponentOwner::default();
        owner.request_update(1);
        let state = match &*definition {
            Definition::Chase => State::Chase {
                herd: None,
                candidates: Vec::new(),
                previous: Vec::new(),
                running: false,
            },
            Definition::Melee { .. } => State::Melee {
                primary: None,
                members: Vec::new(),
            },
            Definition::Search => State::Search,
            Definition::Link => State::Link(Spots::new()),
            Definition::Ranged { .. } => State::Ranged,
            Definition::CombatFake => State::CombatFake,
            Definition::Flank => State::Flank {
                members: Vec::new(),
            },
            Definition::Hideout => State::Hideout {
                spots: Spots::new(),
                distances: Vec::new(),
                selected: -1,
                target: None,
            },
            Definition::Tether => State::Tether {
                members: Vec::new(),
                maximum: 0,
            },
            Definition::Alarm => State::Alarm {
                target: None,
                binding: None,
                phase: 0,
                assignments: Vec::new(),
            },
        };
        Self {
            owner,
            definition,
            core: None,
            coordination: None,
            knowledge: None,
            flags: 0,
            state,
            initialized: false,
            started: false,
        }
    }
    pub fn deinitialize(&mut self) {
        self.core = None;
        self.coordination = None;
        self.knowledge = None;
        if let State::Chase { herd, .. } = &mut self.state {
            *herd = None;
        }
        self.initialized = false;
    }
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
