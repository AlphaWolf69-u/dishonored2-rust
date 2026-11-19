//! HoldObject transactions over the actual Pickable, rigid and reference actor.
use super::*;
use crate::gameplay::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    compound_query::instance::{CompoundChildGeometry, CompoundShapeState, prepare_instance},
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities, Stage},
    pickable_component::CheckedComponent,
    player::{
        Player,
        carrying::{GrabDecl, HoldDecl, Target},
    },
    reference_frame::ReferenceFrame,
    shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
    shape_transform::{ScaledShapeTransform, ShapeMatrix},
    world_convex_query::{WorldConvexQuery, WorldConvexRequest},
};
use bevy::prelude::{Mat3, Mat4, Quat, Vec3};

#[derive(Debug, Default)]
struct Progress {
    loot_eligible: bool,
    begin: u8,
    release: u8,
    planned: bool,
    position: Option<Vec3>,
    velocity: Option<Vec3>,
    angular: Option<Vec3>,
    positioned: bool,
    impulse_posted: bool,
}
#[derive(Resource, Default)]
pub(in crate::app) struct CarryPhysics {
    pub(super) resources: super::carrying_resources::Resources,
    progress: HashMap<EntityId, Progress>,
    convex: WorldConvexQuery,
    pub(super) catch_ray:crate::gameplay::world_ray_query::WorldRayQuery,
    pub(super) catch_notices:[super::super::entity_graphs::GrappledReceipt;2],
    mesh: crate::gameplay::compressed_mesh_query::CompressedMeshQuery,
    support: crate::gameplay::convex_support::ConvexSupportWorkspace,
    hits: Vec<ConvexContact>,
    starts: Vec<ConvexContact>,
    pending: Vec<(usize, CompoundShapeState)>,
    vertices: Vec<[f32; 4]>,
    sounds: Vec<crate::gameplay::sound::Request>,
    next_effect: u64,
    error: Option<String>,
}
pub(in crate::app) fn describe_fixture_progress(world:&bevy::prelude::World,id:EntityId)->String {
    world.get_resource::<CarryPhysics>().map_or_else(||"carry owner absent".into(),|owner|
        format!("progress={:?} last_error={:?}",owner.progress.get(&id),owner.error))
}
///Diagnostic only: apply the real body delta to the render owner's own
///baseline, preserving authored model/body offsets and comparing game space.
fn placement_alignment(scene:&PhysicsScene,index:usize,body:ReferenceFrame,
    placements:&super::super::RenderSourcePlacements,
    bindings:Option<&super::super::model_motion::ModelMotionBindings>)
    ->Result<Option<([f32;3],[f32;3],[f32;3],f32)>,String>{
    let Some(&placement)=scene.rigid_placements.get(&index)else{return Ok(None)};
    let Some(initial_render)=bindings.and_then(|bindings|bindings.initial_pose(placement))else{return Ok(None)};
    let initial_body=*scene.initial_rigid_frames.get(&index).ok_or("carry alignment original body absent")?;
    let actual=placements.0.get(placement).ok_or("carry alignment current render placement absent")?;
    let delta=super::rigid_actor::render_delta(initial_body,body)?;
    let axis=Mat3::from_cols(Vec3::new(delta[0][0],delta[1][0],delta[2][0]),
        Vec3::new(delta[0][1],delta[1][1],delta[2][1]),Vec3::new(delta[0][2],delta[1][2],delta[2][2]));
    let expected=axis*Vec3::from_array(initial_render.position)+Vec3::new(delta[0][3],delta[1][3],delta[2][3]);
    let expected_basis=(axis*Mat3::from_cols_array_2d(&initial_render.basis)).to_cols_array_2d();
    let basis_error=expected_basis.iter().flatten().zip(actual.rotation.iter().flatten()).map(|(a,b)|(a-b).abs()).fold(0.,f32::max);
    Ok(Some((expected.to_array(),actual.position,(Vec3::from_array(actual.position)-expected).to_array(),basis_error)))
}
/// On-demand carry observation; reads the same physical/render/tag owners.
/// It retains no model, material, entity lease or normal-frame history.
pub(in crate::app) fn describe_pose(world:&bevy::prelude::World,id:EntityId)->Result<String,String>{
    use std::fmt::Write;
    let scene=world.get_resource::<PhysicsScene>().ok_or("carry pose Scene absent")?;
    let live=world.get_resource::<LiveEntities>().ok_or("carry pose LiveEntities absent")?;
    let mut out=String::new();
    let Some(&index)=scene.shared_indices.get(&id) else{return Ok(format!("carry target={id:?} actual_scene_absent=true"))};
    let entity=&scene.entities[index];
    let _=writeln!(out,"carry_pose target={id:?} name={} live={:?} parent={:?} entity_frame={:?} initial_rigid_frame={:?}",entity.name,live.stage(id),live.parent(id),entity.frame,scene.initial_rigid_frames.get(&index));
    if let Some(actor)=entity.components.reference_actor.and_then(|handle|scene.actors.actors.get(handle.slot)){
        let _=writeln!(out,"  reference_actor frame={:?} binding={:?} current={} default={}",actor.frame.published(),(actor.binding.entity,actor.binding.joint,actor.binding.offset,actor.binding.orientation,actor.binding.phase),actor.current_body,actor.default_body);
    }
    if let Some(handle)=scene.document_rigid(index)?{
        let component=scene.rigid_component(handle)?;
        let shared=scene.point_queries()?;let queries=shared.read().map_err(|_|"carry pose physics world poisoned")?;
        if let Some(body)=component.body().and_then(|body|queries.storage.bodies.get(body)){
            let _=writeln!(out,"  actual_body id={} flags={:#x} transform={:?} motion={:?} requested_velocities={:?}",body.id,body.flags,body.transform,queries.storage.motions.get(body.motion_id),component.velocities);
            if let Some(placements)=world.get_resource::<super::super::RenderSourcePlacements>(){
                let pose=ReferenceFrame{position:body.transform[3][..3].try_into().unwrap(),basis:std::array::from_fn(|i|body.transform[i][..3].try_into().unwrap())};
                let _=writeln!(out,"  registered_body_vs_render_game(expected_origin,actual_origin,residual_m,max_basis_error)={:?}",placement_alignment(scene,index,pose,placements,world.get_resource::<super::super::model_motion::ModelMotionBindings>())?);
            }
        }
    }
    if let Some(&placement)=scene.rigid_placements.get(&index){
        if let Some(model)=world.get_resource::<super::super::RenderSourcePlacements>().and_then(|owner|owner.0.get(placement)){
            let _=writeln!(out,"  placed index={placement} resource={} position={:?} basis={:?} scale={:?}",model.resource_path,model.position,model.rotation,model.scale);
        }
        if let Some(queue)=world.get_resource::<super::super::SceneRenderQueue>(){
            for instance in queue.instances.iter().filter(|instance|instance.source.is_some_and(|source|match source.surface{
                super::super::RenderModelSurface::Bmodel{placement:p,..}|super::super::RenderModelSurface::Bmd6{placement:p,..}=>p==placement,_=>false,
            })).take(4){let _=writeln!(out,"  actual_draw transform={:?} bounds={:?} visible={}",instance.transform,instance.bounds,instance.visible);}
        }
    }
    if let Some(player)=world.get_resource::<super::super::player::PlayerState>().and_then(|state|state.player.as_ref()){
        let _=writeln!(out,"  player feet={:?} axis={:?} camera={:?}",player.feet(),player.physics.axis(),player.published_camera_frame());
        if let Some((_,pickable))=scene.grapple_pickable(live,id)?{
            if let Some(props)=pickable.definition.hold.as_ref(){
                let _=writeln!(out,"  actual_hand_tag name={} world={:?} authored_offset={:?} authored_angles={:?}",props.attach_tag,CarryHost::tag_frame(player,&props.attach_tag),props.center_offset,props.angle_offset);
            }
        }
    }
    let _=writeln!(out,"  {}",describe_fixture_progress(world,id));
    Ok(out)
}
pub(in crate::app) struct CarryHost<'a> {
    pub scene: &'a mut PhysicsScene,
    pub live: &'a mut LiveEntities,
    pub events: &'a mut EntityEvents,
    pub state: &'a mut CarryPhysics,
    pub clock: &'a crate::gameplay::game_clock::GameClock,
    pub bounds: &'a super::super::shadow_updates::ModelShadowSources,
    pub placements: &'a super::super::RenderSourcePlacements,
    pub render_bindings: Option<&'a super::super::model_motion::ModelMotionBindings>,
    pub effects: &'a mut super::super::usable_effects::EffectRequests,
}
///Borrowed real47 transaction metadata. Neither inventory item nor a made-up
///Hold declaration is created to reuse the shared native pull algorithm.
pub(in crate::app) struct LootTarget<'a> {
    pub entity: EntityId,
    pub reference: CheckedComponent,
    pub definition: &'a crate::gameplay::pickable_component::PickableDefinition,
    pub from_grapple: bool,
}
impl CarryHost<'_> {
    fn begin_pull_for(
        &mut self,
        entity: EntityId,
        reference: CheckedComponent,
    ) -> Result<(), String> {
        let progress = self.state.progress.entry(entity).or_default();
        if progress.begin >= 5 {
            return Ok(());
        }
        if let Some(sound) = self.scene.begin_document_pickup(
            self.live,
            self.events,
            reference,
            &mut progress.begin,
        )? {
            self.state
                .sounds
                .push(crate::gameplay::sound::Request::Post {
                    event: sound.id,
                    emitter: crate::gameplay::sound::Emitter::PlayerBody,
                });
        }
        Ok(())
    }
    pub(in crate::app) fn begin_loot_pull(
        &mut self,
        target: &LootTarget<'_>,
    ) -> Result<(), String> {
        let (_, reference) =
            self.pickable_reference(target.entity, target.definition, Some(target.reference))?;
        self.begin_pull_for(target.entity, reference)
    }
    fn pull_delta(
        &mut self,
        entity: EntityId,
        delta: Vec3,
        decl: &GrabDecl,
        player: &Player,
        max_grab: bool,
    ) -> Result<bool, String> {
        if let Some(velocity) = decl.pull_velocity(delta, player.physics.axis().x_axis, max_grab) {
            //877530: bound children do not receive pull motion requests.
            if self.live.parent(entity)?.is_none() {
                let angular = if max_grab {
                    player.physics.axis().y_axis
                        * decl.grappled_angular_velocity
                        * f32::from_bits(0xbc8efa35)
                } else {
                    Vec3::ZERO
                };
                self.scene.request_actor_velocity(
                    entity,
                    velocity.to_array(),
                    angular.to_array(),
                )?;
            }
            Ok(false)
        } else {
            Ok(true)
        }
    }
    pub(in crate::app) fn pull_loot(
        &mut self,
        target: &LootTarget<'_>,
        decl: &GrabDecl,
        player: &Player,
        player_center: Vec3,
        target_center: Vec3,
    ) -> Result<bool, String> {
        let (_, reference) =
            self.pickable_reference(target.entity, target.definition, Some(target.reference))?;
        let pickable = self
            .scene
            .pickables
            .get(reference.handle.slot)
            .ok_or("Loot pull Pickable absent")?;
        //841500 tests raw+28 against all-ones, not the PhysicsBase pointer.
        //Our admitted constructors use native managed-null0x3fff0010 or a
        //real saved binding parent; neither is -1. No saved-state path
        //currently admits the separate all-ones representation.
        if pickable.interaction_flags & 4 != 0
        {
            return Ok(true);
        }
        let delta = if let Some(props) = target.definition.hold.as_ref() {
            let hold = player
                .carrying_hold_decl_for_hands(props.both_hands)
                .ok_or("Loot hold target has no actual mode Hold declaration")?;
            self.hold_pull_delta(target.entity, props, hold, player)?
        } else {
            player_center + Vec3::Z * decl.object_pickup_z_offset - target_center
        };
        self.pull_delta(target.entity, delta, decl, player, target.from_grapple)
    }
    fn hold_pull_delta(
        &self,
        entity: EntityId,
        props: &crate::gameplay::pickable_component::HoldProperties,
        hold: &HoldDecl,
        player: &Player,
    ) -> Result<Vec3, String> {
        let (origin, axis) = Self::tag_frame(player, &props.attach_tag)?;
        let mut offset = Vec3::from_array(props.center_offset);
        offset.x += hold.distance;
        let frame = self
            .scene
            .entity_frame(self.live, entity)?
            .ok_or("pull entity frame absent")?;
        Ok(origin + axis * offset - Vec3::from_array(frame.position))
    }
    fn set_position(&mut self, target: &Target, position: Vec3) -> Result<(), String> {
        let (index, _) = self.reference(target)?;
        let mut pose = self.frame(target)?;
        pose.position = position.to_array();
        let scene=&mut *self.scene;
        let mut registry=scene.registry.take().ok_or("carry move registry absent")?;
        let result=scene.publish_entity_pose_in_registry(index,pose,&mut registry);
        scene.registry=Some(registry);
        result
    }
    fn release_body(
        &mut self,
        target: &Target,
        player: &Player,
        thrown: bool,
    ) -> Result<(), String> {
        let (index, reference) = self.reference(target)?;
        if player.carrying.tracing_enabled(){
            let body=self.scene.document_rigid(index)?.and_then(|handle|self.scene.rigid_component(handle).ok()).and_then(|component|component.body());
            let shared=self.scene.point_queries()?;
            let world=shared.read().map_err(|_|"carry detach observation world poisoned")?;
            if let Some(body)=body.and_then(|id|world.storage.bodies.get(id)){
                let pose=ReferenceFrame{position:body.transform[3][..3].try_into().unwrap(),basis:std::array::from_fn(|i|body.transform[i][..3].try_into().unwrap())};
                eprintln!("ACTION_CARRY_DETACH entity={:?} thrown={thrown} body={} registered_pose={pose:?} actual_actor_pose={:?} planned_position={:?} registered_motion(linear,world_angular)={:?} game_alignment(expected_render_origin,actual_render_origin,residual_m,max_basis_error)={:?}",target.entity,body.id,self.scene.entities[index].frame.pose,self.state.progress.get(&target.entity).and_then(|progress|progress.position),world.storage.motions.get(body.motion_id).map(|motion|(motion.linear_velocity,motion.world_angular_velocity())),placement_alignment(self.scene,index,pose,self.placements,self.render_bindings)?);
            }
        }
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
                        player,
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
        self.pickable_reference(target.entity, &target.definition, None)
    }
    fn pickable_reference(
        &self,
        entity: EntityId,
        definition: &crate::gameplay::pickable_component::PickableDefinition,
        expected: Option<CheckedComponent>,
    ) -> Result<(usize, CheckedComponent), String> {
        let index = *self
            .scene
            .shared_indices
            .get(&entity)
            .ok_or("carry entity has no live scene owner")?;
        let record = self
            .scene
            .entities
            .get(index)
            .ok_or("carry scene identity is absent")?;
        if self.live.stage(entity) != Some(Stage::Spawned) {
            return Err("carry managed entity generation is stale".into());
        }
        let handle = record
            .components
            .find_component([self.scene.classes[6]; 2], [0x28, 0], |handle| {
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
        if !std::ptr::eq(&*component.definition, definition) {
            return Err("carry declaration owner changed".into());
        }
        let reference = CheckedComponent {
            handle,
            generation: component.owner.generation,
        };
        if expected.is_some_and(|expected| expected != reference) {
            return Err("pull Pickable generation differs from accepted transaction".into());
        }
        Ok((index, reference))
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
        let bounds = self
            .bounds
            .0
            .iter()
            .find(|source| source.placement == placement)
            .ok_or("carry render model has no canonical bounds")?
            .bounds;
        //878230 reads the render owner's current world bounds. ModelPose has
        //already applied the body's world delta to these bounds; rebasing their
        //center through the spawn frame would apply the pickup movement twice.
        Ok((Vec3::from_array(bounds[0]) + Vec3::from_array(bounds[1])) * 0.5)
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
                            crate::assets::physworld::query_geometry::QueryGeometry::Convex(
                                convex,
                            ) => convex.vertices.as_slice(),
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
                crate::assets::physworld::query_geometry::QueryGeometry::CompressedMesh(mesh) => {
                    use crate::gameplay::{
                        compressed_mesh_query::{MeshSweep, support_input, support_leaves},
                        ray_query::RayQueryControl,
                    };
                    // A moving composite casts each real leaf, just as its compound
                    // siblings do. The source mesh visitor owns decoding, quad policy
                    // and radii; its bounds only select leaves, never collision hits.
                    let bounds = mesh.bounds();
                    let sweep = MeshSweep {
                        center: std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5),
                        half_extent: std::array::from_fn(|i| (bounds[1][i] - bounds[0][i]) * 0.5),
                        direction: [0.; 3],
                    };
                    let mut control = RayQueryControl {
                        fraction: 1.,
                        early_out: false,
                    };
                    let CarryPhysics {
                        mesh: visitor,
                        support,
                        convex,
                        hits,
                        starts,
                        ..
                    } = self.state;
                    visitor.visit_swept(
                        mesh,
                        sweep,
                        query_shape.header.radius,
                        &mut control,
                        &mut |leaf, _| {
                            support_leaves(leaf, &mut |leaf| {
                                let input = support_input(&leaf, state)?;
                                let shape_type = input.shape_type;
                                let prepared = support.prepare(input)?;
                                let request = WorldConvexRequest {
                                    cast: ConvexCast {
                                        geometry: ConvexQueryGeometry {
                                            shape_type,
                                            vertices: prepared.vertices,
                                            radius: prepared.radius,
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
                                queries.convex_hits(convex, &request, hits, starts)?;
                                for hit in hits.iter() {
                                    fraction = Some(
                                        fraction
                                            .map_or(hit.fraction, |value| value.min(hit.fraction)),
                                    );
                                }
                                Ok(())
                            })
                        },
                    )?;
                }
                crate::assets::physworld::query_geometry::QueryGeometry::Polytope(_)
                | crate::assets::physworld::query_geometry::QueryGeometry::Convex(_) => {
                    let (source_vertices, source_radius) = match &query_shape.geometry {
                        crate::assets::physworld::query_geometry::QueryGeometry::Polytope(
                            polytope,
                        ) => (polytope.vertices.as_slice(), polytope.radius),
                        crate::assets::physworld::query_geometry::QueryGeometry::Convex(convex) => {
                            (convex.vertices.as_slice(), convex.radius)
                        }
                        _ => unreachable!(),
                    };
                    let vertices = if state.scaled {
                        self.state.vertices.clear();
                        self.state
                            .vertices
                            .extend(source_vertices.iter().map(|vertex| {
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
                        source_vertices
                    };
                    let request = WorldConvexRequest {
                        cast: ConvexCast {
                            geometry: ConvexQueryGeometry {
                                shape_type: query_shape.header.shape_type,
                                vertices,
                                radius: if state.scaled {
                                    state.radius
                                } else {
                                    source_radius
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
        let position=fraction.map(|fraction| {
            Vec3::from_array(frame.position)
                - unit * (length - displacement.length() * fraction + f32::from_bits(0x3da3d70a))
        });
        if player.carrying.tracing_enabled(){
            eprintln!("ACTION_CARRY_RELEASE_CAST entity={:?} frame={} actor_origin={:?} render_center={center:?} camera_start={start:?} displacement={displacement:?} fraction={fraction:?} release_position={position:?}",target.entity,self.clock.frame_marker(),frame.position);
        }
        Ok(position)
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
        self.begin_pull_for(target.entity, reference)
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
        let delta = self.hold_pull_delta(target.entity, props, hold, player)?;
        self.pull_delta(target.entity, delta, decl, player, false)
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
        //83d770 ->731470 corrects only an actual PhysicsSystemEvolved
        //center. An ordinary RigidBody COM is a different source owner.
        if self.scene.grapple_dependency(index,4,[0x28,0])?.is_some() {
            return Err("held PhysicsSystemEvolved requires its actual731470 center provider".into());
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
            //83de90/8c3cb0 uses731470 only for a real PhysicsSystem subtype.
            //Ordinary rigid props launch from the entity origin, not render
            //bounds or the Havok motion center.
            if self.scene.grapple_dependency(index,4,[0x28,0])?.is_some() {
                return Err("throw PhysicsSystem requires its actual731470 origin provider".into());
            }
            let mut origin = Vec3::from_array(frame.position);
            let (eye, axis) = player.published_camera_frame();
            let greater = player.carrying_greater_throw();
            let settings = if greater {
                &hold.greater
            } else {
                &hold.ordinary
            };
            //83d460 ->10fc20 reads the final published camera axis in degrees,
            //rather than the player's unconstrained pitch field in radians.
            let horizontal = axis.x_axis.truncate().length();
            let pitch = if horizontal <= f32::from_bits(0x34000000) {
                if axis.x_axis.z < 0. { 90. } else { -90. }
            } else {
                -axis.x_axis.z.atan2(horizontal) * f32::from_bits(0x42652ee0)
            };
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
            //83de90's spin axis is cross(normalized launch,-camera up).
            let direction=velocity/velocity.length_squared().max(f32::MIN_POSITIVE).sqrt();
            let spin_axis=direction.cross(-axis.z_axis);
            let remove_spin=crate::platform::console::shared().read().cvars
                .int("ark_throw_shifting_removeAngularVelocity").unwrap_or(0)!=0;
            let angular=if safe.is_none()&&remove_spin {Vec3::ZERO} else {
                spin_axis/spin_axis.length_squared().max(f32::MIN_POSITIVE).sqrt()*10.
            };
            //8086a0 returns the COM-offset displacement caused by this angular
            //step divided by the actual entity delta. It does not read the
            //body's current linear velocity.
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
                let delta=self.clock.entity_delta(self.scene.entities[index].timespace)
                    .ok_or("throw entity timespace absent")?;
                velocity-=Vec3::from_array(crate::gameplay::physics_actor::body_frame::center_rotation_velocity(
                    body.transform[3][..3].try_into().unwrap(),
                    motion.pose.center[..3].try_into().unwrap(),angular.to_array(),delta));
            }
            drop(queries);
            if safe.is_some() {
                velocity = velocity / velocity.length_squared().max(f32::MIN_POSITIVE).sqrt() * 0.1;
            }
            velocity *= props.throw_factor;
            let progress = self.state.progress.get_mut(&target.entity).unwrap();
            progress.position = Some(position);
            progress.velocity = Some(velocity);
            progress.angular = Some(angular);
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
            let angular=self.state.progress[&target.entity].angular.ok_or("throw angular velocity was not planned")?;
            for value in [velocity.x, velocity.y, velocity.z, angular.x, angular.y, angular.z] {
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
        //83de90 finishes a throw by arming the released actor's Breakable
        //through 7edfa0. A normal drop never enters this branch.
        self.scene.set_breakable_thrown(self.live, target.entity, true)?;
        if player.carrying.tracing_enabled() {
            let progress=&self.state.progress[&target.entity];
            eprintln!("ACTION_SMOKE_THROW_NATIVE entity={:?} frame={} release_position={:?} requested_linear={:?} requested_angular={:?} message149_posted={} release_complete={} source=83de90 actual_solver_consumption_required=true",
                target.entity,self.clock.frame_marker(),progress.position,progress.velocity,
                progress.angular,progress.impulse_posted,progress.release==3);
        }
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
        if !world.resource::<Carrying>().has_work(player) && !player.grapple_catch_has_work() {
            return Ok(());
        }
        if !retiring && !world.resource::<Carrying>().has_transactions(player) && !player.grapple_catch_has_work() {
            if player.carrying.held.as_ref().is_some_and(|target| {
                world
                    .get_resource::<LiveEntities>()
                    .is_some_and(|live| live.stage(target.entity) == Some(Stage::Spawned))
            }) {
                return Ok(());
            }
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
        if !retiring && !world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("carry game clock owner absent")?.advanced() {return Ok(());}
        super::grapple_catch::advance_borrowed(world,&mut scene,&live,player,&mut state,retiring)?;
        let clock=world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("carry game clock owner absent")?.clock().map_err(str::to_owned)?;
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
            render_bindings:world.get_resource::<super::super::model_motion::ModelMotionBindings>(),
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
    run(world, true)?;
    super::carrying_resources::retire(world)
}

///Actual47 producer only. Normal48/49 ownership continues through Carrying;
///these functions neither request new input nor create a second target runtime.
pub(in crate::app) fn update_loot_pull(world: &mut bevy::prelude::World) -> Result<(), String> {
    run_loot_pull(world, false)
}
pub(in crate::app) fn finish_accepted_loot_pull(
    world: &mut bevy::prelude::World,
) -> Result<(), String> {
    run_loot_pull(world, true)
}
pub(in crate::app) fn acknowledge_loot_pull(world: &mut bevy::prelude::World, id: EntityId) {
    if let Some(mut carry) = world.get_resource_mut::<CarryPhysics>() {
        carry.progress.remove(&id);
    }
}
fn run_loot_pull(world: &mut bevy::prelude::World, closing: bool) -> Result<(), String> {
    use super::super::{
        interactions::Interactions, player::PlayerState, usable_effects::EffectRequests,
    };
    let Some(player) = world
        .get_resource::<PlayerState>()
        .and_then(|state| state.player.as_ref())
    else {
        return Ok(());
    };
    let Some(id) = player.world_pickup_target() else {
        return Ok(());
    };
    let offered = player.pending_loot_object() == Some(id);
    let mode = player
        .world_pickup_mode()
        .ok_or("accepted Loot47 mode absent")?;
    let clock = world
        .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .ok_or("Loot47 clock owner absent")?;
    if !closing && !clock.advanced() {
        return Ok(());
    }
    let pending = world
        .get_resource::<Interactions>()
        .and_then(|state| state.pending_pickup.as_ref())
        .ok_or("accepted Loot47 has no actual Pickable resource lease")?;
    if pending.entity != id {
        return Err("Loot47 target differs from retained Pickable lease".into());
    }
    let eligibility_pending = world
        .get_resource::<CarryPhysics>()
        .and_then(|carry| carry.progress.get(&id))
        .is_none_or(|progress| !progress.loot_eligible);
    let full = eligibility_pending
        && pending.inventory_full(
            world
                .get_resource::<super::super::interaction_focus::DiscoveredDocuments>()
                .ok_or("Loot47 inventory owner absent")?,
        );
    let full_sound = if full {
        world
            .get_resource::<super::super::player_actions::PlayerActions>()
            .ok_or("Loot47 sound declaration owner absent")?
            .loot_full_sounds
            .get(mode)
            .ok_or("Loot47 sound mode absent")?
            .as_ref()
            .map(|event| event.id)
    } else {
        None
    };
    let player_center = if !closing && !offered && !full {
        let bounds = world
            .get_resource::<super::super::player_arms::PlayerArms>()
            .ok_or("Loot47 actual player render owner absent")?
            .body_model_bounds(player)?;
        Some(Vec3::from_array(std::array::from_fn(|i| {
            (bounds[0][i] + bounds[1][i]) * 0.5
        })))
    } else {
        None
    };
    let runtime_center = if player_center.is_some() {
        super::super::world_models::bounds(world, id).map(|bounds| {
            Vec3::from_array(std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5))
        })
    } else {
        None
    };
    for present in [
        world.contains_resource::<CarryPhysics>(),
        world.contains_resource::<PhysicsScene>(),
        world.contains_resource::<LiveEntities>(),
        world.contains_resource::<EntityEvents>(),
        world.contains_resource::<EffectRequests>(),
    ] {
        if !present {
            return Err("Loot47 pull is missing a canonical level owner".into());
        }
    }
    let mut interactions = world.remove_resource::<Interactions>().unwrap();
    let mut state = world.remove_resource::<CarryPhysics>().unwrap();
    let mut scene = world.remove_resource::<PhysicsScene>().unwrap();
    let mut live = world.remove_resource::<LiveEntities>().unwrap();
    let mut events = world.remove_resource::<EntityEvents>().unwrap();
    let mut player_state = world.remove_resource::<PlayerState>().unwrap();
    let mut effects = world.remove_resource::<EffectRequests>().unwrap();
    let result = (|| {
        let player = player_state
            .player
            .as_mut()
            .ok_or("accepted Loot47 player absent")?;
        let pending = interactions
            .pending_pickup
            .as_ref()
            .ok_or("accepted Loot47 lease absent")?;
        let valid = live.get(id).is_some()
            && scene.shared_indices.contains_key(&id)
            && scene
                .pickables
                .get(pending.reference.handle.slot)
                .is_some_and(|component| {
                    component.owner.generation == pending.reference.generation
                        && crate::gameplay::component_state::matches_reference_state(
                            component.owner.flags,
                            [0x28, 0],
                        )
                });
        if !valid {
            player.lose_loot_object(id)?;
            super::super::time_sync::detach_entity_time_with_owners(world,&scene,&live,id)?;
            state.progress.remove(&id);
            return Ok(());
        }
        if !state.progress.entry(id).or_default().loot_eligible {
            if full {
                player.reject_loot_object(id)?;
                let pending = interactions
                    .pending_pickup
                    .as_mut()
                    .ok_or("rejected Loot47 lease absent")?;
                pending.rejected_notice = true;
                pending.rejected_sound = full_sound;
                state.progress.remove(&id);
                return Ok(());
            }
            scene.entities[*scene.shared_indices.get(&id).ok_or("Loot47 source entity absent")?].entity_flags &= !(1<<15);
            let player_entity=super::super::player_lifetime::id_with_live(world,&live)
                .ok_or("Loot47 time attachment Player generation absent")?;
            super::super::time_sync::attach_entity_time(world,&scene,&live,player_entity,id)?;
            state.progress.get_mut(&id).unwrap().loot_eligible = true;
        }
        let clock = world
            .resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .clock()
            .map_err(str::to_owned)?;
        let mut host = CarryHost {
            scene: &mut scene,
            live: &mut live,
            events: &mut events,
            state: &mut state,
            clock,
            bounds: world
                .get_resource::<super::super::shadow_updates::ModelShadowSources>()
                .ok_or("Loot47 canonical model bounds absent")?,
            placements: world
                .get_resource::<super::super::RenderSourcePlacements>()
                .ok_or("Loot47 placed render owner absent")?,
            render_bindings:world.get_resource::<super::super::model_motion::ModelMotionBindings>(),
            effects: &mut effects,
        };
        let target = LootTarget {
            entity: id,
            reference: pending.reference,
            definition: &pending.definition,
            from_grapple: pending.from_grapple,
        };
        //Source24 map uses its own immediate callback and skips76e2b0.
        if pending.item.as_ref().is_none_or(|item|item.type_number!=24) {
            host.begin_loot_pull(&target)?;
        }
        if closing || offered {
            return Ok(());
        }
        let target_center = if let Some(center) = runtime_center {
            center
        } else {
            let (index, _) =
                host.pickable_reference(id, target.definition, Some(target.reference))?;
            let frame = host
                .scene
                .entity_frame(host.live, id)?
                .ok_or("Loot47 target frame absent")?;
            host.center(index, frame)?
        };
        let reached = {
            let decl = player
                .world_pickup_decl()
                .ok_or("Loot47 actual declaration absent")?;
            host.pull_loot(
                &target,
                decl,
                player,
                player_center.ok_or("Loot47 physical player center absent")?,
                target_center,
            )?
        };
        if reached {
            player.world_pickup_reached(id)?;
        }
        Ok(())
    })();
    for request in state.sounds.drain(..) {
        world.write_message(super::super::audio::SoundEvent(request));
    }
    world.insert_resource(interactions);
    world.insert_resource(state);
    world.insert_resource(scene);
    world.insert_resource(live);
    world.insert_resource(events);
    world.insert_resource(player_state);
    world.insert_resource(effects);
    result
}
