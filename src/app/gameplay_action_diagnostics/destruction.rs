//! Windowless physical input for the reported shooting locations in Edge of the World.
use super::super as app;
use super::{Input, PlayerState, RangedFamily, bolt, input_for, player, send};
use crate::gameplay::{
    entity_lifetime::{EntityId, LiveEntities},
    world_ray_query::{WorldRayQuery, WorldRayRequest},
};
use bevy::prelude::*;
use std::time::{Duration, Instant};

/// Patrol waypoints need no gameplay component or rigid body. Borrow their
/// small placement records from the existing map parse only for these actions.
#[derive(Resource)]
pub(in crate::app) struct Waypoints(Vec<Vec3>);

impl Waypoints {
    pub(in crate::app) fn capture(
        document: &crate::entities::Document,
    ) -> Result<Option<Self>, String> {
        if !matches!(
            std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(),
            Ok("gun-speaker" | "gun-nest" | "bolt-oil")
        ) {
            return Ok(None);
        }
        let mut points = Vec::new();
        for def in &document.definitions {
            if def.kind == "entity"
                && crate::gameplay::options::decl::atom(&def.fields, "class") == Some("arkWaypoint")
            {
                points.push(Vec3::from(crate::entities::spawn_frame(def)?.position));
            }
        }
        Ok(Some(Self(points)))
    }
}

/// Select a real patrol waypoint with an unobstructed projectile ray. Only the
/// initial pose is restored. Live aiming and the projectile still run normally.
pub(super) fn start_at_visible_waypoint(
    world: &mut World,
    target: EntityId,
) -> Result<bool, String> {
    let scene = world.resource::<app::physics_scene::PhysicsScene>();
    let live = world.resource::<LiveEntities>();
    let center = bolt::Observation::center(world, target)?;
    let p = player(world)?;
    let eye_offset = p.published_camera_frame().0 - p.feet();
    let waypoints = world
        .get_resource::<Waypoints>()
        .ok_or("missing diagnostic waypoint inventory")?;
    let mut points: Vec<_> = waypoints
        .0
        .iter()
        .copied()
        .map(|feet| (center.distance_squared(feet + eye_offset), feet))
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let owner = scene.point_queries()?;
    let queries = owner
        .read()
        .map_err(|_| "diagnostic waypoint query poisoned")?;
    let mut ray = WorldRayQuery::default();
    let mut hits = Vec::new();
    let mut pose = None;
    for (_, feet) in points {
        let eye = feet + eye_offset;
        // Center may be an authored pivot above or below the actual model.
        let direction = center - eye;
        hits.clear();
        queries.ray_hits(
            &mut ray,
            &WorldRayRequest {
                origin: eye.to_array(),
                direction: (direction * 1.05).to_array(),
                maximum_fraction: 1.,
                hit: crate::gameplay::collision_hit::ShapeHitInfo {
                    body_id: None,
                    material_id: u16::MAX,
                    shape_key: u32::MAX,
                    filter_info: 0,
                    user_data: 0,
                },
                filter: Some(crate::gameplay::collision_filter::CollisionQueryFilter {
                    layers: p.ranged_target.layers,
                    required_flags: 0x20000,
                }),
            },
            0,
            &mut hits,
        )?;
        if let Some(hit) = hits.iter().min_by(|a, b| a.fraction.total_cmp(&b.fraction)) {
            if scene.collision_entity(live, &hit.target)? == Some(target) {
                eprintln!(
                    "ACTION_SHOOT_WAYPOINT target={:?} authored_waypoint_feet={feet:?} target_center={center:?} nearest_hit={:?}",
                    live.get(target),
                    hit.position
                );
                pose = Some((feet, direction.y.atan2(direction.x)));
                break;
            }
        }
    }
    drop(queries);
    if let Some((feet, yaw)) = pose {
        world.remove_resource::<Waypoints>();
        world
            .resource_mut::<PlayerState>()
            .player
            .as_mut()
            .ok_or("missing diagnostic player")?
            .teleport(feet, yaw);
        let capture = input_for(19)?;
        send(world, capture, true)?;
        send(world, capture, false)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[derive(Resource)]
struct SpeakerExplosion {
    speaker: EntityId,
    falling_piece: Option<EntityId>,
    damage_complete: bool,
    recipients: usize,
}

pub(in crate::app) fn speaker_chunk_released(world: &mut World, owner: EntityId, child: EntityId) {
    if let Some(mut receipt) = world.get_resource_mut::<SpeakerExplosion>() {
        if receipt.speaker == owner {
            receipt.falling_piece = Some(child);
        }
    }
}

/// Called after the real radial damage loop succeeds. Looking up the source
/// declaration here cannot warm it before the request which already owns it.
pub(in crate::app) fn explosion_completed(
    world: &mut World,
    request: &crate::gameplay::explosions::Request,
    recipients: usize,
) -> Result<(), String> {
    let Some(receipt) = world.get_resource::<SpeakerExplosion>() else {
        return Ok(());
    };
    let Some(inflictor) = request.inflictor else {
        return Ok(());
    };
    if inflictor != receipt.speaker && Some(inflictor) != receipt.falling_piece {
        return Ok(());
    }
    let catalog = world
        .resource::<app::state::SceneData>()
        .catalog
        .as_ref()
        .ok_or("speaker observation catalog absent")?;
    let declaration = crate::assets::explosions::Declaration::acquire(
        catalog,
        "components/base/explosion/explosion_streetspeaker.explosion",
    )?;
    if std::sync::Arc::ptr_eq(&declaration, &request.declaration) {
        let mut receipt = world.resource_mut::<SpeakerExplosion>();
        receipt.damage_complete = true;
        receipt.recipients = recipients;
        eprintln!(
            "ACTION_SHOOT_SPEAKER_EXPLOSION inflictor={:?} position={:?} actual_radial_targets={recipients}",
            request.inflictor, request.position
        );
    }
    Ok(())
}

pub(super) struct Observation {
    target: EntityId,
    health: f32,
    speaker: Option<(EntityId, f32)>,
    released: bool,
    fell: bool,
    release: Option<Input>,
    first_ammo: Option<u32>,
    last_shot: Option<u32>,
    destroyed: Option<Instant>,
    since: Instant,
}

impl Observation {
    pub(super) fn begin(world: &mut World, speaker: bool) -> Result<Self, String> {
        let target = if speaker {
            let name = "script_kar_speaker_turnbuckle_01_3";
            let target = world.resource::<LiveEntities>().find(name)
                .ok_or_else(|| format!("missing destruction target {name}"))?;
            if !start_at_visible_waypoint(world, target)? {
                return Err(format!("no visible authored waypoint for {name}"));
            }
            target
        } else {
            let live = world.resource::<LiveEntities>();
            let scene = world.resource::<app::physics_scene::PhysicsScene>();
            let mut nests = Vec::new();
            for &id in live.spawned_entities() {
                if live.get(id).is_some_and(|(name, _)| name.starts_with("env_interiors_nest_bloodflies"))
                    && scene.entity_health(live, id)?.is_some_and(|(health, _)| health > 0.)
                {
                    nests.push(id);
                }
            }
            let mut visible = None;
            for id in nests {
                if start_at_visible_waypoint(world, id)? {
                    visible = Some(id);
                    break;
                }
            }
            visible.ok_or("no placed bloodfly nest visible from an authored waypoint")?
        };
        let live = world.resource::<LiveEntities>();
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        let (health, _) = scene
            .entity_health(live, target)?
            .ok_or("target has no live Health")?;
        let child = if speaker {
            let id = live
                .find("script_speaker_station")
                .ok_or("speaker missing")?;
            if live.parent(id)? != Some(target) {
                return Err(format!(
                    "speaker {id:?} not bound to {target:?} before shot; parent={:?}",
                    live.parent(id)?
                ));
            }
            let pos = scene
                .entity_frame(live, id)?
                .ok_or("speaker frame missing")?
                .position;
            Some((id, pos[2]))
        } else {
            None
        };
        if let Some((speaker, _)) = child {
            world.insert_resource(SpeakerExplosion {
                speaker,
                falling_piece: None,
                damage_complete: false,
                recipients: 0,
            });
        }
        let input = input_for(27)?;
        send(world, input, true)?;
        Ok(Self {
            target,
            health,
            speaker: child,
            released: false,
            fell: false,
            release: Some(input),
            first_ammo: None,
            last_shot: None,
            destroyed: None,
            since: Instant::now(),
        })
    }
    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        let live = world.resource::<LiveEntities>();
        let scene = world.resource::<app::physics_scene::PhysicsScene>();
        let ammo = player(world)?
            .weapon
            .as_ref()
            .filter(|w| w.settings.family == RangedFamily::Gun)
            .map(|w| w.total_ammo);
        let flags = scene.diagnostic_damage_flags(live, self.target)?;
        let health = if live.get(self.target).is_some() {
            scene.entity_health(live, self.target)?.map(|h| h.0)
        } else {
            None
        };
        let destroyed = live.get(self.target).is_none()
            || flags.is_some_and(|f| f & 0x800 != 0)
            || health.is_some_and(|h| h <= 0.);
        if let Some((child, z)) = self.speaker {
            if live.get(child).is_some() {
                self.released |= live.parent(child)? != Some(self.target);
                if let Some(frame) = scene.entity_frame(live, child)? {
                    self.fell |= self.released && frame.position[2] < z;
                }
            }
        }
        if destroyed
            && self.destroyed.is_none()
            && self.first_ammo.zip(ammo).is_some_and(|(a, b)| b < a)
        {
            self.destroyed = Some(Instant::now());
            eprintln!(
                "ACTION_SMOKE_PASS check=target_destroyed_by_physical_pistol target={:?} before={} after={health:?} flags={flags:?} ammo={ammo:?}",
                self.target, self.health
            );
        }
        if let Some(time) = self.destroyed {
            if self.speaker.is_some() {
                let event = world.resource::<SpeakerExplosion>();
                if self.released && self.fell && event.damage_complete {
                    eprintln!(
                        "ACTION_SMOKE_PASS check=speaker_unbind_fall_explosion actual_damage_recipients={}",
                        event.recipients
                    );
                    return Ok(true);
                }
            } else if time.elapsed() > Duration::from_secs(2) {
                return Ok(true);
            }
        }
        if self.since.elapsed() > Duration::from_secs(40) {
            return Err(format!(
                "physical pistol scenario timeout target={:?} aimed={:?} health={health:?} flags={flags:?} ammunition={ammo:?} before={:?} speaker_released={} speaker_fell={} explosion={}",
                self.target,
                player(world)?.ranged_target_entity(),
                self.first_ammo,
                self.released,
                self.fell,
                world
                    .get_resource::<SpeakerExplosion>()
                    .is_some_and(|e| e.damage_complete)
            ));
        }
        if let Some(input) = self.release.take() {
            send(world, input, false)?;
            return Ok(false);
        }
        if self.destroyed.is_some() || !super::Observation::ranged_ready(world, RangedFamily::Gun)?
        {
            return Ok(false);
        }
        let center = bolt::Observation::center(world, self.target)?;
        if !bolt::Observation::steer(world, center)? {
            return Ok(false);
        }
        let Some(ammo) = ammo else { return Ok(false) };
        if player(world)?.ranged_target_entity() == Some(self.target)
            && ammo > 0
            && self.last_shot != Some(ammo)
        {
            self.first_ammo.get_or_insert(ammo);
            self.last_shot = Some(ammo);
            let input = input_for(20)?;
            send(world, input, true)?;
            self.release = Some(input);
            eprintln!(
                "ACTION_SHOOT_PISTOL target={:?} ammo_before={ammo}",
                self.target
            );
        }
        Ok(false)
    }
}
