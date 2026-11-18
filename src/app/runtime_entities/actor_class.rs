//! Concrete idDynamicEntity constructor881d90, Spawn8820f0 and
//! activation882070. Its destructor87e350 adds no resource owner to idEntity.
use super::*;
///784450's arkPlayer fields for a source-declared body-part actor. This is
///not another player controller. Its existing Arm owns AnimEvolved/Midnight
///and Audio; PlayerArms owns its current model through GPU release. Alternate
///model selection remains with the existing player model owner, not this ID.
///The14 power pointers and optional Attributes/audiocard slots stay absent
///for the actual arm component roster.784540 destroys no additional owner.
pub(in crate::app) struct PlayerPart {
    #[allow(dead_code)]
    aim_at_position: [f32;3],
    #[allow(dead_code)]
    choke_hostage: Option<EntityId>,
    #[allow(dead_code)]
    busy_to_talk: i32,
}
impl PlayerPart {
    pub(in crate::app) fn new(birth_position:[f32;3])->Self {
        //784c10 copies the actual birth transform after the constructor's
        //zero aim vector; no optional audiocard is authored on these parts.
        Self{aim_at_position:birth_position,choke_hostage:None,busy_to_talk:0}
    }
    pub(in crate::app) fn declaration_flags(source:&str)->u8 {
        //7846a0's declaration-name test runs before8aecc0 registration.
        if source.contains("/player/"){8}else{4}
    }
}
///Concrete3db480 members survive with the actual arkAICue generation.
pub(in crate::app) struct Cue {
    pub category: i32,
    pub deadline: f64,
    pub instigator: Option<EntityId>,
    pub instigator_kind: u32,
    pub gives_full_perception: bool,
}
impl Cue {
    pub(super) fn new() -> Self { Self { category: 0, deadline: 0., instigator: None, instigator_kind: 0, gives_full_perception: false } }
}

///783940 initializes the real arkMovable members before4e65a0 installs
///arkGrenade's concrete vtable. The managed link is distinct from owner+ac.
pub(in crate::app) struct Movable {
    linked_entity: Option<EntityId>,
    picked_up: bool,
}
impl Movable {
    pub(in crate::app) fn new() -> Self { Self { linked_entity: None, picked_up: false } }
    pub(in crate::app) fn linked_entity(&self)->Option<EntityId>{self.linked_entity}
    pub(in crate::app) fn picked_up(&mut self, successful: bool) -> bool {
        if successful { self.picked_up = true; }
        self.picked_up
    }
}

///4e6540 constructs the actual reflected +360 bool. The source class
///inherits ordinary idEntity component/render lifetime; it owns no extra
///backend resource. Keep its state on the actual runtime generation only.
pub(super) struct Breakable {
    // Retained reflected source member; no admitted runtime reader yet.
    #[allow(dead_code)]
    navmesh_static_cutting: bool,
}
impl Breakable {
    pub(super) fn new(navmesh_static_cutting: bool) -> Self {
        Self { navmesh_static_cutting }
    }
}

pub(super) struct Dynamic {
    hide_on_spawn: bool,
    last_activation: f64,
    activated: bool,
    spawned: bool,
}
impl Dynamic {
    pub(super) fn new(hide_on_spawn: bool) -> Self {
        Self { hide_on_spawn, last_activation: -1., activated: false, spawned: false }
    }
}
fn time(world: &World) -> Result<f64, String> {
    world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .ok_or("dynamic entity requires the actual game clock")?
        .clock()?.time(3).ok_or_else(|| "dynamic entity clock group3 is absent".into())
}
/// Called at the actual class-Spawn boundary, before component Initialize.
pub(super) fn spawn(world: &mut World, id: EntityId) -> Result<bool, String> {
    if world.resource::<RuntimeEntities>().actors.get(&id)
        .ok_or("class Spawn journal is absent")?.breakable_class.is_some() {
        //1413c19f0 registers actual Spawn0998b0: no callback side effects.
        //The navigation-cut property remains typed source state; this class
        //has no independent runtime navigation consumer to manufacture here.
        return Ok(false)
    }
    if world.resource::<RuntimeEntities>().actors.get(&id)
        .ok_or("class Spawn journal is absent")?.dynamic.is_none() { return Ok(false) }
    let now = time(world)?;
    let mut owner = world.resource_mut::<RuntimeEntities>();
    let actor = owner.actors.get_mut(&id).ok_or("class Spawn journal is absent")?;
    let Some(state) = actor.dynamic.as_mut() else { return Ok(false) };
    if state.spawned { return Ok(state.hide_on_spawn) }
    state.last_activation = now;
    state.activated = false;
    state.spawned = true;
    Ok(state.hide_on_spawn)
}
pub(super) fn activate(world: &mut World, id: EntityId) -> Result<bool, String> {
    let owner = world.resource::<RuntimeEntities>();
    let actor = owner.actors.get(&id).ok_or("activation has no actual constructor journal")?;
    match actor.definition.as_ref().map(|definition| definition.class.as_str()) {
        //1413c3200 registers actual no-op Spawn/activation0998b0 for Grenade.
        Some("idEntity" | "arkProjectile" | "arkGrenade" | "arkAIWorld" | "arkAICue") => return Ok(false),
        Some("arkBreakable") => {
            actor.breakable_class.as_ref().ok_or("arkBreakable constructor state is absent")?;
            //Concrete vtable141b67bc8 +128 is actual no-op0998b0.
            return Ok(false)
        }
        Some("idDynamicEntity") => (),
        _ => return Err("activation concrete class adapter is absent".into()),
    }
    let now = time(world)?;
    let mut owner = world.resource_mut::<RuntimeEntities>();
    let actor = owner.actors.get_mut(&id).ok_or("activation has no actual constructor journal")?;
    match actor.definition.as_ref().map(|definition| definition.class.as_str()) {
        Some("idDynamicEntity") => {
            let state = actor.dynamic.as_mut().ok_or("dynamic constructor state is absent")?;
            if !state.spawned { return Err("dynamic activation precedes class Spawn".into()) }
            state.last_activation = now;
            state.activated = !state.activated;
            Ok(true)
        }
        _ => Err("activation concrete class adapter is absent".into()),
    }
}
