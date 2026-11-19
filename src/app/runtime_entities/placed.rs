//! Placed Breakable source startup. The scene worker retains typed plans;
//!the published canonical factory creates real hidden chunks before admitting
//!the parent's component. Loading remains gated through its real Start.
use super::*;
pub(in crate::app) struct Child {
    pub step: usize,
    pub chunk: usize,
    pub definition: Arc<Definition>,
}
pub(in crate::app) struct Prepared {
    pub actor: EntityId,
    pub definition: Arc<crate::assets::breakable::Definition>,
    pub scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling,
    pub source: usize,
    pub scale: [f32; 3],
    pub scope: Arc<str>,
    pub children: Vec<Child>,
    pub created: Vec<(usize, usize, EntityId)>,
    pub handle: Option<crate::gameplay::pickable_component::CheckedComponent>,
    pub requested: bool,
    pub installed: usize,
}
pub(super) struct Lease {
    pub scope: Arc<str>,
    pub children: Vec<Arc<Definition>>,
}
pub(in crate::app) struct SoundVolume {
    pub actor: EntityId,
    pub definition: Arc<crate::gameplay::sound::volume_control::Definition>,
    pub scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling,
    pub source: usize,
    pub handle: Option<crate::gameplay::pickable_component::CheckedComponent>,
}
pub(super) fn advance(world: &mut World) -> Result<(), String> {
    if world
        .resource::<RuntimeEntities>()
        .cancelled
        .load(Ordering::Acquire)
    {
        return Ok(());
    }
    //The source Breakable Start can immediately adjust this actual component.
    //Its own world/bounds-dependent Initialize must therefore complete first.
    while let Some(mut plan) = world
        .resource_mut::<RuntimeEntities>()
        .placed_sound_volumes
        .pop_front()
    {
        let result = with_scene(world, |world, scene, live, events| {
            scene.admit_prepared_sound_volume(world, live, events, &mut plan)
        });
        if let Err(error) = result {
            world
                .resource_mut::<RuntimeEntities>()
                .placed_sound_volumes
                .push_front(plan);
            return Err(error);
        }
    }
    // Admit all warm demands first, so independent canonical jobs overlap
    //rather than serializing one complete object's GPU/FX wait after another.
    let requests = {
        let owner = world.resource::<RuntimeEntities>();
        owner
            .placed_pending
            .iter()
            .enumerate()
            .filter(|(_, plan)| !plan.requested)
            .map(|(index, plan)| {
                (
                    index,
                    plan.scope.clone(),
                    plan.definition
                        .fx_paths()
                        .map(str::to_owned)
                        .collect::<Vec<_>>(),
                    plan.scale,
                    plan.children
                        .iter()
                        .map(|child| child.definition.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    for (index, scope, paths, scale, children) in requests {
        world
            .resource_mut::<super::super::usable_effects::EffectRequests>()
            .reserve(scope.clone(), paths);
        for child in children {
            request_scaled_for(world, scope.clone(), child, scale)?;
        }
        world.resource_mut::<RuntimeEntities>().placed_pending[index].requested = true;
    }
    super::openables::advance(world)?;
    super::whale_oil_tanks::advance(world)?;
    loop {
        let Some(plan) = world.resource::<RuntimeEntities>().placed_pending.front() else {
            return Ok(());
        };
        if !super::super::usable_effects::reservation_ready(world, &plan.scope)? {
            return Ok(());
        }
        for child in &plan.children {
            if !ready_scaled(world, &child.definition, plan.scale)? {
                return Ok(());
            }
        }
        let mut plan = world
            .resource_mut::<RuntimeEntities>()
            .placed_pending
            .pop_front()
            .ok_or("placed Breakable source plan disappeared")?;
        let result = (|| {
            let frame = with_scene(world, |_, scene, live, _| {
                scene.published_entity_frame(live, plan.actor)
            })?;
            while plan.created.len() < plan.children.len() {
                let child = &plan.children[plan.created.len()];
                let entity = spawn_inner(
                    world,
                    child.definition.clone(),
                    SpawnOptions {
                        frame: frame.pose,
                        scale: Some(plan.scale),
                        hidden_disabled: true,
                    },
                    None,
                    None,
                )?;
                plan.created.push((child.step, child.chunk, entity));
            }
            with_scene(world, |world, scene, live, events| {
                scene.admit_prepared_breakable(world, live, events, &mut plan)
            })?;
            Ok(())
        })();
        if let Err(error) = result {
            world
                .resource_mut::<RuntimeEntities>()
                .placed_pending
                .push_front(plan);
            return Err(error);
        }
        world
            .resource_mut::<RuntimeEntities>()
            .placed_leases
            .entry(plan.actor)
            .or_default()
            .push(Lease {
                scope: plan.scope,
                children: plan
                    .children
                    .into_iter()
                    .map(|child| child.definition)
                    .collect(),
            });
    }
}
pub(super) fn release(world: &mut World, id: EntityId) -> Result<(), String> {
    let leases = world
        .resource_mut::<RuntimeEntities>()
        .placed_leases
        .remove(&id)
        .unwrap_or_default();
    for lease in leases {
        world
            .resource_mut::<super::super::usable_effects::EffectRequests>()
            .release_reservation(lease.scope.clone());
        for child in lease.children {
            release_for(world, &lease.scope, &child)?;
        }
    }
    Ok(())
}
pub(super) fn shutdown(world: &mut World) -> Result<(), String> {
    super::openables::shutdown(world)?;
    super::whale_oil_tanks::shutdown(world)?;
    //Actual admitted components retain their own typed definition until the
    //shared registry releases them; unadmitted plans have no backend lease.
    world
        .resource_mut::<RuntimeEntities>()
        .placed_sound_volumes
        .clear();
    let pending = std::mem::take(&mut world.resource_mut::<RuntimeEntities>().placed_pending);
    for plan in pending {
        world
            .resource_mut::<super::super::usable_effects::EffectRequests>()
            .release_reservation(plan.scope.clone());
        for child in plan.children {
            release_for(world, &plan.scope, &child.definition)?;
        }
    }
    let actors = world
        .resource::<RuntimeEntities>()
        .placed_leases
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for actor in actors {
        release(world, actor)?;
    }
    Ok(())
}
