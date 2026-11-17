//! Player AnimBase/Loco A9 media leases. Clip metadata stays on the existing
//! Body/Arm banks; event graphs/streams stay in EventReservations.
use super::*;

#[derive(Default)]
struct Part {
    revision: Option<u64>,
    scope: Option<Arc<str>>,
    ready: bool,
}
#[derive(Default)]
pub(in crate::app) struct Admission {
    parts: [Part; 3],
    handoffs: Vec<Arc<str>>,
    closed: bool,
}
impl Admission {
    pub(in crate::app) fn ready(&self, revisions: [u64; 3]) -> [bool; 3] {
        std::array::from_fn(|part| {
            self.parts[part].ready && self.parts[part].revision == Some(revisions[part])
        })
    }
    pub(in crate::app) fn transfer(&mut self, scope: Option<Arc<str>>) {
        if let Some(scope) = scope {
            self.handoffs.push(scope);
        }
    }
}

impl Admission {
    pub(in crate::app) fn prepare(
        &mut self,
        world: &mut World,
        id: crate::gameplay::entity_lifetime::EntityId,
        player: &crate::gameplay::player::Player,
    ) -> Result<(), String> {
        let admission = self;
        if admission.closed {
            return Ok(());
        }
        let revisions = player.lower_animation_sound_revision();
        let changed: [bool; 3] =
            std::array::from_fn(|part| admission.parts[part].revision != Some(revisions[part]));
        let mut declarations: [Vec<Arc<crate::assets::wwise::SoundEventDeclaration>>; 3] =
            Default::default();
        if changed.iter().any(|changed| *changed) {
            player.visit_lower_animation_sounds(|part, sound| {
                if changed[part] {
                    declarations[part].push(sound.event.clone());
                }
            });
        }
        for part in 0..3 {
            let state = &mut admission.parts[part];
            if changed[part] {
                let scope = if declarations[part].is_empty() {
                    None
                } else {
                    let scope: Arc<str> =
                        format!("player-animation:{id:?}:{part}:{}", revisions[part]).into();
                    // Bank preparation only appends aliases. Claim the new
                    // union before releasing the previous scope's event users.
                    super::reserve_events(
                        world,
                        scope.clone(),
                        std::mem::take(&mut declarations[part]),
                    )?;
                    Some(scope)
                };
                if let Some(previous) = state.scope.take() {
                    super::release_events(world, &previous)?;
                }
                state.scope = scope;
                state.revision = Some(revisions[part]);
                state.ready = state.scope.is_none();
            }
            if !state.ready {
                state.ready = super::events_ready(
                    world,
                    state.scope.as_ref().expect("pending actual event scope"),
                )?;
            }
        }
        if admission.ready(revisions).into_iter().all(|ready| ready) {
            for scope in admission.handoffs.drain(..) {
                super::release_events(world, &scope)?;
            }
        }
        Ok(())
    }
}
pub(in crate::app) fn prepare_player_animations(world: &mut World) -> Result<(), String> {
    let Some(id) = super::super::player_lifetime::id(world) else {
        return Ok(());
    };
    if !world.contains_resource::<super::super::player_actions::PlayerActions>() {
        return Err("player animation Audio owner absent".into());
    }
    if !world.contains_resource::<super::super::player::PlayerState>() {
        return Ok(());
    }
    world.resource_scope(
        |world, mut actions: Mut<super::super::player_actions::PlayerActions>| {
            world.resource_scope(
                |world, state: Mut<super::super::player::PlayerState>| match state.player.as_ref() {
                    Some(player) => actions.animation_audio.prepare(world, id, player),
                    None => Ok(()),
                },
            )
        },
    )
}

pub(in crate::app) fn player_animations_ready(world: &World) -> bool {
    let Some(player) = world
        .get_resource::<super::super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
    else {
        return true;
    };
    world
        .get_resource::<super::super::player_actions::PlayerActions>()
        .is_some_and(|actions| {
            actions
                .animation_audio
                .ready(player.lower_animation_sound_revision())
                .into_iter()
                .all(|ready| ready)
        })
}

/// Prepared equipment cannot activate its new clip bank before its exact
/// source events are ready. The lease is handed to the live bank owner after
/// successful stage; stale/canceled results release through the same helper.
pub(in crate::app) fn prepare_dependency(
    world: &mut World,
    key: &str,
    scope: &mut Option<Arc<str>>,
    events: Vec<Arc<crate::assets::wwise::SoundEventDeclaration>>,
) -> Result<bool, String> {
    if let Some(scope) = scope {
        return super::events_ready(world, scope);
    }
    if events.is_empty() {
        return Ok(true);
    }
    let value: Arc<str> = format!("{key}:animation-audio").into();
    super::reserve_events(world, value.clone(), events)?;
    *scope = Some(value.clone());
    // Retain the cancellation key even if an already completed decode failed.
    super::events_ready(world, &value)
}
pub(in crate::app) fn cancel_dependency(
    world: &mut World,
    scope: &mut Option<Arc<str>>,
) -> Result<(), String> {
    if let Some(scope) = scope.take() {
        super::release_events(world, &scope)?;
    }
    Ok(())
}

pub(in crate::app) fn retire_player_animations(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<super::super::player_actions::PlayerActions>() {
        return Ok(());
    }
    world.resource_scope(
        |world, mut actions: Mut<super::super::player_actions::PlayerActions>| {
            actions.cancel_pending_animation_audio(world)?;
            let admission = &mut actions.animation_audio;
            admission.closed = true;
            for part in &mut admission.parts {
                if let Some(scope) = part.scope.take() {
                    super::release_events(world, &scope)?;
                }
            }
            for scope in admission.handoffs.drain(..) {
                super::release_events(world, &scope)?;
            }
            Ok(())
        },
    )
}
