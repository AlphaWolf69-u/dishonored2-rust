//! Shared ordinary model draws, with authored ammo/upgrade tag bindings
//! (44dd00/62b240 ->875e80). WorldModels owns runtime world actor lifetimes;
//! equipment owns its attachments. Both use the existing material/mesh owners.
use super::*;
use crate::{
    assets::equipment::{
        Descriptor as Weapon,
        attachment::{Descriptor, Geometry},
    },
    gameplay::{entity_lifetime::EntityId, reference_frame::ReferenceFrame},
};
use std::{collections::HashMap, sync::Arc};

pub(in crate::app) struct PreparedModel {
    descriptor: Arc<Descriptor>,
    geometry: PreparedGeometry,
}

enum PreparedGeometry {
    Rigid(Vec<ArmSurface>),
    Animated(ArmModel),
}

struct SurfaceTemplate {mesh:Handle<Mesh>,source_index:usize,probe_index:u32,material:MaterialHandle}
enum TemplateGeometry {Rigid(Vec<SurfaceTemplate>),Animated{model:String,info:Arc<crate::bmd6::ModelInfo>,skeleton:Arc<crate::edge_skeleton::EdgeSkeleton>,surfaces:Vec<SurfaceTemplate>}}
pub(in crate::app) struct Template {descriptor:Arc<Descriptor>,geometry:TemplateGeometry,lifetime:Option<Arc<()>>}
impl Template{
    pub(in crate::app) fn pipeline_surfaces(&self)->Vec<(bevy::asset::AssetId<Mesh>,bevy::asset::UntypedAssetId,bool)>{
        let (surfaces,skin)=match &self.geometry{TemplateGeometry::Rigid(s)=>(s,false),TemplateGeometry::Animated{surfaces,..}=>(surfaces,true)};
        surfaces.iter().map(|surface|(surface.mesh.id(),match &surface.material{MaterialHandle::Probe(handle)=>handle.id().untyped(),_=>unreachable!("ordinary model template material")},skin)).collect()
    }
    pub(in crate::app) fn retain_lease(&mut self, lease: Arc<()>) { self.lifetime = Some(lease); }
    pub(in crate::app) fn receiver_meshes(&self)->Vec<(Handle<Mesh>,bool)>{
        let (surfaces,skin)=match &self.geometry{TemplateGeometry::Rigid(s)=>(s,false),TemplateGeometry::Animated{surfaces,..}=>(surfaces,true)};
        surfaces.iter().map(|surface|(surface.mesh.clone(),skin)).collect()
    }
    pub(in crate::app) fn descriptor(&self)->&Arc<Descriptor>{&self.descriptor}
    pub(in crate::app) fn instantiate(&self,commands:&mut super::super::setup_commands::SetupCommands)->Result<PreparedModel,String>{
        let sources=match &self.geometry{TemplateGeometry::Rigid(surfaces)|TemplateGeometry::Animated{surfaces,..}=>surfaces};
        let mut surfaces=Vec::with_capacity(sources.len());
        for source in sources{
            let MaterialHandle::Probe(material)=&source.material else{return Err("world model template is not an ordinary material".into())};
            let mut entity=commands.spawn((Mesh3d(source.mesh.clone()),MeshMaterial3d(material.clone()),Transform::IDENTITY,GlobalTransform::IDENTITY,Visibility::Hidden,MeshTag(source.probe_index)));
            if !self.descriptor.cast_shadows{entity.insert(bevy::light::NotShadowCaster);}
            surfaces.push(ArmSurface{entity:entity.id(),_mesh:source.mesh.clone(),source_index:source.source_index,probe_index:source.probe_index,_probe_slot:None,material:source.material.clone()});
        }
        let geometry=match &self.geometry{
            TemplateGeometry::Rigid(_)=>PreparedGeometry::Rigid(surfaces),
            TemplateGeometry::Animated{model,info,skeleton,..}=>PreparedGeometry::Animated(ArmModel{managed_entity:None,model:model.clone(),joints:Vec::new(),inverse_bindposes:None,_equipment:None,skeleton:skeleton.clone(),info:info.clone(),surfaces,lighting:Default::default(),shown:false,world_bounds:None,depth_hack:1.}),
        };
        Ok(PreparedModel{descriptor:self.descriptor.clone(),geometry})
    }
}

/// Share the existing geometry/material builders, then discard their queued
/// ECS proxy reservations. The template retains assets only, no actor IDs.
pub(in crate::app) fn prepare_template(descriptor:Arc<Descriptor>,resolver:&mut MaterialResolver,meshes:&mut dyn AssetWrite<Mesh>,commands:&mut super::super::setup_commands::SetupCommands)->Result<Template,String>{
    let model=prepare_world(descriptor,resolver,meshes,commands)?;
    let project=|surface:ArmSurface|SurfaceTemplate{mesh:surface._mesh,source_index:surface.source_index,probe_index:surface.probe_index,material:surface.material};
    let geometry=match model.geometry{
        PreparedGeometry::Rigid(surfaces)=>TemplateGeometry::Rigid(surfaces.into_iter().map(project).collect()),
        PreparedGeometry::Animated(model)=>TemplateGeometry::Animated{model:model.model,info:model.info,skeleton:model.skeleton,surfaces:model.surfaces.into_iter().map(project).collect()},
    };
    Ok(Template{descriptor:model.descriptor,geometry,lifetime:None})
}

impl PreparedModel {
    pub(in crate::app) fn pipeline_surfaces(&self) -> Vec<super::super::world_models::pipelines::Surface> {
        let skinned=matches!(&self.geometry,PreparedGeometry::Animated(_));
        self.surfaces().iter().map(|surface|super::super::world_models::pipelines::Surface {
            mesh:surface._mesh.id(),material:match &surface.material {MaterialHandle::Probe(handle)=>handle.id().untyped(),_=>unreachable!("attachment ordinary material")},skinned,cpu_skin:false,alpha:None,
        }).collect()
    }
    pub(in crate::app) fn assign_probe_slots(&mut self, records: Option<&[[Vec4; 9]]>, world: &mut World,
        commands: &mut super::super::setup_commands::SetupCommands) -> Result<(), String> {
        let surfaces = match &mut self.geometry {
            PreparedGeometry::Rigid(surfaces) => surfaces,
            PreparedGeometry::Animated(model) => &mut model.surfaces,
        };
        super::assign_probe_slots(surfaces, records, world, commands)
    }
    pub(in crate::app) fn uploads(&self) -> Vec<(bevy::asset::AssetId<Mesh>, bevy::asset::UntypedAssetId)> {
        self.surfaces().iter().map(|surface| {
            let material = match &surface.material { MaterialHandle::Probe(handle)=>handle.id().untyped(), _=>unreachable!("model preparation admits ordinary probe materials") };
            (surface._mesh.id(),material)
        }).collect()
    }
    fn surfaces(&self) -> &[ArmSurface] {
        match &self.geometry {
            PreparedGeometry::Rigid(surfaces) => surfaces,
            PreparedGeometry::Animated(model) => &model.surfaces,
        }
    }
    pub(in crate::app) fn descriptor(&self) -> &Arc<Descriptor> {
        &self.descriptor
    }
}

pub(in crate::app) fn prepare(
    descriptor: Arc<Descriptor>,
    player_depth_hack: f32,
    resolver: &mut MaterialResolver,
    meshes: &mut dyn AssetWrite<Mesh>,
    commands: &mut super::super::setup_commands::SetupCommands,
) -> Result<PreparedModel, String> {
    let geometry = match &descriptor.geometry {
        Geometry::Rigid(source) => {
            let mut surfaces = Vec::new();
            for (index, surface) in source.surfaces.iter().enumerate() {
                if let Some(surface) = prepare_rigid_surface(
                    surface,
                    index,
                    descriptor
                        .placement
                        .custom_materials
                        .get(index)
                        .and_then(Option::as_deref),
                    player_depth_hack,
                    descriptor.cast_shadows,
                    resolver,
                    meshes,
                    commands,
                )? {
                    surfaces.push(surface);
                }
            }
            if surfaces.is_empty() {
                return Err(format!(
                    "{}: attachment has no drawable surfaces",
                    descriptor.entity
                ));
            }
            PreparedGeometry::Rigid(surfaces)
        }
        Geometry::Animated { rig, model } => {
            let (hack, parameter) = arm_model_depth_hack(player_depth_hack);
            let overrides = model
                .surfaces
                .iter()
                .enumerate()
                .map(|(index, surface)| {
                    descriptor
                        .placement
                        .custom_materials
                        .get(index)
                        .and_then(Option::as_deref)
                        .unwrap_or(&surface.material_path)
                        .to_owned()
                })
                .collect::<Vec<_>>();
            let model = prepare_model(
                descriptor.model.clone(),
                model.clone(),
                rig.skeleton.clone(),
                hack,
                parameter,
                &overrides,
                true,
                resolver,
                meshes,
                commands,
            )?;
            if descriptor.cast_shadows {
                for surface in &model.surfaces {
                    let entity = surface.entity;
                    commands.queue(move |world: &mut World| {
                        world
                            .entity_mut(entity)
                            .remove::<bevy::light::NotShadowCaster>();
                    });
                }
            }
            PreparedGeometry::Animated(model)
        }
    };
    Ok(PreparedModel {
        descriptor,
        geometry,
    })
}

pub(in crate::app) fn prepare_world(descriptor:Arc<Descriptor>,resolver:&mut MaterialResolver,meshes:&mut dyn AssetWrite<Mesh>,commands:&mut super::super::setup_commands::SetupCommands)->Result<PreparedModel,String>{
    let model=prepare(descriptor,0.0,resolver,meshes,commands)?;
    //873340 can enable any actual model instance later. Retain the source
    //shadow vertex ABI once in its shared mesh, irrespective of the initial
    //instance bit; toggling never rebuilds/clones geometry or templates.
    {
        for part in model.surfaces() {
            let mesh = meshes.get_mut(&part._mesh).ok_or("world shadow mesh preparation missing")?;
            match &model.descriptor.geometry {
                Geometry::Rigid(source) => shadow_preparation::retain_model_shadow_position(mesh, &source.surfaces[part.source_index].positions),
                Geometry::Animated { model: source, .. } => {
                    let surface = &source.surfaces[part.source_index];
                    shadow_preparation::retain_model_shadow_position(mesh, &surface.positions);
                    shadow_preparation::retain_skin_inputs(mesh, surface);
                }
            }
        }
    }
    for surface in model.surfaces(){let entity=surface.entity;commands.queue(move|world:&mut World|{
        world.entity_mut(entity).remove::<bevy::camera::visibility::NoFrustumCulling>();
    });}
    Ok(model)
}

struct Model {
    prepared: PreparedModel,
    ///Mutable actual instance state; the shared descriptor is not modified.
    cast_shadows: bool,
    // Source weapon rig is the actual tag owner, retained while bound.
    weapon: Option<Arc<Weapon>>,
    parent: Option<EntityId>,
    binding: Option<crate::gameplay::player::equipment_attachment::Binding>,
    world: Option<ReferenceFrame>,
    world_scale: Option<[f32;3]>,
    node: Option<crate::gameplay::player::animation::AnimBasicNode>,
    command: Option<crate::gameplay::player::animation::AnimBasicCommand>,
    visible: bool,
    shown: bool,
    frame: Option<ReferenceFrame>,
    bounds: Option<[[f32; 3]; 2]>,
    lighting: crate::probe_lighting::ProbeQuery,
    /// Current game-space skin palette shared by both custom shadow passes.
    /// Kept with the actual actor through the existing retirement receipt.
    shadow_skin: Option<(Handle<ShaderBuffer>, Vec<[[f32; 4]; 3]>)>,
}

#[derive(Default)]
pub(in crate::app) struct Models {
    live: HashMap<EntityId, Model>,
}
impl Models {
    /// World actors share geometry/material/skinning primitives, while their
    /// actual entity frame and lifecycle stay with the world-model owner.
    pub(in crate::app) fn publish_world(&mut self, id: EntityId, prepared: PreparedModel, frame: ReferenceFrame, visible: bool) -> Result<(), String> {
        if self.live.contains_key(&id) { return Err("world model generation already published".into()); }
        // Runtime AnimBase belongs to its real component pool, not rendering.
        let node = None;
        let cast_shadows=prepared.descriptor.cast_shadows;
        self.live.insert(id, Model { prepared, cast_shadows, weapon:None,parent:None,binding:None,world:Some(frame),world_scale:None,node,command:None,
            visible,shown:false,frame:Some(frame),bounds:None,lighting:Default::default(),shadow_skin:None });
        Ok(())
    }
    pub(in crate::app) fn set_world_frame(&mut self, id:EntityId, frame:ReferenceFrame)->Result<(),String>{
        let model=self.live.get_mut(&id).ok_or("world model generation is absent")?;
        if model.weapon.is_some(){return Err("bound equipment cannot accept a world actor frame".into());}
        model.world=Some(frame);Ok(())
    }
    pub(in crate::app) fn set_world_scale(&mut self,id:EntityId,scale:[f32;3])->Result<(),String>{
        self.live.get_mut(&id).ok_or("world model generation is absent")?.world_scale=Some(scale);Ok(())
    }
    pub(in crate::app) fn projection_draws(&self,id:EntityId)->Option<impl Iterator<Item=(Entity,&Handle<Mesh>,usize,u32,&MaterialHandle)> + '_>{
        let model=self.live.get(&id)?;
        Some(model.prepared.surfaces().iter().map(|surface|(surface.entity,&surface._mesh,surface.source_index,surface.probe_index,&surface.material)))
    }
    /// Color skin palettes already include the world frame, so the draw's
    /// identity transform needs actual world-space bounds, not bind-pose bounds.
    pub(in crate::app) fn publish_animated_bounds(&self,
        bounds: &mut Query<&mut bevy::camera::primitives::Aabb, With<Mesh3d>>,
        commands: &mut Commands) {
        for model in self.live.values() {
            if model.world.is_none() || !matches!(&model.prepared.geometry, PreparedGeometry::Animated(_)) { continue }
            let Some([low, high]) = model.bounds else { continue };
            let actual = bevy::camera::primitives::Aabb::from_min_max(
                Vec3::new(low[0], low[2], -high[1]), Vec3::new(high[0], high[2], -low[1]));
            for part in model.prepared.surfaces() {
                if let Ok(mut previous) = bounds.get_mut(part.entity) {
                    if previous.center != actual.center || previous.half_extents != actual.half_extents { *previous = actual; }
                } else { commands.entity(part.entity).insert(actual); }
            }
        }
    }
    pub(in crate::app) fn shadow_surfaces(&self, id: EntityId,
        materials: &super::super::ShadowMaterials) -> Result<Option<([[f32; 3]; 2], Vec<super::super::shadow_updates::RuntimeShadowSurface>)>, String> {
        let Some(model) = self.live.get(&id) else { return Ok(None) };
        if !model.cast_shadows { return Ok(None) }
        let (Some(frame), Some(bounds)) = (model.frame, model.bounds) else { return Ok(None) };
        let placement = &model.prepared.descriptor.placement;
        let affine = render_model::affine_rows(frame.position, frame.basis, model.world_scale.unwrap_or(placement.scale));
        let rigid = [affine[0], affine[1], affine[2], [0., 0., 0., 1.]];
        let mut surfaces = Vec::new();
        for part in model.prepared.surfaces() {
            let Some(inputs) = materials.0.get(&part.material) else { return Err("world caster material owner missing".into()) };
            let inputs = inputs.as_ref().map_err(Clone::clone)?;
            if inputs.world_gather_flags48()? & 1 == 0 { continue; }
            let skin = match &model.prepared.geometry {
                PreparedGeometry::Rigid(_) => None,
                PreparedGeometry::Animated(arm) => {
                    let surface = &arm.info.surfaces[part.source_index];
                    let (handle, _) = model.shadow_skin.as_ref().ok_or("world caster current skin palette missing")?;
                    Some(super::super::shadow_updates::RuntimeSurfaceSkin {
                        palette: handle.clone(), matrix_offset: surface.matrix_offset, influences: surface.skin_influences,
                        raster_model: rigid,
                    })
                }
            };
            surfaces.push(super::super::shadow_updates::RuntimeShadowSurface {
                mesh: part._mesh.clone(), material: part.material.clone(),
                // Skinned matrices already carry the actual world frame.
                models: vec![if skin.is_some() { Mat4::IDENTITY.to_cols_array_2d() } else { rigid }],
                bounds: vec![bounds], skin, clock:None,
            });
        }
        Ok(Some((bounds, surfaces)))
    }
    pub(in crate::app) fn set_shadow_casting(&mut self,id:EntityId,value:bool)->Result<(),String>{
        self.live.get_mut(&id).ok_or("model shadow property targeted absent generation")?.cast_shadows=value;Ok(())
    }
    pub(in crate::app) fn shadow_entities(&self,id:EntityId)->Vec<Entity>{
        self.live.get(&id).into_iter().flat_map(|model|model.prepared.surfaces().iter().map(|surface|surface.entity)).collect()
    }
    pub(in crate::app) fn shadow_casting(&self,id:EntityId)->Option<bool>{
        self.live.get(&id).map(|model|model.cast_shadows)
    }
    pub(in crate::app) fn draw_entities(&self)->impl Iterator<Item=Entity>+'_ {
        self.live.values().flat_map(|model|model.prepared.surfaces().iter().map(|surface|surface.entity))
    }
    pub(in crate::app) fn view_visible(&self,id:EntityId,views:&Query<&ViewVisibility>)->bool{
        self.live.get(&id).is_some_and(|model|model.prepared.surfaces().iter().any(|surface|views.get(surface.entity).is_ok_and(|visibility| visibility.get())))
    }
    pub(in crate::app) fn submit_area(&self,id:EntityId,area:f32,bindings:&bevy::platform::collections::HashMap<MaterialHandle,Vec<usize>>,demands:&[crate::texture::ProjectedAreaDemand])->Result<(),String>{
        let Some(model)=self.live.get(&id).filter(|model|model.shown)else{return Ok(())};
        for surface in model.prepared.surfaces(){for &slot in bindings.get(&surface.material).into_iter().flatten(){demands.get(slot).ok_or("world model material references an unknown image")?.submit_dynamic(area);}}
        Ok(())
    }
    /// The caller supplies visibility from the actual magazine callback. There
    /// is no inferred loaded round or fallback bind joint.
    pub(in crate::app) fn publish(
        &mut self,
        id: EntityId,
        parent: EntityId,
        weapon: Arc<Weapon>,
        binding: crate::gameplay::player::equipment_attachment::Binding,
        prepared: PreparedModel,
        visible: bool,
    ) -> Result<(), String> {
        if self.live.contains_key(&id) {
            return Err("ammunition model generation already published".into());
        }
        let node = match &prepared.descriptor.geometry {
            Geometry::Animated { rig, .. } => Some(rig.template.clone()),
            Geometry::Rigid(_) => None,
        };
        let cast_shadows=prepared.descriptor.cast_shadows;
        self.live.insert(
            id,
            Model {
                prepared,
                cast_shadows,
                weapon: Some(weapon),
                parent: Some(parent),
                binding: Some(binding),
                world: None,
                world_scale: None,
                node,
                command: None,
                visible,
                shown: false,
                frame: None,
                bounds: None,
                lighting: Default::default(),
                shadow_skin: None,
            },
        );
        Ok(())
    }

    pub(in crate::app) fn set_visible(
        &mut self,
        id: EntityId,
        visible: bool,
    ) -> Result<(), String> {
        self.live
            .get_mut(&id)
            .ok_or("ammunition visibility targeted an absent generation")?
            .visible = visible;
        Ok(())
    }
    /// Native62b440 asks for the real attachment and its own hidden flag;
    /// frustum/view omission must not disable firing.
    pub(in crate::app) fn visible(&self, id: EntityId) -> bool {
        self.live.get(&id).is_some_and(|model| model.visible)
    }
    pub(in crate::app) fn frame(&self, id: EntityId) -> Option<ReferenceFrame> {
        self.live.get(&id).and_then(|model| model.frame)
    }
    pub(in crate::app) fn has(&self, id: EntityId) -> bool {
        self.live.contains_key(&id)
    }

    pub(in crate::app) fn animation_ready(&self, id: EntityId) -> bool {
        self.live.get(&id).is_some_and(|model| model.node.is_some())
    }
    pub(in crate::app) fn animate(
        &mut self,
        id: EntityId,
        play: bool,
        looping: bool,
    ) -> Result<bool, String> {
        use crate::gameplay::player::animation::{AnimBasicCommand, PlayMode};
        let Some(model) = self.live.get_mut(&id) else {
            return Ok(false);
        };
        let Some(node) = model.node.as_ref() else {
            return Ok(false);
        };
        if play
            && looping
            && node
                .current_clip()
                .is_some_and(|(item, _, playing)| item == 0 && playing)
        {
            return Ok(true);
        }
        model.command = Some(if play {
            AnimBasicCommand {
                idx: 0,
                weight: 1.,
                mode: if looping {
                    PlayMode::Loop
                } else {
                    PlayMode::Once
                },
                rate: 1.,
                start: 0.,
                play_on_end: false,
            }
        } else {
            AnimBasicCommand::STOP
        });
        Ok(true)
    }
    pub(in crate::app) fn has_alias(&self, id: EntityId, alias: &str) -> bool {
        self.live
            .get(&id)
            .and_then(|model| model.node.as_ref())
            .is_some_and(|node| node.index_of(alias).is_some())
    }
    pub(in crate::app) fn animate_alias(
        &mut self,
        id: EntityId,
        alias: &str,
        mode: crate::gameplay::player::animation::PlayMode,
    ) -> Result<bool, String> {
        use crate::gameplay::player::animation::AnimBasicCommand;
        let Some(model) = self.live.get_mut(&id) else {
            return Ok(false);
        };
        let Some(node) = model.node.as_ref() else {
            return Ok(false);
        };
        let Some(index) = node.index_of(alias) else {
            return Err(format!(
                "attachment has no authored AnimBasic alias {alias}"
            ));
        };
        model.command = Some(AnimBasicCommand {
            idx: index as i32,
            weight: 1.,
            mode,
            rate: 1.,
            start: 0.,
            play_on_end: false,
        });
        Ok(true)
    }

    pub(in crate::app) fn update(
        &mut self,
        renders: &[ArmRender],
        visibility: &mut Query<&mut Visibility>,
        transforms: &mut Query<
            (&mut Transform, &mut GlobalTransform),
            (With<Mesh3d>, Without<Camera3d>),
        >,
        probes: Option<&crate::probe_lighting::ProbeLightingSet>,
        mut lighting: Option<&mut model_motion::ModelProbeLighting>,
        patches: &mut crate::rendering::bevy_backend::shader_buffer_patches::ShaderBufferPatches,
        buffers: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        inverse_bindposes: &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
        joint_transforms: &mut Query<&mut GlobalTransform, Without<Mesh3d>>,
        identities: Option<&super::super::equipment_identity::Identities>,
        clock: Option<&crate::gameplay::game_clock_runtime::ClockRuntime>,
        world_poses: Option<&super::super::physics_scene::PhysicsScene>,
    ) -> Result<(), String> {
        for (&id, model) in &mut self.live {
            let render = model.weapon.as_ref().and_then(|weapon| weapon.visual.as_ref()).and_then(|visual| renders.iter().find(|render| {
                render.managed_entity == model.parent && render.model == visual.model
            }));
            let parent = identities
                .and_then(|identities| model.parent.and_then(|parent|identities.published_frame(parent)))
                .map(|(frame, _)| frame);
            let joint = render
                .and_then(|render| {
                    model.binding.as_ref().and_then(|binding|binding.joint)
                        .and_then(|joint| render.joints.get(joint))
                })
                .map(|rows| ReferenceFrame {
                    position: [rows[0][3], rows[1][3], rows[2][3]],
                    basis: std::array::from_fn(|column| {
                        std::array::from_fn(|row| rows[row][column])
                    }),
                });
            let frame = model.world.or_else(|| parent.and_then(|parent| model.binding.as_ref().and_then(|binding|binding.world(parent, joint))));
            model.frame = frame;
            let shown =
                model.visible && (model.world.is_some() || render.is_some_and(|render| render.visible)) && frame.is_some();
            if let Some(frame) = frame {
                let placement = &model.prepared.descriptor.placement;
                let scale=model.world_scale.unwrap_or(placement.scale);
                let bounds = render_model::transformed_bounds(
                    model.prepared.descriptor.geometry.bounds(),
                    frame.position,
                    frame.basis,
                    scale,
                );
                model.bounds = Some(bounds);
                publish_model_lighting(
                    bounds,
                    &mut model.lighting,
                    model.prepared.surfaces(),
                    probes,
                    lighting.as_deref_mut(),
                    patches,
                    buffers,
                );
                match &mut model.prepared.geometry {
                    PreparedGeometry::Rigid(surfaces) => {
                        let rows =
                            render_model::affine_rows(frame.position, frame.basis, scale);
                        let matrix = std::array::from_fn(|index| rows[index / 4][index % 4]);
                        let transform = super::super::view_geometry::game_affine_to_view(&matrix);
                        let global = GlobalTransform::from(transform);
                        for part in surfaces {
                            let (mut local, mut published) = transforms
                                .get_mut(part.entity)
                                .map_err(|_| "attachment surface transform is absent")?;
                            *local = transform;
                            *published = global;
                        }
                    }
                    PreparedGeometry::Animated(arm) => {
                        let Geometry::Animated { rig, .. } = &model.prepared.descriptor.geometry
                        else {
                            return Err("animated attachment lost its source rig".into());
                        };
                        let equipment_matrices;
                        let matrices = if model.world.is_some() {
                            world_poses.ok_or("runtime world model has no component pose owner")?
                                .runtime_animation_matrices(id)?
                                .ok_or("runtime animated model has no actual AnimBase pose")?
                        } else {
                            let node = model.node.as_mut().ok_or("animated attachment lost its actual AnimBasic state")?;
                            let clock = clock.ok_or("animated attachment has no canonical clock owner")?;
                            let delta = if clock.advanced() { clock.entity_delta(id).map_err(str::to_owned)? } else { 0. };
                            node.update(delta, model.command.take(), &rig.animations);
                            let pose = node.evaluate(&rig.animations, &rig.skeleton.rest)?;
                            let joints = pose.as_ref().map_or(rig.skeleton.rest.as_slice(), |pose| pose.joints.as_slice());
                            equipment_matrices = edge_skeleton::pose_matrices(joints, &rig.skeleton.hierarchy, &rig.root)?;
                            equipment_matrices.as_slice()
                        };
                        let axis = Mat3::from_cols_array_2d(&frame.basis)
                            * Mat3::from_diagonal(Vec3::from_array(scale));
                        let position = Vec3::from_array(frame.position);
                        let rows = matrices
                            .iter()
                            .map(|rows| {
                                let local_position = Vec3::new(rows[0][3], rows[1][3], rows[2][3]);
                                let local_axis =
                                    Mat3::from_cols_array_2d(&std::array::from_fn(|column| {
                                        std::array::from_fn(|row| rows[row][column])
                                    }));
                                let point = position + axis * local_position;
                                let rotation = axis * local_axis;
                                std::array::from_fn(|row| {
                                    [
                                        rotation.x_axis[row],
                                        rotation.y_axis[row],
                                        rotation.z_axis[row],
                                        point[row],
                                    ]
                                })
                            })
                            .collect();
                        let render = ArmRender {
                            model: arm.model.clone(),
                            managed_entity: None,
                            joints: rows,
                            visible: shown,
                        };
                        let palette = publish_model_skin(
                            arm,
                            &render,
                            commands,
                            inverse_bindposes,
                            joint_transforms,
                        )?;
                        if model.cast_shadows {
                            if let Some((handle, previous)) = &mut model.shadow_skin {
                                if *previous != palette {
                                    let bytes = palette.iter().flat_map(|matrix| matrix.iter().flatten())
                                        .flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>();
                                    patches.enqueue(handle.id(), 0, &bytes);
                                    *previous = palette;
                                }
                            } else {
                                let gpu = palette.iter().map(|matrix| matrix.map(Vec4::from_array)).collect::<Vec<_>>();
                                let mut buffer = ShaderBuffer::from(gpu);
                                buffer.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
                                model.shadow_skin = Some((buffers.add(buffer), palette));
                            }
                        }
                    }
                }
            } else {
                model.bounds = None;
            }
            if model.shown != shown {
                for part in model.prepared.surfaces() {
                    *visibility
                        .get_mut(part.entity)
                        .map_err(|_| "ammunition surface visibility is absent")? = if shown {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                }
                model.shown = shown;
            }
        }
        Ok(())
    }

    /// Despawn real draw entities, then transfer every source/geometry/material
    /// lease to PlayerArms' existing submitted-work acknowledgement.
    pub(in crate::app) fn remove(
        &mut self,
        id: EntityId,
        commands: &mut super::super::setup_commands::SetupCommands,
    ) -> Option<equipment::RetiredModel> {
        let model = self.live.remove(&id)?;
        let mut entities = model
            .prepared
            .surfaces()
            .iter()
            .map(|surface| surface.entity)
            .collect::<Vec<_>>();
        if let PreparedGeometry::Animated(arm) = &model.prepared.geometry {
            entities.extend_from_slice(&arm.joints);
        }
        commands.queue(move |world: &mut World| {
            for entity in entities {
                if world.entities().contains(entity) {
                    world.despawn(entity);
                }
            }
        });
        Some(equipment::RetiredModel::new(model))
    }

    pub(in crate::app) fn shown_materials(&self) -> impl Iterator<Item = &MaterialHandle> {
        self.live
            .values()
            .filter(|model| model.shown)
            .flat_map(|model| {
                model
                    .prepared
                    .surfaces()
                    .iter()
                    .map(|surface| &surface.material)
            })
    }
    pub(in crate::app) fn submit_texture_demands(
        &self,
        state: Option<&crate::visibility::tome::RenderQueryState>,
        bindings: &bevy::platform::collections::HashMap<MaterialHandle, Vec<usize>>,
        demands: &[crate::texture::ProjectedAreaDemand],
    ) -> Result<(), String> {
        for model in self.live.values().filter(|model| model.shown) {
            let bounds = model
                .bounds
                .ok_or("shown ammunition has no published bounds")?;
            let (_, area) =
                crate::visibility::tome::render_query_aabb_visibility_without_depth(state, bounds)?;
            for part in model.prepared.surfaces() {
                for &slot in bindings.get(&part.material).into_iter().flatten() {
                    demands
                        .get(slot)
                        .ok_or("ammunition material references an unknown image")?
                        .submit_dynamic(area.unwrap_or(1.0));
                }
            }
        }
        Ok(())
    }
}
