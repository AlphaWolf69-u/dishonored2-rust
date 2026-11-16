//! HoldObject transactions over the actual Pickable, rigid and reference actor.
use super::*;
use crate::gameplay::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    compound_query::instance::{prepare_instance, CompoundChildGeometry, CompoundShapeState},
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities, Stage},
    pickable_component::CheckedComponent,
    player::{
        carrying::{GrabDecl, HoldDecl, Target},
        Player,
    },
    reference_frame::ReferenceFrame,
    shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
    shape_transform::{ScaledShapeTransform, ShapeMatrix},
    world_convex_query::{WorldConvexQuery, WorldConvexRequest},
};
use bevy::prelude::{Mat3, Mat4, Quat, Vec3};

#[derive(Default)]
struct Progress {
    begin: u8,
    release: u8,
    planned: bool,
    position: Option<Vec3>,
    velocity: Option<Vec3>,
    positioned: bool,
    impulse_posted: bool,
}
#[derive(Resource, Default)]
pub(in crate::app) struct CarryPhysics {
    progress: HashMap<EntityId, Progress>,
    convex: WorldConvexQuery,
    hits: Vec<ConvexContact>,
    starts: Vec<ConvexContact>,
    pending: Vec<(usize, CompoundShapeState)>,
    vertices: Vec<[f32; 4]>,
    sounds: Vec<crate::gameplay::sound::Request>,
    next_effect: u64,
    error: Option<String>,
}
pub(in crate::app) struct CarryHost<'a> {
    pub scene: &'a mut PhysicsScene,
    pub live: &'a mut LiveEntities,
    pub events: &'a mut EntityEvents,
    pub state: &'a mut CarryPhysics,
    pub clock: &'a crate::gameplay::game_clock::GameClock,
    pub bounds: &'a super::super::shadow_updates::ModelShadowSources,
    pub placements: &'a super::super::RenderSourcePlacements,
    pub effects: &'a mut super::super::usable_effects::EffectRequests,
}
impl CarryHost<'_> {
    fn set_position(&mut self, target: &Target, position: Vec3) -> Result<(), String> {
        let (index, _) = self.reference(target)?;
        let mut pose = self.frame(target)?;
        pose.position = position.to_array();
        if let Some(handle) = self.scene.document_rigid(index)? {
            let body = self
                .scene
                .rigid_component(handle)?
                .body()
                .ok_or("carry moved body absent")?;
            let scene = &mut *self.scene;
            let mut registry = scene.registry.take().ok_or("carry move registry absent")?;
            let result = (|| {
                let shared = scene.world.clone();
                let mut guard = shared.write().map_err(|_| "carry move world poisoned")?;
                let world = &mut *guard;
                scene_publication_events!(scene, &mut registry, publication);
                let frame = Mat4::from_cols(
                    Vec3::from_array(pose.basis[0]).extend(0.),
                    Vec3::from_array(pose.basis[1]).extend(0.),
                    Vec3::from_array(pose.basis[2]).extend(0.),
                    position.extend(1.),
                )
                .to_cols_array_2d();
                world.storage.set_body_transform(
                    &mut world.geometry,
                    &mut publication,
                    body,
                    frame,
                    0,
                )?;
                Ok::<_, String>(())
            })();
            scene.registry = Some(registry);
            result?;
        }
        self.scene.entities[index].frame.pose = pose;
        if let Some(handle) = self.scene.entities[index].components.reference_actor {
            self.scene
                .actors
                .actors
                .get_mut(handle.slot)
                .ok_or("moved carry reference actor absent")?
                .set_transform(pose, false);
            let generation = self.scene.actors.actors[handle.slot].owner.generation;
            self.scene
                .request_post_physics_actor(handle.slot as usize, generation)?;
        }
        Ok(())
    }
    fn release_body(
        &mut self,
        target: &Target,
        player: &Player,
        thrown: bool,
    ) -> Result<(), String> {
        let (index, reference) = self.reference(target)?;
        if self
            .state
            .progress
            .entry(target.entity)
            .or_default()
            .release
            == 0
        {
            self.restore_filter(index)?;
            self.state.progress.get_mut(&target.entity).unwrap().release = 1;
        }
        if self.state.progress[&target.entity].release == 1 {
            self.scene
                .unbind_entity(self.live, target.entity, self.events)?;
            self.state.progress.get_mut(&target.entity).unwrap().release = 2;
        }
        if self.state.progress[&target.entity].release == 2 {
            if let Some(handle) = self.scene.document_rigid(index)? {
                if handle.class_id == self.scene.classes[12] {
                    self.scene.mark_carried_release(
                        self.live,
                        target.entity,
                        player
                            .managed_entity()
                            .ok_or("carry release player identity absent")?,
                        thrown,
                        self.events,
                    )?;
                }
            }
            let component = self
                .scene
                .pickables
                .get_mut(reference.handle.slot)
                .ok_or("released Pickable absent")?;
            component.interaction_flags &= !3;
            self.scene.entity_flags_d4[index] |= 4;
            self.scene.entities[index].entity_flags_d7 |= 1;
            self.state.progress.get_mut(&target.entity).unwrap().release = 3;
        }
        Ok(())
    }
    fn reference(&self, target: &Target) -> Result<(usize, CheckedComponent), String> {
        let index = *self
            .scene
            .shared_indices
            .get(&target.entity)
            .ok_or("carry entity has no live scene owner")?;
        let record = self
            .scene
            .entities
            .get(index)
            .ok_or("carry scene identity is absent")?;
        if self.live.stage(target.entity) != Some(Stage::Spawned) {
            return Err("carry managed entity generation is stale".into());
        }
        let handle = record
            .components
            .find_component([self.scene.classes[6]; 2], [0x20, 0], |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.scene.owner(handle)?.flags,
                        start_phases: self
                            .scene
                            .declarations
                            .get(&handle.packed())
                            .map(|decl| decl.scheduling.start_phases),
                    },
                )
            })?
            .ok_or("carry Pickable component is unavailable")?;
        let component = self
            .scene
            .pickables
            .get(handle.slot)
            .ok_or("carry Pickable slot is absent")?;
        if !Arc::ptr_eq(&component.definition, &target.definition) {
            return Err("carry declaration owner changed".into());
        }
        Ok((
            index,
            CheckedComponent {
                handle,
                generation: component.owner.generation,
            },
        ))
    }
    fn frame(&self, target: &Target) -> Result<ReferenceFrame, String> {
        self.scene
            .entity_frame(self.live, target.entity)?
            .ok_or("carry entity has no published frame".into())
    }
    fn center(&self, index: usize, frame: ReferenceFrame) -> Result<Vec3, String> {
        let Some(&placement) = self.scene.rigid_placements.get(&index) else {
            return Ok(Vec3::from_array(frame.position)); //878230 null render-entity branch.
        };
        let model = self
            .placements
            .0
            .get(placement)
            .ok_or("carry model placement is absent")?;
        let bounds = self
            .bounds
            .0
            .iter()
            .find(|source| source.placement == placement)
            .ok_or("carry render model has no canonical bounds")?
            .local_bounds;
        let local = (Vec3::from_array(bounds[0]) + Vec3::from_array(bounds[1])) * 0.5;
        let initial = self
            .scene
            .initial_rigid_frames
            .get(&index)
            .ok_or("carry initial rigid frame is absent")?;
        let initial_center = Vec3::from_array(model.position)
            + Mat3::from_cols_array_2d(&model.rotation) * (local * Vec3::from_array(model.scale));
        let initial_axis = Mat3::from_cols_array_2d(&initial.basis);
        let local = initial_axis.inverse() * (initial_center - Vec3::from_array(initial.position));
        Ok(Vec3::from_array(frame.position) + Mat3::from_cols_array_2d(&frame.basis) * local)
    }
    fn tag_frame(player: &Player, tag: &str) -> Result<(Vec3, Mat3), String> {
        let (joint, offset, orientation) = player.reference_tag_binding(tag)?;
        let joint = player.reference_local_joint(joint)?;
        let joint_axis = Mat3::from_cols_array_2d(&joint.basis);
        let axis = player.physics.axis();
        Ok((
            player.physics.position()
                + axis * (Vec3::from_array(joint.position) + joint_axis * Vec3::from_array(offset)),
            axis * joint_axis * Mat3::from_quat(Quat::from_array(orientation)),
        ))
    }
    fn restore_filter(&mut self, index: usize) -> Result<(), String> {
        let Some(handle) = self.scene.document_rigid(index)? else {
            return Ok(());
        };
        let filter = self.scene.rigid_component(handle)?.hold_restore_filter()?;
        let body = self
            .scene
            .rigid_component(handle)?
            .body()
            .ok_or("carry body's lifetime ended before release")?;
        let scene = &mut *self.scene;
        let mut registry = scene.registry.take().ok_or("carry registry absent")?;
        let result = (|| {
            let shared = scene.world.clone();
            let mut world = shared.write().map_err(|_| "carry world is poisoned")?;
            let world = &mut *world;
            scene_publication_events!(scene, &mut registry, publication);
            world
                .storage
                .set_body_filter(&mut world.geometry, &mut publication, body, filter)
        })();
        scene.registry = Some(registry);
        result
    }
    /// 83c860 casts the retained physical shape using the actual camera axis.
    /// Query scratch reuses capacity; geometry is borrowed from its one owner.
    fn safe_position(&mut self, target: &Target, player: &Player) -> Result<Option<Vec3>, String> {
        let (index, _) = self.reference(target)?;
        let Some(handle) = self.scene.document_rigid(index)? else {
            return Ok(None);
        };
        let body = self
            .scene
            .rigid_component(handle)?
            .body()
            .ok_or("carry cast body absent")?;
        let frame = self.frame(target)?;
        let center = self.center(index, frame)?;
        let props = target
            .definition
            .hold
            .as_ref()
            .ok_or("carry target lost hold properties")?;
        let (eye, axis) = player.published_camera_frame();
        let start = eye + axis.y_axis * props.center_offset[1];
        let direction = center - start;
        let length = direction.length();
        let unit = direction / direction.length_squared().max(f32::MIN_POSITIVE).sqrt();
        let displacement = direction + unit * f32::from_bits(0x3da3d70a);
        let shared = self.scene.world.clone();
        let queries = shared
            .read()
            .map_err(|_| "carry shape query world is poisoned")?;
        let record = queries
            .storage
            .bodies
            .get(body)
            .ok_or("carry query body is stale")?;
        let shape = queries
            .geometry
            .shapes
            .get(record.shape)
            .and_then(Option::as_ref)
            .ok_or("carry retained shape is absent")?;
        let matrix = Mat4::from_cols(
            axis.x_axis.extend(0.),
            axis.y_axis.extend(0.),
            axis.z_axis.extend(0.),
            start.extend(1.),
        )
        .to_cols_array_2d();
        let root = shape.query_shapes.get(shape.query_shapes.root())?;
        self.state.pending.clear();
        self.state.pending.push((
            shape.query_shapes.root(),
            CompoundShapeState {
                transform: ScaledShapeTransform {
                    world_from_local: matrix,
                    scale: [1.; 4],
                    offset: [0.; 4],
                },
                scaled: false,
                keep_radius: true,
                radius: root.header.radius,
            },
        ));
        let mut fraction: Option<f32> = None;
        while let Some((item, state)) = self.state.pending.pop() {
            let query_shape = shape.query_shapes.get(item)?;
            match &query_shape.geometry {
                crate::assets::physworld::query_geometry::QueryGeometry::Compound(compound) => {
                    for instance in compound
                        .instances
                        .iter()
                        .filter(|instance| instance.flags & 64 != 0)
                    {
                        let child = shape.query_shapes.get(instance.shape)?;
                        let vertices = match &child.geometry {
                            crate::assets::physworld::query_geometry::QueryGeometry::Polytope(
                                polytope,
                            ) => polytope.vertices.as_slice(),
                            _ => &[],
                        };
                        let prepared = prepare_instance(
                            instance,
                            state,
                            Mat4::IDENTITY.to_cols_array_2d(),
                            CompoundChildGeometry {
                                convex: child.header.flags & 1 != 0,
                                shape_type: child.header.shape_type,
                                vertices,
                                radius: child.header.radius,
                            },
                        )?;
                        self.state.pending.push((instance.shape, prepared.state));
                    }
                }
                crate::assets::physworld::query_geometry::QueryGeometry::Polytope(polytope) => {
                    let vertices = if state.scaled {
                        self.state.vertices.clear();
                        self.state
                            .vertices
                            .extend(polytope.vertices.iter().map(|vertex| {
                                std::array::from_fn(|lane| {
                                    if lane < 3 {
                                        vertex[lane] * state.transform.scale[lane]
                                            + state.transform.offset[lane]
                                    } else {
                                        vertex[lane]
                                    }
                                })
                            }));
                        self.state.vertices.as_slice()
                    } else {
                        polytope.vertices.as_slice()
                    };
                    let request = WorldConvexRequest {
                        cast: ConvexCast {
                            geometry: ConvexQueryGeometry {
                                shape_type: query_shape.header.shape_type,
                                vertices,
                                radius: if state.scaled {
                                    state.radius
                                } else {
                                    polytope.radius
                                },
                            },
                            transform: state.transform.world_from_local,
                            displacement: displacement.to_array(),
                            tolerance: f32::from_bits(0x3a83126f),
                            start_tolerance: 0.,
                            hit: ShapeHitInfo {
                                body_id: None,
                                material_id: 0xffff,
                                shape_key: u32::MAX,
                                filter_info: 0,
                                user_data: 0,
                            },
                            filter: Some(CollisionQueryFilter {
                                layers: 0x1ffff,
                                required_flags: 0x800,
                            }),
                        },
                        maximum_fraction: 1.,
                    };
                    queries.convex_hits(
                        &mut self.state.convex,
                        &request,
                        &mut self.state.hits,
                        &mut self.state.starts,
                    )?;
                    for hit in &self.state.hits {
                        fraction =
                            Some(fraction.map_or(hit.fraction, |value| value.min(hit.fraction)));
                    }
                }
            }
        }
        Ok(fraction.map(|fraction| {
            Vec3::from_array(frame.position)
                - unit * (length - displacement.length() * fraction + f32::from_bits(0x3da3d70a))
        }))
    }
}

impl super::super::carrying::Host for CarryHost<'_> {
    fn retired(&mut self, target: &Target) -> Result<(), String> {
        if self.live.get(target.entity).is_some() {
            return Err("carry owner stopped before its detach transaction acknowledged".into());
        }
        // Managed removal follows actual component/body retirement. A stale
        // generation must never operate on a newly reused body/component slot.
        self.state.progress.remove(&target.entity);
        Ok(())
    }
    fn live(&self, entity: EntityId) -> bool {
        self.live.stage(entity) == Some(Stage::Spawned)
            && self.scene.shared_indices.contains_key(&entity)
    }
    fn begin_pull(&mut self, target: &Target) -> Result<(), String> {
        let (_, reference) = self.reference(target)?;
        let progress = self.state.progress.entry(target.entity).or_default();
        if progress.begin >= 5 {
            return Ok(());
        }
        if let Some(sound) =
            self.scene
                .begin_document_pickup(self.live, reference, &mut progress.begin)?
        {
            self.state
                .sounds
                .push(crate::gameplay::sound::Request::Post {
                    event: sound.id,
                    emitter: crate::gameplay::sound::Emitter::PlayerBody,
                });
        }
        Ok(())
    }
    fn pull(
        &mut self,
        target: &Target,
        decl: &GrabDecl,
        hold: &HoldDecl,
        player: &Player,
    ) -> Result<bool, String> {
        let props = target
            .definition
            .hold
            .as_ref()
            .ok_or("pull target has no hold properties")?;
        let (origin, axis) = Self::tag_frame(player, &props.attach_tag)?;
        let mut offset = Vec3::from_array(props.center_offset);
        offset.x += hold.distance;
        let frame = self.frame(target)?;
        let delta = origin + axis * offset - Vec3::from_array(frame.position);
        if let Some(velocity) = decl.pull_velocity(delta, player.physics.axis().x_axis, false) {
            self.scene.carry_motion_request(
                target.entity,
                velocity.to_array(),
                [0.; 3],
                self.clock,
            )?;
            Ok(false)
        } else {
            Ok(true)
        }
    }
    fn attach(&mut self, target: &Target, hold: &HoldDecl, player: &Player) -> Result<(), String> {
        let (index, reference) = self.reference(target)?;
        let props = target
            .definition
            .hold
            .as_ref()
            .ok_or("attached target has no hold properties")?;
        let mut offset = Vec3::from_array(props.center_offset);
        offset.x += hold.distance;
        let rotation = Mat3::from_cols_array_2d(&props.angle_offset);
        //83d770 corrects the entity-origin offset by its actual rigid COM.
        if let Some(handle) = self.scene.document_rigid(index)? {
            let body = self
                .scene
                .rigid_component(handle)?
                .body()
                .ok_or("attached body absent")?;
            let shared = self.scene.world.clone();
            let queries = shared.read().map_err(|_| "carry COM world poisoned")?;
            let body = queries
                .storage
                .bodies
                .get(body)
                .ok_or("attached body stale")?;
            let motion = queries
                .storage
                .motions
                .get(body.motion_id)
                .ok_or("attached body motion absent")?;
            let frame = self.frame(target)?;
            let local = Mat3::from_cols_array_2d(&frame.basis).transpose()
                * (Vec3::from_array(frame.position)
                    - Vec3::from_array(motion.pose.center[..3].try_into().unwrap()));
            offset += rotation * local;
        }
        self.scene.bind_to_player(
            self.live,
            target.entity,
            player,
            &props.attach_tag,
            offset.to_array(),
            Quat::from_mat3(&rotation).to_array(),
            self.events,
        )?;
        self.scene
            .pickables
            .get_mut(reference.handle.slot)
            .ok_or("attached Pickable absent")?
            .interaction_flags &= !2;
        self.scene.entity_flags_d4[index] |= 4;
        self.scene.entities[index].entity_flags_d7 &= !1;
        Ok(())
    }
    fn drop(&mut self, target: &Target, _hold: &HoldDecl, player: &Player) -> Result<(), String> {
        if !self
            .state
            .progress
            .entry(target.entity)
            .or_default()
            .planned
        {
            let position = self.safe_position(target, player)?;
            let progress = self.state.progress.get_mut(&target.entity).unwrap();
            progress.position = position;
            progress.planned = true;
        }
        if !self.state.progress[&target.entity].positioned {
            if let Some(position) = self.state.progress[&target.entity].position {
                self.set_position(target, position)?;
            }
            self.state
                .progress
                .get_mut(&target.entity)
                .unwrap()
                .positioned = true;
        }
        self.release_body(target, player, false)?;
        self.state.progress.remove(&target.entity);
        Ok(())
    }
    fn throw(&mut self, target: &Target, hold: &HoldDecl, player: &Player) -> Result<(), String> {
        let (index, _) = self.reference(target)?;
        if !self
            .state
            .progress
            .entry(target.entity)
            .or_default()
            .planned
        {
            let props = target
                .definition
                .hold
                .as_ref()
                .ok_or("thrown target has no hold properties")?;
            let frame = self.frame(target)?;
            let mut origin = self.center(index, frame)?;
            let (eye, axis) = player.published_camera_frame();
            let greater = player.carrying_greater_throw();
            let settings = if greater {
                &hold.greater
            } else {
                &hold.ordinary
            };
            //83d460 ->10fc20 reads the final published camera axis in degrees,
            //rather than the player's unconstrained pitch field in radians.
            let horizontal=axis.x_axis.truncate().length();
            let pitch=if horizontal<=f32::from_bits(0x34000000){if axis.x_axis.z<0.{90.}else{-90.}}
                else{-axis.x_axis.z.atan2(horizontal)*f32::from_bits(0x42652ee0)};
            let distance = settings.target_distance(pitch);
            let destination = if let Some(hit) = player.ranged_target_hit() {
                let direction = hit - eye;
                eye + direction / direction.length_squared().max(f32::MIN_POSITIVE).sqrt()
                    * direction.length().min(distance)
            } else {
                eye + axis.x_axis * distance
            };
            let safe = self.safe_position(target, player)?;
            let position = if let Some(position) = safe {
                position
            } else {
                let shift = -axis.y_axis * props.center_offset[1];
                origin += shift;
                Vec3::from_array(frame.position) + shift
            };
            let shared = self.scene.world.clone();
            let queries = shared.read().map_err(|_| "throw world poisoned")?;
            let gravity = Vec3::from_array(queries.storage.info.gravity[..3].try_into().unwrap());
            let mut velocity = settings
                .velocity(origin, destination, gravity, player.physics.velocity)
                .ok_or("invalid authored throw speed or target")?;
            //83de90 subtracts the actual current point velocity before149.
            if let Some(handle) = self.scene.document_rigid(index)? {
                let body = self
                    .scene
                    .rigid_component(handle)?
                    .body()
                    .ok_or("thrown body absent")?;
                let body = queries
                    .storage
                    .bodies
                    .get(body)
                    .ok_or("thrown body stale")?;
                let motion = queries
                    .storage
                    .motions
                    .get(body.motion_id)
                    .ok_or("thrown motion absent")?;
                velocity -= Vec3::from_array(motion.linear_velocity[..3].try_into().unwrap());
            }
            drop(queries);
            if safe.is_some() {
                velocity = velocity / velocity.length_squared().max(f32::MIN_POSITIVE).sqrt() * 0.1;
            }
            velocity *= props.throw_factor;
            let progress = self.state.progress.get_mut(&target.entity).unwrap();
            progress.position = Some(position);
            progress.velocity = Some(velocity);
            progress.planned = true;
        }
        if !self.state.progress[&target.entity].positioned {
            self.set_position(
                target,
                self.state.progress[&target.entity]
                    .position
                    .ok_or("throw placement was not planned")?,
            )?;
            self.state
                .progress
                .get_mut(&target.entity)
                .unwrap()
                .positioned = true;
        }
        if !self.state.progress[&target.entity].impulse_posted {
            let velocity = self.state.progress[&target.entity]
                .velocity
                .ok_or("throw velocity was not planned")?;
            // Native149 has six floats, flagbyte then source-defined padding.
            let mut payload = Vec::with_capacity(28);
            for value in [velocity.x, velocity.y, velocity.z, 0., 0., 0.] {
                payload.extend_from_slice(&value.to_le_bytes());
            }
            payload.extend_from_slice(&[0; 4]);
            self.events.post(Event {
                id: 0x149,
                target: Some(target.entity),
                payload: Payload::Bytes(payload),
            });
            self.state
                .progress
                .get_mut(&target.entity)
                .unwrap()
                .impulse_posted = true;
        }
        self.release_body(target, player, true)?;
        if let Some(path) = hold.throw_fx.as_deref() {
            let name = self
                .live
                .get(target.entity)
                .ok_or("thrown FX entity generation retired")?
                .0
                .to_owned();
            self.state.next_effect = self.state.next_effect.wrapping_add(1);
            let key = Arc::from(format!("carry_throw:{}", self.state.next_effect));
            self.effects.start_entity_once(
                key,
                path,
                super::super::usable_effects::EntityPlacement {
                    target: crate::gameplay::event_graph::EntityReference::Named(name),
                    tag: String::new(),
                    local: Default::default(),
                    attached: true,
                },
            );
        }
        self.state.progress.remove(&target.entity);
        Ok(())
    }
    fn consumed(&mut self, target: &Target) -> Result<(), String> {
        self.scene
            .unbind_entity(self.live, target.entity, self.events)?;
        self.state.progress.remove(&target.entity);
        Ok(())
    }
}

fn run(world: &mut bevy::prelude::World, retiring: bool) -> Result<(), String> {
    use super::super::{carrying::Carrying, player::PlayerState, usable_effects::EffectRequests};
    if !world.contains_resource::<Carrying>() {
        return Ok(());
    }
    if let Some(player) = world
        .get_resource::<PlayerState>()
        .and_then(|state| state.player.as_ref())
    {
        if !world.resource::<Carrying>().has_work(player) {
            return Ok(());
        }
        if !retiring&&!world.resource::<Carrying>().has_transactions(player){
            if player.carrying.held.as_ref().is_some_and(|target|world.get_resource::<LiveEntities>()
                .is_some_and(|live|live.stage(target.entity)==Some(Stage::Spawned))){return Ok(());}
        }
    }
    if !world.contains_resource::<CarryPhysics>()
        || !world.contains_resource::<PhysicsScene>()
        || !world.contains_resource::<LiveEntities>()
        || !world.contains_resource::<EntityEvents>()
        || !world.contains_resource::<PlayerState>()
        || !world.contains_resource::<EffectRequests>()
    {
        return Err("carry transaction is missing a canonical level owner".into());
    }
    let mut carrying = world.remove_resource::<Carrying>().unwrap();
    let mut state = world.remove_resource::<CarryPhysics>().unwrap();
    let mut scene = world.remove_resource::<PhysicsScene>().unwrap();
    let mut live = world.remove_resource::<LiveEntities>().unwrap();
    let mut events = world.remove_resource::<EntityEvents>().unwrap();
    let mut player = world.remove_resource::<PlayerState>().unwrap();
    let mut effects = world.remove_resource::<EffectRequests>().unwrap();
    let result = (|| {
        let Some(player) = player.player.as_mut() else {
            return Ok(());
        };
        let clock = world
            .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("carry game clock owner absent")?;
        if !retiring && !clock.advanced() {
            return Ok(());
        }
        let clock = clock.clock().map_err(str::to_owned)?;
        let bounds = world
            .get_resource::<super::super::shadow_updates::ModelShadowSources>()
            .ok_or("carry model bounds owner absent")?;
        let placements = world
            .get_resource::<super::super::RenderSourcePlacements>()
            .ok_or("carry model placement owner absent")?;
        let mut host = CarryHost {
            scene: &mut scene,
            live: &mut live,
            events: &mut events,
            state: &mut state,
            clock,
            bounds,
            placements,
            effects: &mut effects,
        };
        if retiring {
            carrying.retire(player, &mut host)
        } else {
            carrying.update(player, &mut host)
        }
    })();
    for request in state.sounds.drain(..) {
        world.write_message(super::super::audio::SoundEvent(request));
    }
    world.insert_resource(carrying);
    world.insert_resource(state);
    world.insert_resource(scene);
    world.insert_resource(live);
    world.insert_resource(events);
    world.insert_resource(player);
    world.insert_resource(effects);
    result
}
pub(in crate::app) fn update(world: &mut bevy::prelude::World) {
    if let Err(error) = run(world, false) {
        let repeat = world
            .get_resource::<CarryPhysics>()
            .is_some_and(|state| state.error.as_ref() == Some(&error));
        if !repeat {
            bevy::log::error!("Carry transaction failed: {error}");
        }
        if let Some(mut state) = world.get_resource_mut::<CarryPhysics>() {
            state.error = Some(error);
        }
    }
}
/// Root LevelLifetime calls this before the player reference/rigid/FX owners.
/// Any failure retains the level resources and pending journal for retry.
pub(in crate::app) fn retire(world: &mut bevy::prelude::World) -> Result<(), String> {
    run(world, true)
}
