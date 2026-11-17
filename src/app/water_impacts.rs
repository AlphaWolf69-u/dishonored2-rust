//!651020 first-submersion SurfaceImpact adapter. The fluid manager retains
//! its real packet until the canonical ContactFX owner accepts that packet.
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};
struct Pair {
    scope: Arc<str>,
    requested: bool,
    ready: bool,
}
#[derive(Resource, Default)]
struct WaterImpacts {
    revision: Option<usize>,
    pairs: HashMap<(String, String), Pair>,
    closed: bool,
    pending: bool,
}
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<WaterImpacts>() {
        return Err("water impact owner already exists before level publication".into());
    }
    world.insert_resource(WaterImpacts::default());
    Ok(())
}
pub(super) fn prepare(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<WaterImpacts>() else {
        return Ok(());
    };
    let result = (|| {
        if owner.closed {
            return Ok(());
        }
        let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() else {
            return Ok(());
        };
        let revision = scene.water_impact_material_revision()?;
        if owner.revision != Some(revision) {
            let pairs = scene.water_impact_material_pairs()?;
            for (source, target) in pairs {
                if owner.pairs.contains_key(&(source.clone(), target.clone())) {
                    continue;
                }
                let scope: Arc<str> = Arc::from(format!("water-impact:{}:{}", source, target));
                owner.pending = true;
                //Record before acquisition; a partial reservation must retire.
                owner.pairs.insert(
                    (source.clone(), target.clone()),
                    Pair {
                        scope: scope.clone(),
                        requested: false,
                        ready: false,
                    },
                );
            }
            owner.revision = Some(revision);
        }
        if !owner.pending {
            return Ok(());
        }
        let mut pending = false;
        for ((source, target), pair) in &mut owner.pairs {
            if !pair.requested {
                super::explosions::prepare_contact_pair_owned(
                    world,
                    pair.scope.clone(),
                    source,
                    target,
                )?;
                pair.requested = true;
            }
            if !pair.ready {
                pair.ready = super::explosions::ready_contact_owned(world, &pair.scope, source)?;
            }
            pending |= !pair.ready;
        }
        owner.pending = pending;
        Ok(())
    })();
    world.insert_resource(owner);
    result
}
pub(super) fn startup_pending(world: &World) -> bool {
    world
        .get_resource::<WaterImpacts>()
        .is_some_and(|owner| owner.revision.is_none() || owner.pending)
}
pub(super) fn update(world: &mut World) -> Result<(), String> {
    if world
        .get_resource::<WaterImpacts>()
        .is_none_or(|owner| owner.closed)
    {
        return Ok(());
    }
    loop {
        let Some(impact) = world
            .get_resource::<super::physics_scene::PhysicsScene>()
            .and_then(|scene| scene.pending_water_impact())
            .cloned()
        else {
            return Ok(());
        };
        let (source, target) = world
            .resource::<super::physics_scene::PhysicsScene>()
            .water_impact_material_names(&impact)?;
        let Some(pair) = world
            .resource::<WaterImpacts>()
            .pairs
            .get(&(source.clone(), target.clone()))
        else {
            return Ok(());
        };
        if !pair.ready {
            return Ok(());
        }
        let scope = pair.scope.clone();
        let context = crate::rendering::bevy_backend::world_effects::EntityEffectContext {
            owner: Some(impact.owner),
            culprit: None,
            cannot_incapacitate_player_and_allies: false,
            player_impact: false,
            velocities: [[0.; 3]; 2],
            contact_speed: impact.speed,
            contact_mass: impact.mass,
            //8520d0 passes this actual source body bit solely to the native
            //contact-stimulus completion callback, never to a time-space clock.
            contact_stimulus_enabled: Some(impact.body_flags & (1 << 19) != 0),
        };
        super::explosions::post_contact_pair_owned(
            world,
            &scope,
            super::physics_scene::explosion_effects::Contact {
                source,
                target,
                target_definition: None,
                position: bevy_math::Vec3::from_array(impact.point),
                normal: bevy_math::Vec3::from_array(impact.normal),
                direction: bevy_math::Vec3::from_array(impact.direction),
            },
            context,
        )?;
        //None is accepted too: the real ContactDatabase can select no effect.
        world
            .resource_mut::<super::physics_scene::PhysicsScene>()
            .acknowledge_water_impact()?;
    }
}
pub(super) fn begin_retirement(world: &mut World) {
    if let Some(mut owner) = world.get_resource_mut::<WaterImpacts>() {
        owner.closed = true;
    }
}
pub(super) fn retire(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<WaterImpacts>() else {
        return Ok(());
    };
    owner.closed = true;
    let result = (|| {
        for pair in owner.pairs.values() {
            super::explosions::release_owned(world, &pair.scope)?;
        }
        Ok(())
    })();
    if result.is_err() {
        world.insert_resource(owner);
    }
    result
}

///Only the already accepted source packet is awaited during world close.
pub(super) fn accepted_pending(world: &World) -> bool {
    world
        .get_resource::<super::physics_scene::PhysicsScene>()
        .is_some_and(|scene| scene.pending_water_impact().is_some())
}
