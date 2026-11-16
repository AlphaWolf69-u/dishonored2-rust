//! Real ranged callbacks: prepared entity birth, source aiming and retained
//! equipment children. This is the single consumer of the weapon queue.
use crate::{
    assets::projectiles::Prepared,
    gameplay::{
        collision_filter::CollisionQueryFilter,
        collision_hit::ShapeHitInfo,
        entity_lifetime::EntityId,
        player::{
            Player,
            weapons::{
                FireSource, Ranged, RangedFamily, WeaponEffect, WeaponMessage,
                shooting::{self, SightQueries},
            },
        },
        reference_frame::ReferenceFrame,
        shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
        shape_ray_query::ShapeRayContact,
        world_convex_query::{WorldConvexQuery, WorldConvexRequest},
        world_point_query::level::LevelPointQueries,
        world_ray_query::{WorldRayQuery, WorldRayRequest},
    },
};
use bevy::{camera::CameraProjection, prelude::*};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
mod owned;
pub(super) use owned::{remove_owned, request_owned, shutdown, startup_pending};

#[derive(Resource, Default)]
struct Firing {
    owned: HashMap<EntityId, owned::Owned>,
    generation: u64,
    next_effect: u64,
    ray: WorldRayQuery,
    convex: WorldConvexQuery,
    rays: Vec<ShapeRayContact>,
    casts: Vec<ConvexContact>,
    starts: Vec<ConvexContact>,
    error: Option<String>,
    deliveries: VecDeque<Delivery>,
}
struct Delivery {
    message: WeaponMessage,
    phase: u8,
}
pub(super) fn startup_error(world: &World) -> Option<&str> {
    world
        .get_resource::<Firing>()
        .and_then(|owner| owner.error.as_deref())
}
fn hit_identity() -> ShapeHitInfo {
    ShapeHitInfo {
        body_id: None,
        material_id: 0xffff,
        shape_key: u32::MAX,
        filter_info: 0,
        user_data: 0,
    }
}
fn ray(
    owner: &mut Firing,
    queries: &LevelPointQueries,
    from: Vec3,
    to: Vec3,
    layers: u32,
    required_flags: u32,
) -> Result<Option<ShapeRayContact>, String> {
    queries.ray_hits(
        &mut owner.ray,
        &WorldRayRequest {
            origin: from.to_array(),
            direction: (to - from).to_array(),
            maximum_fraction: 1.0,
            hit: hit_identity(),
            filter: Some(CollisionQueryFilter {
                layers,
                required_flags,
            }),
        },
        0,
        &mut owner.rays,
    )?;
    Ok(owner
        .rays
        .iter()
        .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
        .copied())
}
struct SightWorld<'a> {
    owner: &'a mut Firing,
    world: &'a LevelPointQueries,
    shape: [[f32; 4]; 8],
    error: Option<String>,
}
impl SightQueries for SightWorld<'_> {
    fn ray_distance(&mut self, from: Vec3, to: Vec3) -> Option<f32> {
        match ray(self.owner, self.world, from, to, 0x1ffff, 0x800) {
            Ok(hit) => hit.map(|hit| hit.fraction * (to - from).length()),
            Err(error) => {
                self.error = Some(error);
                Some(0.0)
            }
        }
    }
    fn shape_distance(&mut self, from: Vec3, to: Vec3) -> Option<f32> {
        let mut transform = Mat4::IDENTITY.to_cols_array_2d();
        transform[3] = from.extend(1.0).to_array();
        let request = WorldConvexRequest {
            cast: ConvexCast {
                geometry: ConvexQueryGeometry {
                    shape_type: 5,
                    vertices: &self.shape,
                    radius: 0.01,
                },
                transform,
                displacement: (to - from).to_array(),
                tolerance: 0.001,
                start_tolerance: 0.0,
                hit: hit_identity(),
                filter: Some(CollisionQueryFilter {
                    layers: 0x1ffff,
                    required_flags: 0x20000,
                }),
            },
            maximum_fraction: 1.0,
        };
        match self.world.convex_hits(
            &mut self.owner.convex,
            &request,
            &mut self.owner.casts,
            &mut self.owner.starts,
        ) {
            Ok(()) => self
                .owner
                .casts
                .iter()
                .map(|hit| hit.fraction)
                .min_by(f32::total_cmp)
                .map(|fraction| fraction * (to - from).length()),
            Err(error) => {
                self.error = Some(error);
                Some(0.0)
            }
        }
    }
}

///638ae0 is shared by every Ranged-derived weapon, including WeaponThrowable.
///Reuse the canonical sight query scratch instead of a second ray/cast owner.
pub(in crate::app) fn spawn_origin(
    world: &mut World, eye: Vec3, camera_axis: Mat3, muzzle: Vec3,
    weapon_axis: Mat3, bounds: [[f32; 3]; 2], max_deviation_degrees: f32, aim: Vec3,
) -> Result<Vec3, String> {
    if !world.contains_resource::<Firing>() { world.insert_resource(Firing::default()); }
    let mut owner = world.remove_resource::<Firing>().ok_or("weapon sight owner is absent")?;
    let result = sight_origin(world, &mut owner, eye, camera_axis, muzzle, weapon_axis, bounds,
        max_deviation_degrees, aim);
    world.insert_resource(owner);
    result
}
fn sight_origin(
    world: &World, owner: &mut Firing, eye: Vec3, camera_axis: Mat3, muzzle: Vec3,
    weapon_axis: Mat3, bounds: [[f32; 3]; 2], max_deviation_degrees: f32, aim: Vec3,
) -> Result<Vec3, String> {
    let minimum = Vec3::from_array(bounds[0]);
    let maximum = Vec3::from_array(bounds[1]);
    let Some(shape) = shooting::sight_box(minimum, maximum) else { return Ok(muzzle); };
    let queries = world.resource::<super::physics_scene::PhysicsScene>().point_queries()?;
    let queries = queries.read().map_err(|_| "weapon sight physics owner is poisoned")?;
    let mut cast = SightWorld { owner, world: &queries, shape, error: None };
    let position = shooting::Sight { eye, camera_axis, muzzle, weapon_axis,
        bounds_min: minimum, bounds_max: maximum, max_deviation_degrees }
        .spawn_position(aim, false, false, &mut cast);
    cast.error.map_or(Ok(position), Err)
}

/// Run before the player component stages. No archive work occurs here: a
/// selected component is admitted only after its actual current ammo and
/// muzzle dependencies have finished through their canonical owners.
pub(super) fn prepare(world: &mut World) {
    if !world.contains_resource::<Firing>() {
        world.insert_resource(Firing::default());
    }
    let Some(mut owner) = world.remove_resource::<Firing>() else {
        return;
    };
    let Some(mut state) = world.remove_resource::<super::player::PlayerState>() else {
        world.insert_resource(owner);
        return;
    };
    let result = (|| {
        let Some(player) = state.player.as_mut() else {
            return Ok(());
        };
        if let Some(weapon) = player.weapon.as_mut() {
            weapon.bevy_backend_projectile_ready = false;
            weapon.fire_source = None;
        }
        owned::advance(world, &mut owner, player)?;
        let (eye, camera_axis) = player.published_camera_frame();
        let queries = world
            .get_resource::<super::physics_scene::PhysicsScene>()
            .ok_or("weapon aim has no canonical physics world")?
            .point_queries()?;
        let queries = queries
            .read()
            .map_err(|_| "weapon aim physics owner is poisoned")?;
        let target = player.ranged_target;
        //726280 has local crosshair +X when no actual autoaim target exists.
        let target_hit = if target.enabled || player.heart_entity().is_some() {
            ray(
                &mut owner,
                &queries,
                eye,
                eye + camera_axis.x_axis * target.maximum_distance,
                target.layers | if player.heart_entity().is_some() { 0x0200_0000 } else { 0 },
                0x20000,
            )?
        } else {
            None
        };
        let hit =
            target_hit.map(|hit| Vec3::new(hit.position[0], hit.position[1], hit.position[2]));
        player.set_ranged_target_hit(hit);
        let entity = target_hit.as_ref().map(|hit| {
            world.resource::<super::physics_scene::PhysicsScene>().collision_entity(
                world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(), &hit.target)
        }).transpose()?.flatten();
        player.set_ranged_target_entity(entity);
        drop(queries);
        let Some(identity) = player.ranged_identity() else {
            return Ok(());
        };
        let Some(entity) = super::equipment_identity::id(world, identity) else {
            return Ok(());
        };
        let Some(entry) = owner.owned.get(&entity) else {
            return Ok(());
        };
        if !entry.ready {
            return Ok(());
        }
        let equipment = entry.entity;
        let projectile_offset = entry
            .descriptor
            .visual
            .as_ref()
            .and_then(|visual| visual.projectile_offset);
        let muzzle_override = player
            .weapon
            .as_ref()
            .and_then(|weapon| entry.muzzle_overrides.get(weapon.ammunition_index()))
            .cloned()
            .flatten();
        let explosion_fx_override = player
            .weapon
            .as_ref()
            .and_then(|weapon| entry.explosion_overrides.get(weapon.ammunition_index()))
            .cloned()
            .flatten();
        let spread = player.weapon.as_ref().and_then(|weapon| {
            if weapon.settings.family != RangedFamily::Gun || !weapon.ammunition().spread_damage {
                return None;
            }
            entry.spreads[weapon.ammunition_index()][if weapon.target_flags & 8 != 0 {
                1
            } else if weapon.target_flags & 0x10 != 0 {
                2
            } else {
                0
            }]
            .clone()
        });
        let Some(attacker) = player.managed_entity() else {
            return Ok(());
        };
        let Some(weapon) = player.weapon.as_ref() else {
            return Ok(());
        };
        let Some(prepared) = weapon.prepared_projectile().cloned() else {
            return Ok(());
        };
        if !super::projectiles::prepared_ready(world, &prepared)? {
            return Ok(());
        }
        let settings = weapon.settings.clone();
        let ammo = weapon.ammunition().clone();
        let Some((equipment_frame, _)) = super::equipment_identity::frame(world, equipment) else {
            return Ok(());
        };
        //63b3c0 first copies the entity frame; a valid resolved model tag
        //replaces it through53dac0. An absent tag preserves that actual frame.
        let muzzle_frame = settings
            .muzzle_tag
            .as_deref()
            .and_then(|tag| player.equipment_tag_world(0, tag))
            .map_or(equipment_frame, |(position, axis)| ReferenceFrame {
                position: position.to_array(),
                basis: axis.to_cols_array_2d(),
            });
        let mut camera_query = world.query_filtered::<(&Camera, &Projection), With<Camera3d>>();
        let Ok((camera, projection)) = camera_query.single(world) else {
            return Ok(());
        };
        let Some(viewport) = camera.physical_viewport_size().map(|size| size.as_vec2()) else {
            return Ok(());
        };
        let game_from_view = Mat4::from_cols(
            (-camera_axis.y_axis).extend(0.0),
            camera_axis.z_axis.extend(0.0),
            (-camera_axis.x_axis).extend(0.0),
            eye.extend(1.0),
        );
        let clip_from_world = projection.get_clip_from_view() * game_from_view.inverse();
        let world_from_clip = clip_from_world.inverse();
        let aim = hit.unwrap_or(eye + camera_axis.x_axis * settings.aim.ideal_distance);
        let (minimum_tag, maximum_tag, spyglass_tag) = match settings.family {
            RangedFamily::Gun => (
                "ARK_ATTRIBUTE_GUN_MIN_DISPERSION_MULTIPLIER",
                "ARK_ATTRIBUTE_GUN_MAX_DISPERSION_MULTIPLIER",
                "ARK_ATTRIBUTE_GUN_SPYGLASS_DISPERSION_MULTIPLIER",
            ),
            RangedFamily::Crossbow => (
                "ARK_ATTRIBUTE_CROSSBOW_MIN_DISPERSION_MULTIPLIER",
                "ARK_ATTRIBUTE_CROSSBOW_MAX_DISPERSION_MULTIPLIER",
                "ARK_ATTRIBUTE_CROSSBOW_SPYGLASS_DISPERSION_MULTIPLIER",
            ),
        };
        let spyglass = if player
            .machine
            .is_active(crate::gameplay::player::states::StateId(0x4c))
        {
            player.attributes.get(spyglass_tag).max(0.0)
        } else {
            1.0
        };
        let source = FireSource {
            prepared,
            attacker,
            equipment,
            eye,
            camera_axis,
            weapon_axis: Mat3::from_cols_array_2d(&equipment_frame.basis),
            muzzle_frame,
            muzzle_world_fx: muzzle_override
                .or_else(|| ammo.muzzle_world_fx.as_deref().map(Arc::from)),
            explosion_fx_override,
            spread,
            aim,
            clip_from_world,
            world_from_clip,
            viewport,
            gravity_multiplier: ammo
                .gravity_multiplier_attribute
                .as_deref()
                .map(|tag| player.attributes.get(tag)),
            damage_increment: ammo
                .damage_increment_attribute
                .as_deref()
                .map_or(0.0, |tag| player.attributes.get(tag)),
            sleep_timer_multiplier: ammo
                .sleep_timer_multiplier_attribute
                .as_deref()
                .map_or(0.0, |tag| player.attributes.get(tag)),
            bypass_big_hit_immunity: ammo
                .bypass_big_hit_immunity_attribute
                .as_deref()
                .is_some_and(|tag| player.attribute_differs_from_base(tag)),
            speed_multiplier: if settings.family == RangedFamily::Crossbow {
                player
                    .attributes
                    .get("ARK_ATTRIBUTE_CROSSBOW_BOLT_SPEED_MULTIPLIER")
            } else {
                1.0
            },
            minimum_dispersion: settings.aim.minimum_dispersion
                * player.attributes.get(minimum_tag).max(0.0)
                * spyglass,
            maximum_dispersion: settings.aim.maximum_dispersion
                * player.attributes.get(maximum_tag).max(0.0)
                * spyglass,
            dispersion_ratio: weapon.dispersion_ratio(),
            extra_dispersion_chance: player
                .attributes
                .get("ARK_ATTRIBUTE_PLAYER_WEAPON_PROJECTILE_EXTRA_DISPERSION_CHANCE"),
            upgrade_flags: weapon.target_flags,
            always_break_on_hit: settings.family == RangedFamily::Crossbow
                && player
                    .attributes
                    .get("ARK_ATTRIBUTE_CROSSBOW_BOLT_ALWAYS_BREAK")
                    > 0.0,
        };
        let attachment = if settings.family == RangedFamily::Crossbow {
            super::equipment_attachment_admission::current_frame_for_player(
                world, equipment, player,
            )
        } else {
            None
        };
        if let Some(weapon) = player.weapon.as_mut() {
            if settings.family == RangedFamily::Crossbow {
                weapon.projectile_attachment_ready = attachment.is_some();
                //62ba10 caches the Fire animation's model-local socket;
                //638ae0 transports it with the actual weapon entity frame.
                weapon.projectile_frame = projectile_offset.map(|offset| {
                    (
                        Vec3::from_array(equipment_frame.position)
                            + Mat3::from_cols_array_2d(&equipment_frame.basis) * offset,
                        Mat3::from_cols_array_2d(&equipment_frame.basis),
                    )
                });
            }
            weapon.fire_source = Some(source);
            weapon.bevy_backend_projectile_ready = true;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        owner.error = Some(error);
    }
    world.insert_resource(state);
    world.insert_resource(owner);
}

fn fire(
    world: &mut World,
    owner: &mut Firing,
    weapon: &Ranged,
    _ammunition: usize,
    source: FireSource,
    frame: (Vec3, Mat3),
    parameters: crate::gameplay::player::weapons::FireParameters,
    samples: [f32; 3],
    phase: &mut u8,
) -> Result<(), String> {
    if *phase == 0 {
        muzzle_effects(world, owner, weapon, &source)?;
        *phase = 1;
    }
    //44e0c0 queues real spread-area damage before the optional projectile.
    if *phase == 1 {
        if let Some(declaration) = source.spread.clone() {
            super::explosions::enqueue(
                world,
                crate::gameplay::explosions::Request {
                    declaration,
                    inflictor: Some(source.equipment),
                    attacker: Some(source.attacker),
                    position: source.eye + source.camera_axis * weapon.spread_area_offset,
                    direction: source.camera_axis.x_axis,
                    cannot_incapacitate_player_and_allies: false,
                },
            )?;
        }
        *phase = 2;
    }
    if *phase == 2 {
        if weapon.family != RangedFamily::Gun || source.upgrade_flags & 0x20 == 0 {
            let original_ndc = source.clip_from_world.project_point3(source.aim);
            let screen = shooting::dispersed_screen_samples(
                original_ndc.truncate(),
                source.viewport,
                source.minimum_dispersion,
                source.maximum_dispersion,
                source.dispersion_ratio,
                source.extra_dispersion_chance,
                samples,
            );
            let ndc = Vec3::new(
                screen.x / source.viewport.x * 2.0 - 1.0,
                1.0 - screen.y / source.viewport.y * 2.0,
                0.0,
            );
            let aim = shooting::dispersed_aim(
                source.eye,
                source.aim,
                source.world_from_clip.project_point3(ndc),
            );
            let position = sight_origin(world, owner, source.eye, source.camera_axis, frame.0,
                source.weapon_axis, source.prepared.sight_bounds, weapon.aim.max_deviation_angle, aim)?;
            let axis = crate::gameplay::reference_frame::direction_basis(aim - position);
            let charged = source.upgrade_flags & 4 == 0 || parameters.charged_shot;
            let flags = crate::gameplay::projectiles::Flags {
                can_pin_limbs: charged && source.upgrade_flags & 0x40 != 0,
                ignore_armor: charged && source.upgrade_flags & 0x80 != 0,
                ignore_knocked_off_armor: source.upgrade_flags & 0x100 != 0,
                pass_through_npcs: charged && source.upgrade_flags & 0x200 != 0,
                electrified_damage: source.upgrade_flags & 0x400 != 0,
                bounce_once_instead_of_breaking: source.upgrade_flags & 0x800 != 0,
                bypass_big_hit_immunity: source.bypass_big_hit_immunity,
                always_break_on_hit: source.always_break_on_hit,
                ..Default::default()
            };
            let request = crate::gameplay::projectiles::SpawnRequest {
                attacker: Some(source.attacker),
                equipment: Some(source.equipment),
                frame: ReferenceFrame {
                    position: position.to_array(),
                    basis: axis.to_cols_array_2d(),
                },
                velocity: axis.x_axis
                    * source.prepared.declaration.linear_velocity
                    * source.speed_multiplier,
                gravity_multiplier: source.gravity_multiplier,
                damage_increment: source.damage_increment,
                sleep_timer_multiplier: source.sleep_timer_multiplier,
                flags,
                homing_target: None,
                explosion_fx_override: source.explosion_fx_override,
            };
            super::projectiles::spawn(world, source.prepared, request)?;
        }
        *phase = 3;
    }

    Ok(())
}

fn muzzle_effects(
    world: &mut World,
    owner: &mut Firing,
    weapon: &Ranged,
    source: &FireSource,
) -> Result<(), String> {
    //Validate the only fallible binding before publishing either effect.
    //Retries must not duplicate a world muzzle flash after a binding error.
    let attached_name = if weapon.family == RangedFamily::Gun && weapon.muzzle_attached_fx.is_some()
    {
        Some(
            world
                .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .and_then(|live| live.get(source.equipment))
                .map(|(name, _)| name.to_owned())
                .ok_or("fired equipment has no actual managed name for its muzzle effect")?,
        )
    } else {
        None
    };
    if !owner.owned.contains_key(&source.equipment) {
        return Err("fired equipment has no retained actual owner".into());
    }
    if let Some(path) = source.muzzle_world_fx.as_deref() {
        owner.next_effect = owner.next_effect.wrapping_add(1);
        let key: Arc<str> = Arc::from(format!("weapon_muzzle:{}", owner.next_effect));
        if let Some(entry) = owner
            .owned
            .values_mut()
            .find(|entry| entry.entity == source.equipment)
        {
            let ticket = world
                .resource_mut::<super::usable_effects::EffectRequests>()
                //63b3c0/87cb90: detached muzzle FX retain439480 with
                //callback mask8000 and the firing weapon as its payload.
                //The temporary FX actor starts with zero motion; this is
                //not the weapon's or attacker's inherited velocity.
                .start_context_once(key.clone(), path, super::usable_effects::EntityPlacement {
                    target: crate::gameplay::event_graph::EntityReference::Invalid,
                    tag: String::new(), local: ReferenceFrame::default(), attached: false,
                }, source.muzzle_frame, crate::rendering::bevy_backend::world_effects::EntityEffectContext {
                    owner: Some(source.equipment), culprit: None,
                    cannot_incapacitate_player_and_allies: false, player_impact: false,
                    velocities: [[0.0; 3]; 2], contact_speed: 0.0, contact_mass: 0.0,
                    contact_stimulus_enabled: Some(true),
                });
            entry.effects.push(owned::Effect {
                key,
                ticket,
                attached: false,
            });
        }
    }
    if weapon.family == RangedFamily::Gun {
        if let Some(path) = weapon.muzzle_attached_fx.as_deref() {
            let name = attached_name.expect("attached binding was validated before publication");
            owner.next_effect = owner.next_effect.wrapping_add(1);
            let key: Arc<str> = Arc::from(format!("weapon_barrel:{}", owner.next_effect));
            let placement = super::usable_effects::EntityPlacement {
                target: crate::gameplay::event_graph::EntityReference::Named(name),
                tag: weapon.muzzle_tag.clone().unwrap_or_default(),
                local: Default::default(),
                attached: true,
            };
            let ticket = world
                .resource_mut::<super::usable_effects::EffectRequests>()
                .start_entity_once(key.clone(), path, placement);
            if let Some(entry) = owner
                .owned
                .values_mut()
                .find(|entry| entry.entity == source.equipment)
            {
                entry.effects.push(owned::Effect {
                    key,
                    ticket,
                    attached: true,
                });
            }
        }
    }
    Ok(())
}

/// Drain immediately after the player frame and before equipment callbacks
/// select another component. Every message already retained its true owner.
pub(super) fn publish(world: &mut World) {
    let Some(mut owner) = world.remove_resource::<Firing>() else {
        return;
    };
    let Some(mut state) = world.remove_resource::<super::player::PlayerState>() else {
        world.insert_resource(owner);
        return;
    };
    if let Some(player) = state.player.as_mut() {
        for message in player.drain_weapon_effects() {
            owner.deliveries.push_back(Delivery { message, phase: 0 });
        }
    }
    while let Some(mut delivery) = owner.deliveries.pop_front() {
        if let Err(error) = dispatch(world, &mut owner, &delivery.message, &mut delivery.phase) {
            owner.error = Some(error);
            owner.deliveries.push_front(delivery);
            break;
        }
    }
    world.insert_resource(state);
    world.insert_resource(owner);
}
fn dispatch(
    world: &mut World,
    owner: &mut Firing,
    message: &WeaponMessage,
    phase: &mut u8,
) -> Result<(), String> {
    let entity = message
        .entity
        .ok_or("weapon callback has no actual managed component parent")?;
    if world
        .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
        .is_none_or(|live| {
            live.stage(entity) != Some(crate::gameplay::entity_lifetime::Stage::Spawned)
        })
    {
        return Ok(());
    }
    use super::equipment_attachment_admission as children;
    match message.effect.clone() {
        WeaponEffect::Fire {
            weapon,
            ammunition,
            source,
            frame,
            parameters,
            dispersion_samples,
        } => fire(
            world,
            owner,
            &weapon,
            ammunition,
            source,
            frame,
            parameters,
            dispersion_samples,
            phase,
        ),
        WeaponEffect::MagazineChanged { previous, current } => {
            children::update_pending_magazine(world, entity, current);
            let visible = match message.weapon.family {
                RangedFamily::Gun => current > 0 && current >= previous,
                RangedFamily::Crossbow => current > 0,
            };
            children::set_visible(world, entity, visible)
        }
        WeaponEffect::ShowLoadedAttachment => children::set_visible(world, entity, true),
        WeaponEffect::AnimateUpgradeForFire => children::animate_upgrade(world, entity, 3, true),
        WeaponEffect::AnimateUpgradeForReload => {
            if *phase == 0 {
                children::animate_upgrade(world, entity, 4, true)?;
                *phase = 1;
            }
            if message.weapon.family == RangedFamily::Gun && message.capacity > 1 {
                children::magazine_reload(
                    world,
                    entity,
                    &message.weapon,
                    message.capacity,
                    message.total_ammo,
                )?;
            }
            Ok(())
        }
        WeaponEffect::AnimateMagazineForFire => {
            children::magazine_fire(world, entity, &message.weapon, message.magazine)
        }
        WeaponEffect::UpgradeAnimation { mode, play, .. } => {
            children::animate_upgrade(world, entity, mode, play)
        }
        WeaponEffect::MagazineCapacityRecomputed {
            weapon, magazine, ..
        } => children::queue_magazine_final(world, entity, weapon, magazine),
        WeaponEffect::AmmunitionChanged { current, .. } => {
            let Some(entry) = owner.owned.get(&entity) else {
                return Err("ammo callback has no retained owned equipment prototype".into());
            };
            let ammo = &message.weapon.ammunition[current];
            children::select(
                world,
                entity,
                entry.descriptor.clone(),
                &ammo.item,
                ammo.attachment.as_deref(),
                message.weapon.projectile_attachment_tag.as_deref(),
                true,
            )
        }
        // An empty actual sensor target list remains empty; world target
        // acquisition owns its source request and never fabricates actors.
        WeaponEffect::Targeting { .. } => Ok(()),
    }
}
