//!651020 first-submersion SurfaceImpact adapter. The fluid manager retains
//! its real packet until the canonical ContactFX owner accepts that packet.
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};
#[derive(Resource, Default)]
struct WaterImpacts {
    database: Option<Arc<crate::assets::entities::contacts::ContactDatabase>>,
    pairs: HashMap<(String, String), Arc<str>>,
    closed: bool,
}
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<WaterImpacts>() {
        return Err("water impact owner already exists before level publication".into());
    }
    world.insert_resource(WaterImpacts::default());
    Ok(())
}
pub(super) fn prepare(world: &mut World) -> Result<(), String> {
    let Some(owner) = world.get_resource::<WaterImpacts>() else { return Ok(()); };
    if owner.closed || owner.database.is_some() { return Ok(()); }
    if !world.contains_resource::<super::physics_scene::PhysicsScene>() { return Ok(()); }
    //ScenePreparation already admits this through AudioPreparation on the
    //map worker. Borrow its exact result, including overlay generations and
    //failures; never reacquire/parse metadata on the loading UI thread.
    let Some(database) = super::audio::prepared_contact_database(world)? else { return Ok(()); };
    world.resource_mut::<WaterImpacts>().database = Some(database);
    Ok(())
}
pub(super) fn startup_pending(world: &World) -> bool {
    //Maps without a physics scene have no fluid manager or impact producer.
    //Keep initialization unconditional, but do not wait for absent work.
    world.contains_resource::<super::physics_scene::PhysicsScene>()
        && world.get_resource::<WaterImpacts>().is_some_and(|owner|owner.database.is_none())
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
        let source = impact.source_material.name.clone();
        let target = impact.target_material.name.clone();
        let pair = (source.clone(), target.clone());
        let scope = if let Some(scope) = world.resource::<WaterImpacts>().pairs.get(&pair) {
            scope.clone()
        } else {
            let scope: Arc<str> = Arc::from(format!("water-impact:{source}:{target}"));
            let database = world.resource::<WaterImpacts>().database.clone()
                .ok_or("water contact database was not prepared")?;
            super::explosions::prepare_contact_pair_leases_owned(world,scope.clone(),
                impact.source_material.clone(),impact.target_material.clone(),database)?;
            world.resource_mut::<WaterImpacts>().pairs.insert(pair,scope.clone());
            scope
        };
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
                target_definition: Some(impact.target_material.clone()),
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
            super::explosions::release_owned(world, pair)?;
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
