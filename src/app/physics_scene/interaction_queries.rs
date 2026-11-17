//! 3b8d60/3b7b90 sphere sensors over the canonical admitted-body world.
//! The navigation adapter remains available for its static interaction owners;
//! this bridge admits real Pickable generations, including runtime projectiles.
use super::*;
use crate::{
    assets::entities::usable::Sensor,
    gameplay::{
        collision_filter::CollisionQueryFilter,
        collision_hit::ShapeHitInfo,
        collision_user_data::CollisionUserData,
        entity_lifetime::{EntityId, LiveEntities, Stage},
        interaction_sensor::{Hit, Scratch},
        shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
        world_convex_query::{WorldConvexQuery, WorldConvexRequest},
    },
};
use bevy_math::Vec3;

/// Numeric query scratch only. No geometry, material or actor resources are
/// acquired or cached here; the existing level world remains their owner.
#[derive(Default)]
pub(in crate::app) struct InteractionQueries {
    query: WorldConvexQuery,
    hits: Vec<ConvexContact>,
    initial: Vec<ConvexContact>,
}
impl InteractionQueries {
    fn cast(
        &mut self,
        world: &LevelPointQueries,
        start: Vec3,
        displacement: Vec3,
        radius: f32,
        layers: u32,
        required_flags: u32,
    ) -> Result<(), String> {
        //3b82c0 constructs radius-only spheres. The native query shape type
        //is2; W stores the support ID, not a fabricated polygonal sphere.
        let vertex = [[0., 0., 0., 0.5]];
        world.convex_hits(
            &mut self.query,
            &WorldConvexRequest {
                cast: ConvexCast {
                    geometry: ConvexQueryGeometry {
                        shape_type: 2,
                        vertices: &vertex,
                        radius,
                    },
                    transform: [
                        [1., 0., 0., 0.],
                        [0., 1., 0., 0.],
                        [0., 0., 1., 0.],
                        [start.x, start.y, start.z, 0.],
                    ],
                    displacement: displacement.to_array(),
                    tolerance: f32::from_bits(0x3a83_126f),
                    start_tolerance: 0.,
                    hit: ShapeHitInfo {
                        body_id: None,
                        material_id: u16::MAX,
                        shape_key: u32::MAX,
                        filter_info: 0,
                        user_data: 0,
                    },
                    filter: Some(CollisionQueryFilter {
                        layers,
                        required_flags,
                    }),
                },
                maximum_fraction: 1.,
            },
            &mut self.hits,
            &mut self.initial,
        )?;
        //c89bc0's start collector is null in3b7b90; the async sensor also
        //uses the cast collector. Initial overlaps are not extra candidates.
        self.hits
            .sort_unstable_by(|a, b| a.fraction.total_cmp(&b.fraction));
        Ok(())
    }
}
impl PhysicsScene {
    ///On-demand action diagnostic over the same retained bodies and source
    ///sphere casts. It changes no component, admission flag or query result.
    pub(in crate::app) fn describe_interaction_fixture(
        &self,live:&LiveEntities,target:EntityId,eye:Vec3,forward:Vec3,sensor:Sensor,
    )->Result<String,String> {
        use std::fmt::Write;
        let mut out=String::new();
        writeln!(out,"identity={:?} stage={:?} parent={:?} frame={:?}",live.get(target),live.stage(target),live.parent(target),self.entity_frame(live,target)).unwrap();
        if let Some(&index)=self.shared_indices.get(&target) {
            let entity=&self.entities[index];
            writeln!(out,"scene index={index} retired={} flagsD0D1={:#x} D4={:#x} D7={:#x} timespace={} sensor={sensor:?}",entity.retired,entity.entity_flags,self.entity_flags_d4[index],entity.entity_flags_d7,entity.timespace).unwrap();
            for &packed in entity.components.entries.values() {
                let h=ComponentHandle::from_packed(packed)?;
                let owner=self.owner(h)?;
                writeln!(out,"component={h:?} generation={} flags={:#x} scheduling={:?}",owner.generation,owner.flags,self.declarations.get(&h.packed()).map(|d|&d.scheduling)).unwrap();
                if h.class_id==self.classes[6] {
                    let p=self.pickables.get(h.slot).ok_or("diagnostic Pickable absent")?;
                    writeln!(out,"pickable enabled={} rawD0={:#x} interaction={:?} definition={:?} item={:?} player={:?} references=audio:{:?}/physics:{:?}/system:{:?}/mark:{:?}",p.interaction_enabled,p.interaction_flags,p.interaction,p.definition,p.item.as_ref().map(|i|(&i.path,i.type_number,i.note_type)),p.player,p.references.as_ref().and_then(|r|r.player_audio),p.references.as_ref().and_then(|r|r.physics_base),p.references.as_ref().and_then(|r|r.physics_system),p.references.as_ref().and_then(|r|r.outsider_mark)).unwrap();
                }
            }
            if let Some(h)=self.document_rigid(index)? {
                let body_id=self.rigid_component(h)?.body();
                let world=self.world.read().map_err(|_|"fixture world poisoned")?;
                writeln!(out,"rigid={h:?} body={body_id:?} record={:?}",body_id.and_then(|id|world.storage.bodies.get(id)).map(|b|(b.flags,b.filter_info,b.broadphase_id,b.user_data,b.motion_id))).unwrap();
            }
        }
        writeln!(out,"production_pickable={:?} simple_reference={:?} interactable={:?}",self.interaction_pickable(live,target).map(|p|p.map(|(r,_)|r)),self.grapple_pickable(live,target).map(|p|p.map(|(r,_)|r)),self.grapple_has_interactable(live,target)).unwrap();
        let world=self.world.read().map_err(|_|"fixture query world poisoned")?;
        world.ready().map_err(str::to_owned)?;
        let mut scratch=InteractionQueries::default();
        for group in 0..5 {
            let radius=sensor.shape_radii[group];
            if radius<=0.||radius>sensor.max_distance {continue;}
            let start=eye+forward*radius;
            scratch.cast(&world,start,(start+forward*(sensor.max_distance-radius))-start,radius,if group<3{0x20cc38}else{0x408},0)?;
            writeln!(out,"raw_cast group={group} radius={radius} count={} initial_count={}",scratch.hits.len(),scratch.initial.len()).unwrap();
            let target_hit=scratch.hits.iter().find(|h|self.interaction_hit_entity(live,h).ok().flatten()==Some(target)).map(|h|h.position);
            for hit in scratch.hits.iter().take(64) {
                let actor=self.interaction_hit_entity(live,hit);
                writeln!(out,"raw_hit fraction={} point={:?} normal={:?} target={:?} entity={actor:?} transparentMaterial={:?} breakable={:?}",hit.fraction,hit.position,hit.normal,hit.target,world.storage.materials.material_for_id(hit.target.material_id).map(|m|m.can_blink_through()),actor.as_ref().ok().and_then(|id|*id).map(|id|self.is_breakable(live,id))).unwrap();
            }
            if let Some(point)=target_hit {
                scratch.cast(&world,eye,Vec3::from_array(point[..3].try_into().unwrap())-eye,radius,0xc43e,4)?;
                writeln!(out,"occlusion group={group} count={} initial_count={}",scratch.hits.len(),scratch.initial.len()).unwrap();
                for hit in scratch.hits.iter().take(64) {
                    writeln!(out,"occlusion_hit fraction={} point={:?} target={:?} entity={:?}",hit.fraction,hit.position,hit.target,self.interaction_hit_entity(live,hit)).unwrap();
                }
            }
        }
        Ok(out)
    }
    fn interaction_hit_entity(
        &self,
        live: &LiveEntities,
        contact: &ConvexContact,
    ) -> Result<Option<EntityId>, String> {
        self.collision_entity(live, &contact.target)
    }
    ///The shared source userdata bridge preserves the managed generation for
    ///ranged sensor9 as well as the interaction cast's convex witnesses.
    pub(in crate::app) fn collision_entity(&self, live: &LiveEntities,
        target: &crate::gameplay::collision_hit::ShapeHitInfo) -> Result<Option<EntityId>, String> {
        if target.user_data == 0 {
            return Ok(None);
        }
        let index = CollisionUserData::decode(target.user_data)?
            .entity(|handle| self.owner(handle).ok())?;
        let entity = self
            .entities
            .get(index)
            .ok_or("sensor collision actor slot absent")?;
        if entity.retired || live.stage(entity.shared) != Some(Stage::Spawned) {
            return Err("sensor collision actor generation is not spawned".into());
        }
        Ok(Some(entity.shared))
    }

    /// Appends to the matching static cast groups, preserving source group
    /// priority and nearest-first ordering without a separate actor index.
    pub(in crate::app) fn cast_interaction_sensor(
        &self,
        live: &LiveEntities,
        eye_game: Vec3,
        forward_game: Vec3,
        sensor: Sensor,
        scratch: &mut InteractionQueries,
        output: &mut Scratch,
    ) -> Result<(), String> {
        if forward_game == Vec3::ZERO {
            return Ok(());
        }
        let world = self
            .world
            .read()
            .map_err(|_| "interaction sensor world poisoned")?;
        world.ready().map_err(str::to_owned)?;
        for (group, hits) in output.hits.iter_mut().enumerate() {
            let radius = sensor.shape_radii[group];
            if radius <= 0. || radius > sensor.max_distance {
                continue;
            }
            let start = eye_game + forward_game * radius;
            //3b8d60 computes endpoint minus origin after camera translation,
            //rather than discarding that source floating-point subtraction.
            let displacement = (start + forward_game * (sensor.max_distance - radius)) - start;
            scratch.cast(
                &world,
                start,
                displacement,
                radius,
                if group < 3 { 0x20cc38 } else { 0x408 },
                0,
            )?;
            for contact in &scratch.hits {
                let Some(entity) = self.interaction_hit_entity(live, contact)? else {
                    continue;
                };
                if self.interaction_pickable(live, entity)?.is_none()
                    && self.interaction_usable(live, entity)?.is_none()
                {
                    continue;
                }
                //61e1a0: transparent collision material AND a real Breakable
                //component, not a name/tag or the material bit alone.
                if self.is_breakable(live, entity)
                    && world
                        .storage
                        .materials
                        .material_for_id(contact.target.material_id)?
                        .can_blink_through()
                {
                    continue;
                }
                hits.push(Hit {
                    entity: Some(entity),
                    key: contact.target.shape_key,
                    owner: u32::MAX,
                    fraction: contact.fraction,
                    position: super::super::game_to_view(contact.position[..3].try_into().unwrap()),
                });
            }
            hits.sort_unstable_by(|a, b| a.fraction.total_cmp(&b.fraction));
        }
        Ok(())
    }

    /// Source3b7b90's Pickable occlusion result. Transparent Breakables still
    /// return blocked=true: its additional out-byte is only for NPC highlighting.
    /// Pickable admission in3b8430 requires the returned blocked flag be false.
    pub(in crate::app) fn interaction_occluded(
        &self,
        live: &LiveEntities,
        target: EntityId,
        eye_game: Vec3,
        hit_game: Vec3,
        radius: f32,
        scratch: &mut InteractionQueries,
    ) -> Result<bool, String> {
        if live.stage(target) != Some(Stage::Spawned) {
            return Err("interaction occlusion target generation stale".into());
        }
        let world = self
            .world
            .read()
            .map_err(|_| "interaction occlusion world poisoned")?;
        world.ready().map_err(str::to_owned)?;
        scratch.cast(&world, eye_game, hit_game - eye_game, radius, 0xc43e, 4)?;
        for contact in &scratch.hits {
            if let Some(entity) = self.interaction_hit_entity(live, contact)? {
                if entity == target || live.parent(entity)? == Some(target) {
                    return Ok(false);
                }
                //878c70 ignores committed context4 before its NPC-specific
                //state branch. The currently admitted Pickable actor families
                //have no NPC+368 payload; do not invent that missing state.
                if live.entity_context(entity)?.current == 4 {
                    continue;
                }
                let &index = self
                    .shared_indices
                    .get(&entity)
                    .ok_or("occlusion scene actor absent")?;
                if self.entities[index].entity_flags & 0x2800 != 0 {
                    let (_, mut class) = live.get(entity).ok_or("occlusion actor stale")?;
                    loop {
                        let classes = self.types.classes();
                        let metadata = classes
                            .metadata(class)
                            .ok_or("occlusion actor class absent")?;
                        if classes.name_text(metadata.name) == Some(b"arkNPC".as_slice()) {
                            return Err(
                                "interaction occlusion NPC state requires its admitted NPC owner"
                                    .into(),
                            );
                        }
                        let Some(base) = classes.hierarchy().node(class).and_then(|node| node.base)
                        else {
                            break;
                        };
                        class = base;
                    }
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}
