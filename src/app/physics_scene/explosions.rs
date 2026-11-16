//! Explosion manager queries over the same level bodies, shapes and health.
use super::*;
use crate::gameplay::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    collision_user_data::CollisionUserData,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities, Stage},
    explosions::{RadiusModifiers, Request},
    shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
    shape_ray_query::ShapeRayContact,
    world_convex_query::{WorldConvexQuery, WorldConvexRequest},
    world_ray_query::{WorldRayQuery, WorldRayRequest},
};
use bevy_math::Vec3;
struct Group {
    entity: EntityId,
    bodies: Vec<u32>,
}
struct Action {
    group: usize,
    params: crate::gameplay::damage::Parameters,
    impulse: Vec3,
}
#[derive(Default)]
pub(in crate::app) struct ExplosionQueries {
    convex: WorldConvexQuery,
    hits: Vec<ConvexContact>,
    initial: Vec<ConvexContact>,
    ray: WorldRayQuery,
    ray_hits: Vec<ShapeRayContact>,
    groups: Vec<Group>,
    used: usize,
    samples: Vec<Vec3>,
    actions: Vec<Action>,
}
impl PhysicsScene {
    pub(in crate::app) fn explosion_queries_ready(&self) -> Result<bool, String> {
        let world = self
            .world
            .read()
            .map_err(|_| "explosion canonical world poisoned")?;
        Ok(self.registry.is_some() && world.storage.publication_idle())
    }
    pub(in crate::app) fn execute_explosion(
        &mut self,
        queries: &mut ExplosionQueries,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        request: &Request,
        modifiers: RadiusModifiers,
        mut bounds: impl FnMut(&LiveEntities, EntityId) -> Result<Option<[[f32; 3]; 2]>, String>,
    ) -> Result<(), String> {
        let d = &request.declaration;
        if d.attacker_always_player {
            return Err("explosion attacker override requires actual player identity source".into());
        }
        let hit = ShapeHitInfo {
            body_id: None,
            material_id: u16::MAX,
            shape_key: u32::MAX,
            filter_info: 0,
            user_data: 0,
        };
        let vertex = [[0., 0., 0., 0.5]];
        let p = request.position;
        let cast = ConvexCast {
            geometry: ConvexQueryGeometry {
                shape_type: 2,
                vertices: &vertex,
                radius: request.query_radius(modifiers),
            },
            transform: [
                [1., 0., 0., 0.],
                [0., 1., 0., 0.],
                [0., 0., 1., 0.],
                [p.x, p.y, p.z, 1.],
            ],
            displacement: [0.; 3],
            tolerance: f32::from_bits(0x3a83126f),
            start_tolerance: 0.,
            hit,
            filter: Some(CollisionQueryFilter {
                layers: d.collision_layers,
                required_flags: 0,
            }),
        };
        let world = self
            .world
            .read()
            .map_err(|_| "explosion canonical world poisoned")?;
        world.convex_hits(
            &mut queries.convex,
            &WorldConvexRequest {
                cast,
                maximum_fraction: 1.,
            },
            &mut queries.hits,
            &mut queries.initial,
        )?;
        queries.used = 0;
        for contact in &queries.initial {
            let Some(body_id) = contact.target.body_id else {
                continue;
            };
            let body = world
                .storage
                .bodies
                .get(body_id)
                .ok_or("explosion candidate body generation stale")?;
            if body.flags & 3 == 0 || contact.target.user_data == 0 {
                continue;
            }
            let index = CollisionUserData::decode(contact.target.user_data)?
                .entity(|h| self.owner(h).ok())?;
            let id = self
                .entities
                .get(index)
                .ok_or("explosion candidate actor absent")?
                .shared;
            if live.stage(id) != Some(Stage::Spawned) {
                return Err("explosion candidate actor is not spawned".into());
            }
            if d.filter_inflictor && request.inflictor == Some(id)
                || d.filter_attacker && request.attacker == Some(id)
            {
                continue;
            }
            if self.explosion_actor_class(live,id,b"arkNPC")? {return Err("area damage NPC requires actual NPC and target-point owners".into())}
            if self.explosion_actor_class(live,id,b"arkPlayer")? {return Err("area damage player requires actual Player damage callback owner".into())}
            if d.filter_neutral && request.attacker.is_some_and(|attacker| attacker != id) {
                if d.filter_carried {
                    if let Some(parent)=live.parent(id)? {
                        if self.explosion_actor_class(live,parent,b"arkPlayer")? {continue}
                    }
                }
                let manager = self
                    .faction_manager
                    .as_ref()
                    .ok_or("explosion faction manager unavailable")?;
                if manager
                    .manager()
                    .relationship(id, request.attacker.unwrap())
                    == 0
                {
                    continue;
                }
            }
            let group = if let Some(group) = queries.groups[..queries.used]
                .iter()
                .position(|g| g.entity == id)
            {
                group
            } else {
                let group = queries.used;
                queries.used += 1;
                if group == queries.groups.len() {
                    queries.groups.push(Group {
                        entity: id,
                        bodies: Vec::new(),
                    })
                }
                queries.groups[group].entity = id;
                queries.groups[group].bodies.clear();
                group
            };
            //01d6b0 appends the collector's body entries; source duplicates
            //are retained rather than replaced with a guessed body dedup.
            queries.groups[group].bodies.push(body_id);
        }
        drop(world);
        queries.actions.clear();
        for (group_index, group) in queries.groups[..queries.used].iter().enumerate() {
            let Some(aabb) = bounds(live, group.entity)? else {
                return Err("explosion target lacks its actual entity RenderModel bounds".into());
            };
            let aabb = [Vec3::from_array(aabb[0]), Vec3::from_array(aabb[1])];
            let center = (aabb[0] + aabb[1]) * 0.5;
            queries.samples.clear();
            if let Some(handle) = self.explosion_health_handle(live, group.entity)? {
                let receiver = &self
                    .health
                    .get(handle.slot)
                    .ok_or("explosion Health payload absent")?
                    .receiver;
                if !receiver.explosion_target_points.is_empty() {
                    return Err(
                        "explosion named target points require actual admitted animation-tag owner"
                            .into(),
                    );
                }
                if receiver.explosion_aabb_samples.is_empty() {
                    queries.samples.push(center)
                } else {
                    let half = (aabb[1] - aabb[0]) * 0.5;
                    queries.samples.extend(
                        receiver
                            .explosion_aabb_samples
                            .iter()
                            .map(|point| center + half * *point),
                    );
                }
            } else {
                queries.samples.push(center)
            }
            if !request.in_cone(aabb, &queries.samples) {
                continue;
            }
            let mut occluded = d.occlusion_layers != 0;
            if occluded {
                let world = self
                    .world
                    .read()
                    .map_err(|_| "explosion occlusion world poisoned")?;
                for sample in &queries.samples {
                    let ray = WorldRayRequest {
                        origin: request.position.to_array(),
                        direction: (*sample - request.position).to_array(),
                        maximum_fraction: 1.,
                        hit,
                        filter: Some(CollisionQueryFilter {
                            layers: d.occlusion_layers,
                            required_flags: 0x800,
                        }),
                    };
                    world.ray_hits(&mut queries.ray, &ray, 2, &mut queries.ray_hits)?;
                    let closest = queries
                        .ray_hits
                        .iter()
                        .min_by(|a, b| a.fraction.total_cmp(&b.fraction));
                    let blocked = if let Some(contact) = closest {
                        if contact.target.user_data == 0 {
                            true
                        } else {
                            let index = CollisionUserData::decode(contact.target.user_data)?
                                .entity(|h| self.owner(h).ok())?;
                            self.entities
                                .get(index)
                                .ok_or("explosion occluder actor absent")?
                                .shared
                                != group.entity
                        }
                    } else {
                        false
                    };
                    if !blocked {
                        occluded = false;
                        break;
                    }
                }
            }
            if occluded {
                continue;
            }
            if let Some((params, impulse)) = request.damage(aabb, center, false, modifiers, false) {
                queries.actions.push(Action {
                    group: group_index,
                    params,
                    impulse,
                });
            }
        }
        for action in &queries.actions {
            let group = &queries.groups[action.group];
            self.damage_entity(live, events, group.entity, &action.params)?;
            for &body in &group.bodies {
                self.explosion_body_impulse(body, action.impulse)?;
            }
        }
        Ok(())
    }
    fn explosion_actor_class(&self,live:&LiveEntities,id:EntityId,name:&[u8])->Result<bool,String>{
        let(_,mut class)=live.get(id).ok_or("explosion actor class generation stale")?;
        loop{let classes=self.types.classes();let metadata=classes.metadata(class).ok_or("explosion actor class metadata absent")?;
            if classes.name_text(metadata.name)==Some(name){return Ok(true)}
            let Some(base)=classes.hierarchy().node(class).and_then(|node|node.base)else{return Ok(false)};class=base;
        }
    }
    fn explosion_health_handle(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<ComponentHandle>, String> {
        //Health lookup must retain the same owner/generation and active
        //component flags as ordinary damage delivery.
        if live.stage(id) != Some(Stage::Spawned) {
            return Err("explosion Health identity stale".into());
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        self.entities[index]
            .components
            .find_component([self.classes[10]; 2], [0x20, 0], |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(handle)?.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|d| d.scheduling.start_phases),
                    },
                )
            })
    }
    fn explosion_body_impulse(&mut self, body: u32, impulse: Vec3) -> Result<(), String> {
        let mut registry = self
            .registry
            .take()
            .ok_or("explosion component registry unavailable")?;
        let result = (|| {
            let shared = self.world.clone();
            scene_publication_events!(self, &mut registry, publication);
            let mut world = shared
                .write()
                .map_err(|_| "explosion impulse world poisoned")?;
            let world = &mut *world;
            world.storage.apply_body_impulse(
                &mut world.geometry,
                &mut publication,
                body,
                [impulse.x, impulse.y, impulse.z, 0.],
                None,
            )
        })();
        self.registry = Some(registry);
        result
    }
}
