//! Actual collector storage: constructors5699f0/57cd90/56d1f0/8e8990/3e89a0.
use crate::{
    assets::ai_world::crew_collectors::Definition,
    gameplay::{
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
        pickable_component::CheckedComponent,
    },
};
use std::sync::Arc;

///56d310 copies the two source words. Message decoding retains their exact
///bits; no process pointer is created from the opaque notification data.
pub struct Notification(pub [u8; 16]);
pub struct PathRequest {
    pub submitted: Option<u32>,
    pub path_points: Vec<[f32; 4]>,
    pub path_flags: u32,
    pub last_time: f64,
    pub target: Option<EntityId>,
}
pub struct Evaluator {
    pub index: i32,
    pub flags: u8,
    pub kind: Evaluation,
}
pub enum Evaluation {
    Distance {
        parameters: Option<([f32; 3], [f32; 3])>,
    },
    NavigationLos {
        target: Option<[f32; 4]>,
    },
    AngleToPositions {
        origin: [f32; 3],
        positions: Vec<[f32; 3]>,
    },
}
pub struct Formation {
    ///3eb5c0 borrows the actual Chase declaration's embedded settings+90.
    ///The shared declaration lease cannot outlive the source component by
    ///accident: Chase586a00 clears it before its own owner is removed.
    pub settings: Option<Arc<crate::assets::ai_world::crew_modules::Definition>>,
    pub origin: [f32;3],
    pub evaluators: Vec<Evaluator>,
    pub members: Vec<CheckedComponent>,
    pub destinations: Vec<[f32; 3]>,
    pub active: bool,
}
pub enum State {
    Area,
    Combat(PathRequest),
    Notification {
        pending: Vec<Notification>,
    },
    Search {
        searching: bool,
        subjects: Vec<EntityId>,
        candidates: Vec<EntityId>,
        dispatched: bool,
    },
    Herd(Formation),
}
pub struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub core: Option<CheckedComponent>,
    pub knowledge: Option<CheckedComponent>,
    pub state: State,
    pub initialized: bool,
    pub started: bool,
}
impl Component {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        let mut owner = ComponentOwner::default();
        let state = match &*definition {
            Definition::Area => State::Area,
            Definition::Combat(_) => State::Combat(PathRequest {
                submitted: None,
                path_points: Vec::new(),
                path_flags: 0x1ffff,
                last_time: -1.,
                target: None,
            }),
            Definition::Notification => {
                owner.request_update(1); //56d1f0 →7e7ab0, independently of declaration masks.
                State::Notification {
                    pending: Vec::new(),
                }
            }
            Definition::Search { .. } => State::Search {
                searching: false,
                subjects: Vec::new(),
                candidates: Vec::new(),
                dispatched: false,
            },
            Definition::Herd => State::Herd(Formation {
                settings: None,
                origin: [0.;3],
                evaluators: Vec::new(),
                members: Vec::with_capacity(16),
                destinations: Vec::with_capacity(8),
                active: false,
            }),
        };
        Ok(Self {
            owner,
            definition,
            core: None,
            knowledge: None,
            state,
            initialized: false,
            started: false,
        })
    }
    ///3ea320: the three real evaluator owners are admitted during Init,
    ///independently of whether any NPC has joined the crew.
    pub fn initialize_herd(&mut self) {
        if let State::Herd(formation) = &mut self.state {
            if !formation.evaluators.is_empty() {
                return;
            }
            formation.evaluators = vec![
                //Source constructors leave these parameters unwritten until
                //3e94b0 configures a real formation; None preserves that state.
                Evaluator {
                    index: -1,
                    flags: 9,
                    kind: Evaluation::Distance { parameters: None },
                },
                Evaluator {
                    index: -1,
                    flags: 9,
                    kind: Evaluation::NavigationLos { target: None },
                },
                Evaluator {
                    index: -1,
                    flags: 8,
                    kind: Evaluation::AngleToPositions {
                        origin: [0.; 3],
                        positions: Vec::new(),
                    },
                },
            ];
        }
    }
    ///3eaa80 unregisters formation evaluators before destroying their owners.
    pub fn deinitialize(&mut self) -> Result<(), String> {
        if self.started {
            return Err("Crew collector Deinitialize precedes Stop".into());
        }
        if let State::Herd(formation) = &mut self.state {
            formation.active = false;
            formation.evaluators.clear();
        }
        self.core = None;
        self.knowledge = None;
        self.initialized = false;
        Ok(())
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
