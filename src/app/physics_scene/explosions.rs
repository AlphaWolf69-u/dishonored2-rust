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
    pub(in crate::app) contact_hits:Vec<ShapeRayContact>,
    convex: WorldConvexQuery,
    hits: Vec<ConvexContact>,
    initial: Vec<ConvexContact>,
    ray: WorldRayQuery,
    ray_hits: Vec<ShapeRayContact>,
    groups: Vec<Group>,
    used: usize,
    samples: Vec<Vec3>,
    actions: Vec<Action>,
    applying: bool,
    action_cursor: usize,
    body_cursor: usize,
    damage_delivered: bool,
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
        force_full_player_owned:bool,
        mut bounds: impl FnMut(&LiveEntities, EntityId) -> Result<Option<[[f32; 3]; 2]>, String>,
    ) -> Result<(), String> {
        let _request_timing = super::super::breakables::measure("explosion_request",
            super::super::breakables::TimingOwner::Entity(request.inflictor));
        if queries.applying {return self.deliver_explosion(queries,live,events);}
        let d = &request.declaration;
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
        let mut overlap_timing = super::super::breakables::measure("explosion_overlap",
            super::super::breakables::TimingOwner::Entity(request.inflictor));
        world.convex_hits(
            &mut queries.convex,
            &WorldConvexRequest {
                cast,
                maximum_fraction: 1.,
            },
            &mut queries.hits,
            &mut queries.initial,
        )?;
        overlap_timing.count(0, queries.hits.len());
        overlap_timing.count(1, queries.initial.len());
        drop(overlap_timing);
        let mut grouping_timing = super::super::breakables::measure("explosion_grouping",
            super::super::breakables::TimingOwner::Entity(request.inflictor));
        queries.used = 0;
        //7b3830 submits095280 with initial-collector flag0.096ac0's
        //all-hits result therefore comes from the MAIN collector only;
        //cbd060 also publishes actual overlaps there. The independent
        //initial collector is not its result (and must not be merged).
        for contact in &queries.hits {
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
        grouping_timing.count(0, queries.hits.len());
        grouping_timing.count(1, queries.used);
        drop(grouping_timing);
        drop(world);
        queries.actions.clear();
        queries.contact_hits.clear();
        let mut targets_timing = super::super::breakables::measure("explosion_target_queries",
            super::super::breakables::TimingOwner::Entity(request.inflictor));
        for (group_index, group) in queries.groups[..queries.used].iter().enumerate() {
            targets_timing.count(0, 1);
            let Some(aabb) = bounds(live, group.entity)? else {
                return Err("explosion target lacks its actual entity RenderModel bounds".into());
            };
            let aabb = [Vec3::from_array(aabb[0]), Vec3::from_array(aabb[1])];
            let center = (aabb[0] + aabb[1]) * 0.5;
            queries.samples.clear();
            if let Some(handle) = self.explosion_health_handle(live, group.entity)? {
                let receiver = &self.health_payload(handle)?.receiver;
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
                    targets_timing.count(1, 1);
                    world.ray_hits(&mut queries.ray, &ray, 2, &mut queries.ray_hits)?;
                    targets_timing.count(2, queries.ray_hits.len());
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
            //7b24f0 consumes the actual material-bearing ray witness. It is
            //independent of damage amount and never substitutes the AABB center.
            if d.contact_material.is_some(){
                let world=self.world.read().map_err(|_|"explosion contact ray world poisoned")?;
                let ray=WorldRayRequest{origin:request.position.to_array(),direction:(queries.samples[0]-request.position).to_array(),maximum_fraction:1.,hit,
                    filter:Some(CollisionQueryFilter{layers:d.occlusion_layers,required_flags:0x800})};
                targets_timing.count(1, 1);
                world.ray_hits(&mut queries.ray,&ray,2,&mut queries.ray_hits)?;
                targets_timing.count(2, queries.ray_hits.len());
                if let Some(contact)=queries.ray_hits.iter().min_by(|a,b|a.fraction.total_cmp(&b.fraction)){queries.contact_hits.push(*contact);}
            }
            if let Some((params, impulse)) = request.damage(aabb, center, false, modifiers, force_full_player_owned) {
                queries.actions.push(Action {
                    group: group_index,
                    params,
                    impulse,
                });
            }
        }
        targets_timing.count(3, queries.actions.len());
        drop(targets_timing);
        queries.applying=true;
        queries.action_cursor=0;
        queries.body_cursor=0;
        queries.damage_delivered=false;
        self.deliver_explosion(queries,live,events)
    }
    /// Host callbacks can fail after earlier recipients succeeded. Preserve
    ///the native grouped recipient plan and completed callbacks rather than
    ///running the spatial query and damaging those recipients a second time.
    fn deliver_explosion(&mut self,queries:&mut ExplosionQueries,live:&mut LiveEntities,events:&mut EntityEvents)->Result<(),String>{
        let mut timing = super::super::breakables::measure("explosion_damage_impulse_delivery",
            super::super::breakables::TimingOwner::Entity(None));
        while queries.action_cursor<queries.actions.len(){
            let action=&queries.actions[queries.action_cursor];
            let group=&queries.groups[action.group];
            if !queries.damage_delivered{
                timing.count(0, 1);
                self.damage_entity(live,events,group.entity,&action.params)?;
                queries.damage_delivered=true;
            }
            while queries.body_cursor<group.bodies.len(){
                timing.count(1, 1);
                self.explosion_body_impulse(group.bodies[queries.body_cursor],action.impulse)?;
                queries.body_cursor+=1;
            }
            queries.action_cursor+=1;
            queries.body_cursor=0;
            queries.damage_delivered=false;
        }
        queries.applying=false;
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
        //7b3830 selects Health sample metadata through877720 mode2;
        //141c70e88[2] is {0x28,0}, as in879270 actual damage delivery.
        if live.stage(id) != Some(Stage::Spawned) {
            return Err("explosion Health identity stale".into());
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        self.entities[index]
            .components
            .find_component(self.health_class_range()?, [0x28, 0], |handle| {
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
        result.map(|_| ())
    }
}
