//! Crew core startup state579b00/582760/41ac70. No NPC is inferred from an
//! actor's name. References always carry canonical component generations.
use crate::{
    assets::ai_world::crew::Definition,
    gameplay::{
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
        pickable_component::CheckedComponent,
    },
};
use std::sync::Arc;

pub struct RuleInstance {
    pub rule: usize,
    pub next_action: usize,
    pub finished: bool,
}
pub struct RuleRuntime {
    pub instances: Vec<RuleInstance>,
    pub entity: EntityId,
}
impl RuleRuntime {
    ///8952a0 instantiates only893e30's surviving rules; the installed Crew
    ///default has no live rules because every omitted policy is NONE0.
    pub fn new(
        definition: &crate::assets::slash_rules::crew::Definition,
        entity: EntityId,
    ) -> Self {
        Self {
            instances: definition
                .active_rules()
                .map(|(rule, _)| RuleInstance {
                    rule,
                    next_action: 0,
                    finished: false,
                })
                .collect(),
            entity,
        }
    }
}
pub struct Core {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub runtime: Option<RuleRuntime>,
    pub variable_offset: i32,
    pub decision: Option<CheckedComponent>,
    pub knowledge: Option<CheckedComponent>,
    pub binding: Option<EntityId>,
    pub binding_name: Arc<str>,
    pub members: Vec<EntityId>,
    pub requester: Option<EntityId>,
    pub request_active: bool,
    pub request_count: i32,
    pub request_next: i64,
    pub members_changed: bool,
    pub initialized: bool,
    pub started: bool,
}
impl Core {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Core { .. }) {
            return Err("CrewCore declaration class differs".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            runtime: None,
            variable_offset: i32::MIN,
            decision: None,
            knowledge: None,
            binding: None,
            binding_name: Arc::from(""),
            members: Vec::new(),
            requester: None,
            request_active: false,
            request_count: 0,
            request_next: -1,
            members_changed: false,
            initialized: false,
            started: false,
        })
    }
    ///57ba40 resets the role-request count and deadline. With no admitted
    ///NPC requests, its source in-place16 records need no heap allocation.
    pub fn reset_requests(&mut self) {
        self.request_count = 0;
        self.request_next = -1;
    }
}

///582760 constructs empty lists; no producer for their NPC rows is admitted
///until a real NPC reader is installed. The actual published flag bytes are
///retained independently, never inferred from a missing NPC implementation.
pub struct Knowledge {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub area: i32,
    pub flags: [bool; 20],
    pub selected: Option<EntityId>,
    pub initialized: bool,
    pub started: bool,
}
impl Knowledge {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Knowledge) {
            return Err("CrewKnowledge declaration class differs".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            area: -1,
            flags: [false; 20],
            selected: None,
            initialized: false,
            started: false,
        })
    }
    pub fn reset(&mut self) {
        self.flags.fill(false);
    }
    ///5842f0/583390, value+150 and availability+1a5.
    pub fn set_area(&mut self, area: i32) {
        self.area = area;
        self.flags[0x1a5 - 0x198] = true;
    }
    pub fn clear_area(&mut self) {
        self.flags[0x1a5 - 0x198] = false;
    }
    ///57e410 clears combat publication198..19f without changing payloads.
    pub fn clear_combat(&mut self) {
        self.flags[..8].fill(false);
    }
    ///5812e0→5832f0/583420/583400/583410.
    pub fn clear_search(&mut self) {
        for offset in [0x1aa, 0x1a0, 0x1a2, 0x1ab] {
            self.flags[offset - 0x198] = false;
        }
    }
}
pub struct Coordination {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub last_update: f64,
    pub count: u32,
    pub last_request: f64,
    pub request_count: u32,
    pub target: Option<EntityId>,
    pub flags: [bool; 8],
    pub initialized: bool,
    pub started: bool,
}
pub struct Decision {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub runtime: Option<super::crew_tree::Instance>,
    pub pending_tree: Option<Arc<crate::assets::ai_world::crew_tree::Definition>>,
    pub initialized: bool,
    pub started: bool,
    ///Host retry acknowledgement for accepted phase3 traversal.
    pub updated_frame:Option<i32>,
}
impl Decision {
    ///90b5d0→5880a0 initializes allthree Decision pointers NULL.
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Decision { .. }) {
            return Err("CrewDecision declaration class differs".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            runtime: None,
            pending_tree: None,
            initialized: false,
            started: false,
            updated_frame:None,
        })
    }
}
impl Coordination {
    pub fn new(definition: Arc<Definition>) -> Result<Self, String> {
        if !matches!(&*definition, Definition::Coordination) {
            return Err("CrewCoordination declaration class differs".into());
        }
        Ok(Self {
            owner: ComponentOwner::default(),
            definition,
            last_update: -1.,
            count: 0,
            last_request: -1.,
            request_count: 0,
            target: None,
            flags: [false; 8],
            initialized: false,
            started: false,
        })
    }
    ///41b690 resets only published6c..73 and the second list's count.
    pub fn reset(&mut self) {
        self.flags.fill(false);
    }
}
macro_rules! component {($($kind:ty),+)=>{$(impl ComponentInstance for $kind {
    fn owner(&self)->&ComponentOwner{&self.owner}
    fn owner_mut(&mut self)->&mut ComponentOwner{&mut self.owner}
})+}}
component!(Core, Knowledge, Coordination, Decision);
