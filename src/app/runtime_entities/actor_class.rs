//! Concrete idDynamicEntity constructor881d90, Spawn8820f0 and
//! activation882070. Its destructor87e350 adds no resource owner to idEntity.
use super::*;

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
        Some("idEntity" | "arkProjectile") => return Ok(false),
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
