//! Native local-light CPU ownership. Projection/request publication is separate
//! from GPU validity: this module never labels an undrawn atlas ready.
use super::*;

#[derive(Component,Clone,Copy)]
pub(super) struct NativeLocalLightSource(pub usize);

#[derive(Resource,Default)]
pub(super) struct NativeLocalShadowViewControls(pub native_local_shadow_cache::LocalShadowCacheControls);

impl NativeLocalShadowViewControls {
    /// Runtime actor owners supply their real graph and resolved actor list.
    /// The resulting view signals are consumed before the next local prepare.
    pub fn set_actors_dynamic_caster(&mut self,access:&mut impl render_model::ActorCasterAccess,
        actors:impl IntoIterator<Item=Option<usize>>,dynamic:bool)->bool {
        render_model::set_actors_dynamic_caster_for_view(access,actors,dynamic,&mut self.0)
    }
}

#[derive(Resource)]
pub(super) struct NativeLocalShadowOwner {
    pub settings:original_settings::LocalShadowSettings,
    pub atlas_image:Handle<Image>,pub cache_image:Handle<Image>,
    pub atlas:render_shadow::ShadowAtlasAllocator,
    pub cache:native_local_shadow_cache::LocalShadowCache,
    pub world_query:native_local_world_query::NativeLocalWorldQuery,
    pub precomputed_world_casters:HashMap<usize,Vec<Vec<u32>>>,
    pub query_job_count:u32,
    pub hardware_bias_point:[f32;3],pub hardware_bias_spot:[f32;3],
    pub copy_state:[u32;9],
    /// 043800 clears source view-definition WORD+64;374e40 copies it to
    /// prepared view+3a34. This is the rewrite's ordinary main view owner.
    pub view_flags:u16,
}

pub(super) struct NativeLocalProjectedLight {
    pub entity:Entity,pub request:render_shadow::LocalShadowRequest,
    pub clips:Vec<render_shadow::ShadowMatrix>,pub crops:Vec<[f32;4]>,
    pub sides:Vec<Option<NativeLocalShadowSide>>,
    pub receiver_sides:Vec<(render_shadow::AtlasAllocation,render_shadow::ShadowMatrix)>,
    pub bounds:[[f32;3];2],
    pub world_casters:Vec<Vec<u32>>,
    ///3285a0/328900 whole-light surface lists, static then dynamic. Each
    ///entry retains its model source and independently owned draw instance.
    pub model_casters:[Vec<(usize,usize)>;2],
    pub refresh_dynamic:bool,
}

pub(super) struct NativeLocalShadowSide {
    pub allocation:render_shadow::AtlasAllocation,
    pub geometry:render_shadow::LocalShadowSideGeometry,
    pub cache_key:u32,
    pub cache:Option<native_local_shadow_cache::LocalShadowCacheEntry>,
    /// Metadata match only. GPU publication also requires the render owner's
    /// acknowledgement of this exact cache allocation's completed draw.
    pub cache_match:bool,
}

#[derive(Resource,Default)]
pub(super) struct NativeLocalShadowCpu {
    pub current:Option<(u32,Vec<NativeLocalProjectedLight>)>,
    error:Option<String>,reported:bool,
    verified_parallel:bool,
}

type LocalProjection=(Vec<render_shadow::ShadowMatrix>,Vec<[f32;4]>);
type LocalModelBuckets=[Vec<(usize,usize)>;2];
type LocalModelInputs=[(Vec<render_model::LocalModelCasterRecord>,Vec<(usize,usize)>);2];

fn collect_local_light_models(request:&render_shadow::LocalShadowRequest,light:&entities::PlacedLight,
    view_flags:u16,camera_clip:&render_shadow::ShadowMatrix,inputs:&LocalModelInputs,
)->Result<LocalModelBuckets,String> {
    let (volume_clip,_)=render_shadow::local_light_volume_projection(light.position,light.rotation,
        request.candidate.kind as u32,request.candidate.range,authored_light_cosines(light.angular_cutoff,light.angular_falloff).0)?;
    Ok(std::array::from_fn(|bucket| {
        let (records,keys)=&inputs[bucket];
        //32d4b0 initializes definition+20 to zero for both model passes;
        //3285a0/328900 therefore retain their original radial sorting.
        render_model::collect_local_model_casters(records,&volume_clip,camera_clip,light.position,
            bucket!=0,view_flags,false).into_iter().map(|index|keys[index]).collect()
    }))
}

fn project_local_shadow(request:&render_shadow::LocalShadowRequest,light:&entities::PlacedLight,
    view_flags:u16,camera:&render_shadow::SpotFitCamera<'_>,scratch:&mut render_shadow::ProjectionFitScratch,
)->Result<LocalProjection,String> {
    let near=light.shadow_near_plane.unwrap_or(0.1);
    let mode=render_shadow::local_shadow_fit_mode(request.priority,request.candidate.flags[0],view_flags,2);
    let mut crops=vec![[-1.0,-1.0,1.0,1.0];request.side_count];
    let clips=if request.candidate.kind==2 {
        render_shadow::prepare_spot_projections(&render_shadow::SpotProjectionInput {position:light.position,basis:light.rotation,
            cutoff_cosine:authored_light_cosines(light.angular_cutoff,light.angular_falloff).0,near,far:request.candidate.range,
            side_count:request.side_count,fit_mode:mode},Some(camera),&mut crops,scratch)?
    } else {
        let mut bank:[[_;4];6]=crops.as_slice().try_into().unwrap();
        let clips=render_shadow::prepare_omni_projections(light.position,near,request.candidate.range,mode,Some(camera),&mut bank,scratch)?;
        crops.copy_from_slice(&bank);clips.to_vec()
    };
    Ok((clips,crops))
}

struct LocalStaticInputs {
    clip:render_shadow::ShadowMatrix,world:Vec<u32>,models:Vec<(usize,usize)>,
    clocks:game_clock::GameplayViewClocks,dimensions:[u32;2],
    prepared:Option<(Vec<native_shadow_scene::NativeShadowInstance>,bool)>,
}

#[derive(Resource,Default)]
pub(super) struct NativeLocalShadowDraws {
    static_inputs:HashMap<u32,LocalStaticInputs>,matrices:Option<Handle<ShaderBuffer>>,
    error:Option<String>,reported:bool,
}

fn local_side_models(records:&[(usize,usize)],clip:render_shadow::ShadowMatrix,
    sources:&native_shadow_live::NativeModelShadowSources)->Vec<(usize,usize)> {
    let clip=std::array::from_fn(|i|clip[i/4][i%4]);
    //329120 repeats side-frustum admission after the whole-light collector.
    records.iter().copied().filter(|&(source,_)| {
        let bounds=sources.0[source].bounds;
        !render_model::native_transformed_aabb_clip_cull(&clip,bounds[0],bounds[1]).culled
    }).collect()
}

pub(super) fn publish_native_local_shadow_draws(
    owner:Option<Res<NativeLocalShadowOwner>>,cpu:Res<NativeLocalShadowCpu>,scene:Res<SceneData>,
    sources:Res<NativeWorldShadowMeshSources>,model_sources:Res<native_shadow_live::NativeModelShadowSources>,
    placements:Res<RenderSourcePlacements>,materials:Res<NativeShadowMaterials>,meshes:Res<Assets<Mesh>>,
    images:Res<Assets<Image>>,vegetation:Res<NativeVegetationMeshInputs>,
    clocks:Res<game_clock_runtime::NativeGameplayViewClock>,mut queue:ResMut<SceneRenderQueue>,
    mut buffers:ResMut<Assets<ShaderBuffer>>,mut draws:ResMut<NativeLocalShadowDraws>,
    mut output:ResMut<native_shadow_scene::NativeLocalShadowSceneFrame>,
    (completion,profile):(Res<native_shadow_scene::NativeLocalShadowCompletion>,Option<Res<frame_profile::CpuFrameProfile>>),
) {
    let _span=profile.as_deref().map(|p|p.measure(25));
    output.0=None;
    let (Some(owner),Some((frame,lights)))=(owner,cpu.current.as_ref()) else {return;};
    let result=(||->Result<native_shadow_scene::NativeLocalShadowSubmission,String>{
        let clocks=*clocks.clocks().map_err(str::to_owned)?;
        let quality=scene.shadow_quality.as_ref().ok_or("native local shadow quality is unavailable")?;
        let sort_shadows_ccw=scene.shadow_sort_ccw.ok_or("native WORLD shadow sorting setting is unavailable")?;
        let completed=completion.0.lock().map_err(|_|"native local completion lock poisoned")?.as_ref()
            .filter(|(_,atlas,cache,_)|*atlas==owner.atlas_image.id() && *cache==owner.cache_image.id())
            .map(|(_,_,_,entries)|entries.clone()).unwrap_or_default();
        // Retain the first source generation before resolving streaming assets.
        // A delayed GPU upload must not replace its pose, clock or caster list.
        for light in lights {for (side_index,side) in light.sides.iter().enumerate() {
            let Some(side)=side else {continue;};let Some(cache)=&side.cache else {continue;};
            let allocation=cache.allocation;
            draws.static_inputs.entry(allocation.id).or_insert_with(||LocalStaticInputs {
                clip:light.clips[side_index],world:light.world_casters[side_index].clone(),
                models:local_side_models(&light.model_casters[0],light.clips[side_index],&model_sources),
                clocks,dimensions:[(allocation.rect[2]-allocation.rect[0]+1) as u32,(allocation.rect[3]-allocation.rect[1]+1) as u32],prepared:None,
            });
        }}
        draws.static_inputs.retain(|id,_|lights.iter().any(|l|l.sides.iter().flatten().any(|s|s.cache.as_ref().is_some_and(|c|c.allocation.id==*id))));
        if draws.matrices.is_none() {draws.matrices=Some(buffers.add(ShaderBuffer::from(scene.world.matrices.clone())));}
        let matrices=draws.matrices.clone();
        let mut layers=Vec::new();let mut regions=Vec::new();
        for light in lights {
            let bias=if light.request.candidate.kind==2 {owner.hardware_bias_spot}else{owner.hardware_bias_point};
            for (side_index,side) in light.sides.iter().enumerate() {
                let Some(side)=side else {continue;};
                let clip=light.clips[side_index];
                let allocation=side.allocation;
                let dimensions=[(allocation.rect[2]-allocation.rect[0]+1) as u32,(allocation.rect[3]-allocation.rect[1]+1) as u32];
                let cache=side.cache.as_ref().map(|c|c.allocation);
                let mut uncached=LocalStaticInputs {clip,world:light.world_casters[side_index].clone(),
                    models:local_side_models(&light.model_casters[0],clip,&model_sources),clocks,dimensions,prepared:None};
                let input=if let Some(cache)=cache {draws.static_inputs.get_mut(&cache.id).unwrap()}else{&mut uncached};
                if input.prepared.is_none() {
                    let target=native_shadow_scene::NativeParallelShadowTargetDimensions {width:input.dimensions[0],height:input.dimensions[1],layers:1,depth_32_bits:quality.depth_32_bits};
                    let settings=NativeWorldShadowDrawSettings {dimensions:target,hardware_bias:bias,sort_shadows_ccw,
                        source_flag_04:game_clock::RESET_VIEW_LOCAL_TIME_FLAG,exclude_lod:true,depth_clip:1};
                    let mut mask=vec![0;scene.world.visibility.len()];
                    for &id in &input.world {*mask.get_mut(id as usize).ok_or("native local WORLD ID exceeds world records")?=1;}
                    let plans=prepare_native_world_shadow_masks_with_settings(&settings,&scene.world,&sources,&materials,&meshes,&images,
                        &mut queue,&[mask],&input.clip,Some(input.clocks.view_shader_times(settings.source_flag_04).local_time),None)?;
                    let models=native_shadow_live::prepare_native_model_shadow_instances(&input.models,input.clip,bias,0.0,
                        input.dimensions,&input.clocks,&model_sources,&placements,&materials,&images,&vegetation,&mut queue)?;
                    let contains_models=!models.is_empty();
                    let mut built=build_native_world_layer(plans,matrices.as_ref(),target,&mut buffers).instances;
                    built.extend(models);input.prepared=Some((built,contains_models));
                }
                let static_prepared=!cache.is_some_and(|c|completed.contains_key(&c.id));
                let (mut instances,contains_models)=if static_prepared {input.prepared.as_ref().unwrap().clone()}
                    else {(Vec::new(),input.prepared.as_ref().unwrap().1)};
                let static_instances=instances.len();
                let dynamic=if light.refresh_dynamic {local_side_models(&light.model_casters[1],clip,&model_sources)}else{Vec::new()};
                instances.extend(native_shadow_live::prepare_native_model_shadow_instances(&dynamic,clip,bias,0.0,
                    dimensions,&clocks,&model_sources,&placements,&materials,&images,&vegetation,&mut queue)?);
                layers.push(native_shadow_scene::NativeShadowLayer {ops:native_shadow_scene::NativeShadowLayerOps {
                    depth:native_shadow_scene::NativeShadowLoad::Load,stencil:native_shadow_scene::NativeStencilLoad::Load},
                    cache:native_shadow_scene::NativeShadowCacheOp::None,instances});
                regions.push(native_shadow_scene::NativeLocalShadowRegion {atlas:allocation,cache,static_instances,contains_models,static_prepared});
            }
        }
        let receiver=(||->Result<native_shadow_scene::NativeLocalShadowReceiver,String>{
            let environment=scene.environment_render_parameters.as_ref().ok_or("native local receiver has no environment parameter owner")?;
            let bias=render_shadow::parallel_shader_bias(environment,quality.depth_32_bits)?;
            let mut ordered=lights.iter().collect::<Vec<_>>();ordered.sort_by_key(|l|l.request.candidate.source);
            let mut receiver=native_shadow_scene::NativeLocalShadowReceiver {bias,lights:Vec::new(),shadows:Vec::new()};
            for light in ordered {
                let count=light.sides.len();let first=receiver.shadows.len();
                //2f7260 admits entire ranges in gathered-light order. A full
                //256-record buffer rejects later ranges without wrapping.
                if first+count>tiled_lighting::SHADOW_BUFFER_CAPACITY {continue;}
                for (index,(allocation,atlas)) in light.receiver_sides.iter().enumerate() {
                    let mut descriptor=[0u8;256];descriptor[4..8].copy_from_slice(&(allocation.slice as u32).to_le_bytes());
                    for (i,value) in atlas.iter().flatten().enumerate() {
                        descriptor[32+i*4..36+i*4].copy_from_slice(&value.to_le_bytes());
                    }
                    // Native culling leaves the inverse untouched. It is not
                    // read by the perspective local receiver; reserve its ABI
                    // bytes rather than inventing an inverse for a culled face.
                    if let Some(side)=&light.sides[index] {
                        for (i,value) in side.geometry.inverse_atlas.iter().flatten().enumerate() {
                            descriptor[96+i*4..100+i*4].copy_from_slice(&value.to_le_bytes());
                        }
                    }
                    let mut packet=[0;36];render_light::fill_tlf_shadow_packet(&descriptor,&mut packet);receiver.shadows.push(packet);
                }
                let source=&scene.placed_lights[light.request.candidate.source];
                receiver.lights.push(native_shadow_scene::NativeLocalShadowReceiverLight {entity:light.entity,
                    kind:light.request.candidate.kind as u32,position:source.position,left:source.rotation[1],
                    direction:source.rotation[0].map(|v|-v),first:first as u32,count:count as u32});
            }
            Ok(receiver)
        })();
        Ok(native_shadow_scene::NativeLocalShadowSubmission {revision:u64::from(*frame),atlas:owner.atlas_image.clone(),cache:owner.cache_image.clone(),
            atlas_extent:owner.settings.atlas,cache_extent:owner.settings.cache,depth_32_bits:quality.depth_32_bits,
            copy_state:owner.copy_state,layers,regions,receiver})
    })();
    match result {
        Ok(frame)=> {
            if draws.error.take().is_some() || !draws.reported {
                println!("Native local atlas submissions online: {} regions, {} draws",frame.regions.len(),frame.layers.iter().map(|l|l.instances.len()).sum::<usize>());
                draws.reported=true;
            }
            output.0=Some(frame);
        }
        Err(error)=>if draws.error.as_ref()!=Some(&error) {eprintln!("Native local atlas submissions pending: {error}");draws.error=Some(error);},
    }
}

pub(super) fn initialize_native_local_shadow_owner(mut commands:Commands,scene:Res<SceneData>,tome_world:Option<Res<TomeWorld>>,
    view_settings:Res<NativeViewSettings>,mut images:ResMut<Assets<Image>>,
    mut programs:ResMut<NativeProgramCacheSource>) {
    let result=(||->Result<NativeLocalShadowOwner,String>{
        let (_,settings)=original_settings::read_local_shadow_settings()?;
        let quality=scene.shadow_quality.as_ref().ok_or("native local shadows have no selected original quality")?;
        let parameters=scene.environment_render_parameters.as_ref().ok_or("native local shadow environment parameters are absent")?;
        let clamp=scene.shadow_depth_bias_clamp.ok_or("native local shadow depth-bias clamp is absent")?;
        let hardware_bias_point=render_shadow::local_hardware_bias(parameters,0,quality.depth_32_bits,clamp)?;
        let hardware_bias_spot=render_shadow::local_hardware_bias(parameters,2,quality.depth_32_bits,clamp)?;
        let copy_state=programs.0.as_mut().map_err(|e|e.clone())?.definition(b"depthcopy")?
            .ok_or("native depthCopy RPD is unavailable")?.native_render_state();
        let format=match render_shadow::parallel_shadow_depth_format(quality.depth_32_bits)? {
            render_shadow::ParallelShadowDepthFormat::Depth16=>bevy::render::render_resource::TextureFormat::Depth16Unorm,
            render_shadow::ParallelShadowDepthFormat::Depth24Stencil8=>bevy::render::render_resource::TextureFormat::Depth24PlusStencil8,
        };
        let mut image=|extent:[u32;3]|->Result<Handle<Image>,String>{
            Ok(images.add(native_shadow_gpu::shadow_depth_image(bevy::render::render_resource::Extent3d {
                width:extent[0],height:extent[1],depth_or_array_layers:extent[2],
            },format)?))
        };
        let atlas_image=image(settings.atlas)?;let cache_image=image(settings.cache)?;
        let roots=vec![[0,0,settings.atlas[0] as i32-1,settings.atlas[1] as i32-1];settings.atlas[2] as usize];
        let cache_roots=vec![[0,0,settings.cache[0] as i32-1,settings.cache[1] as i32-1];settings.cache[2] as usize];
        let topology=native_task_queue::cpu_topology()?;
        let query_job_count=native_task_queue::local_shadow_query_job_count(&topology);
        let mut precomputed_world_casters=HashMap::new();
        if let Some(tome)=tome_world.as_ref() {
            let view=view_settings.0.as_ref().map_err(Clone::clone)?;
            let queries=native_local_world_query::StaticLightCasterQueries {bytes:&tome.bytes,tree:&tome.tree,data:&tome.query_data,
                gates:&tome.gate_states,world:&scene.world,job_count:query_job_count,
                object_imprecision:view.umbra_distance_imprecision,accurate_threshold:view.umbra_accurate_threshold};
            for (source,light) in scene.placed_lights.iter().enumerate() {
                //881640 copies actor m_lightCulling(+448) to render-light+68.
                //232e50 only precomputes static-position shadow-enabled lights.
                if light.no_shadows || light.culling!=entities::LightCulling::StaticPosition {continue;}
                let kind=match light.kind {entities::LightType::Point|entities::LightType::HalfPoint=>0,entities::LightType::Spot=>2,_=>continue};
                let color=Color::srgb(light.color[0].max(0.),light.color[1].max(0.),light.color[2].max(0.)).to_linear();
                let preset=original_light_preset(light,[color.red,color.green,color.blue]).ok_or("local caster precomputation has no original light preset")?;
                let near=light.shadow_near_plane.unwrap_or(0.1);
                let clips=if kind==2 {vec![render_shadow::unfitted_spot_projection(light.position,light.rotation,
                    authored_light_cosines(light.angular_cutoff,light.angular_falloff).0,near,preset.range)]}
                else {render_shadow::prepare_omni_projections(light.position,near,preset.range,0,None,&mut [[-1.,-1.,1.,1.];6],&mut render_shadow::ProjectionFitScratch::default())?.to_vec()};
                precomputed_world_casters.insert(source,queries.query_light(light.position,&clips)?);
            }
        }
        println!("Native local caster precomputation: {} lights, task-pool jobs={query_job_count}, physical cores={}",precomputed_world_casters.len(),topology.words[1]);
        Ok(NativeLocalShadowOwner {settings,atlas_image,cache_image,atlas:render_shadow::ShadowAtlasAllocator::new(&roots,0),
            cache:native_local_shadow_cache::LocalShadowCache::new(&[cache_roots]),view_flags:0,
            world_query:native_local_world_query::NativeLocalWorldQuery::new(&scene.world),precomputed_world_casters,query_job_count,
            hardware_bias_point,hardware_bias_spot,copy_state})
    })();
    match result {
        Ok(owner)=> {println!("Native local shadow targets allocated: atlas={:?}, cache={:?}; awaiting actual side draws",owner.settings.atlas,owner.settings.cache);commands.insert_resource(owner);},
        Err(error)=>eprintln!("Native local shadow owner unavailable: {error}"),
    }
}

pub(super) fn prepare_native_local_shadow_cpu(
    owner:Option<ResMut<NativeLocalShadowOwner>>,scene:Res<SceneData>,
    lights:Query<(Entity,&NativeLocalLightSource,&Visibility)>,
    cameras:Query<(&Camera,&Transform,&Projection),With<Camera3d>>,
    coarse:Res<native_coarse_depth::NativeCoarseDepthMailbox>,
    frame:Res<bevy::diagnostic::FrameCount>,mut cpu:ResMut<NativeLocalShadowCpu>,
    model_frame:Res<native_shadow_live::NativeModelCasterFrame>,
    model_sources:Res<native_shadow_live::NativeModelShadowSources>,
    completion:Res<native_shadow_scene::NativeLocalShadowCompletion>,
    mut view_controls:ResMut<NativeLocalShadowViewControls>,
    profile:Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span=profile.as_deref().map(|p|p.measure(24));
    cpu.current=None;
    let Some(mut owner)=owner else {return;};
    if let Ok(done)=completion.0.lock() {
        if let Some((_,atlas,cache,entries))=&*done {
            if *atlas==owner.atlas_image.id() && *cache==owner.cache_image.id() {owner.cache.acknowledge_models(entries);}
        }
    }
    owner.cache.apply_view_controls(std::mem::take(&mut view_controls.0));
    let result=(||->Result<Vec<NativeLocalProjectedLight>,String>{
        if owner.view_flags&2!=0 {return Ok(Vec::new());}
        let (camera,transform,projection)=cameras.single().map_err(|e|e.to_string())?;
        let Projection::Custom(custom)=projection else {return Err("native local shadows have no original camera projection".into());};
        let projection=custom.get::<native_camera::NativeGameProjection>().ok_or("native local shadow camera type is absent")?;
        let extent=camera.physical_viewport_size().ok_or("native local camera viewport is absent")?;
        let horizontal_fov=tome::original_render_fov_radians(projection.reference_horizontal_degrees,extent.x,extent.y,1.0)?.0;
        let pose=game_pose_from_view_transform(transform);let camera_position=view_to_game(transform.translation);
        let view=tome::render_query_base_rows(Mat4::IDENTITY.to_cols_array_2d(),pose);
        let matrix=projection.rows();let vp=render_shadow::multiply_rows(matrix,view);
        let (model_frame_number,model_casters)=model_frame.current.as_ref()
            .ok_or_else(||model_frame.error.clone().unwrap_or("native local model collector is unavailable".into()))?;
        if *model_frame_number!=frame.0 {return Err("native local model collector belongs to another frame".into());}
        let mut inverse_projection=[[0.0;4];4];let mut inverse_vp=[[0.0;4];4];
        if !render_shadow::invert_rows(&matrix,&mut inverse_projection) || !render_shadow::invert_rows(&vp,&mut inverse_vp) {
            return Err("native local camera is singular".into());
        }
        let snapshot=coarse.latest()?;
        let pixels=snapshot.as_ref().filter(|v|v.metadata.source_extent==[extent.x,extent.y]).map(|v|v.pixels.as_ref());
        let camera_fit=render_shadow::SpotFitCamera {planes:render_shadow::frustum_planes(vp,false,true),
            view,projection:matrix,inverse_projection,inverse_view_projection:inverse_vp,near:projection.near,coarse:pixels};
        let mut sources=HashMap::new();let mut candidates=Vec::new();
        for (entity,source,visibility) in &lights {
            if *visibility==Visibility::Hidden {continue;}
            let light=scene.placed_lights.get(source.0).ok_or("native local light lost its authored source")?;
            let kind=match light.kind {entities::LightType::Point=>0,entities::LightType::HalfPoint=>1,
                entities::LightType::Spot=>2,_=>continue};
            let color=Color::srgb(light.color[0].max(0.0),light.color[1].max(0.0),light.color[2].max(0.0)).to_linear();
            let preset=original_light_preset(light,[color.red,color.green,color.blue]).ok_or("native local light preset is unresolved")?;
            let quality=if kind!=2 || light.no_shadows {0}else{match light.spot_shadow_quality.as_deref() {
                None|Some("SPOT_SHADOWQUALITY_NORMAL")=>0,Some("SPOT_SHADOWQUALITY_DUALSHADOW")=>1,
                Some("SPOT_SHADOWQUALITY_EXTRA_RESOLUTION")=>2,Some(value)=>return Err(format!("unsupported original spot shadow quality {value}")),
            }};
            let flags=[u8::from(!light.no_shadows)*2|u8::from(light.cast_volumetric_shadows)*0x20
                |u8::from(light.dont_alter_volumetric_dynamic_shadow)*0x40,u8::from(light.scatter_only)];
            sources.insert(source.0,entity);
            candidates.push(render_shadow::LocalShadowCandidate {source:source.0,flags,kind,quality,position:light.position,range:preset.range});
        }
        // Runtime capture maps158 authored lights in strictly increasing
        // declaration order, independently of Bevy entity iteration. Spawned
        // runtime lights require their own retained registry entries later.
        candidates.sort_by_key(|candidate|candidate.source);
        let settings=owner.settings;let additional_view=owner.view_flags&1!=0;
        let quality=scene.shadow_quality.as_ref().ok_or("native local shadow quality is absent")?;
        let (mut requests,_)=render_shadow::size_local_shadow_requests(&candidates,camera_position,horizontal_fov,&settings,
            quality.local_low_resolution_divider,additional_view,0,&mut owner.atlas);
        for request in &mut requests {
            request.side_count=render_shadow::local_shadow_side_count(request.candidate.kind,request.candidate.quality,request.priority);
        }
        let model_inputs:LocalModelInputs=std::array::from_fn(|bucket| {
            let mut records=Vec::new();let mut inputs=Vec::new();
            if requests.is_empty() {return (records,inputs);}
            for caster in model_casters.iter().filter(|caster|caster.dynamic==(bucket!=0)) {
                for &instance in &caster.instances {
                    records.push(render_model::LocalModelCasterRecord {bounds:model_sources.0[caster.source].bounds,
                        camera_clip_mask:caster.camera_clip_mask});
                    inputs.push((caster.source,instance));
                }
            }
            (records,inputs)
        });
        // Fitting and model selection have no atlas/cache side effects. Run the same
        // original numeric helpers on independent light banks, then restore
        // request order before the native allocator and cache are touched.
        let chunk_size=requests.len().div_ceil(owner.query_job_count as usize).max(1);
        let mut projected=bevy::tasks::ComputeTaskPool::get().scope(|scope| {
            for (chunk,requests) in requests.chunks(chunk_size).enumerate() {
                let scene=&*scene;let camera=&camera_fit;let view_flags=owner.view_flags;let model_inputs=&model_inputs;
                scope.spawn(async move {
                    let mut scratch=render_shadow::ProjectionFitScratch::default();
                    (chunk,requests.iter().map(|request| {
                        let light=&scene.placed_lights[request.candidate.source];
                        (project_local_shadow(request,light,view_flags,camera,&mut scratch),
                            collect_local_light_models(request,light,view_flags,&vp,model_inputs))
                    }).collect::<Vec<_>>())
                });
            }
        });
        projected.sort_by_key(|(chunk,_)|*chunk);
        if !cpu.verified_parallel && !requests.is_empty() && pixels.is_some() && env::var_os("DH2_VERIFY_LOCAL_PROJECTION_PARALLEL").is_some() {
            let bits=|result:&Result<LocalProjection,String>|result.as_ref().map(|(clips,crops)|
                clips.iter().flatten().flatten().chain(crops.iter().flatten()).map(|v|v.to_bits()).collect::<Vec<_>>()).map_err(Clone::clone);
            let mut scratch=render_shadow::ProjectionFitScratch::default();
            for (request,result) in requests.iter().zip(projected.iter().flat_map(|(_,results)|results)) {
                let serial=project_local_shadow(request,&scene.placed_lights[request.candidate.source],owner.view_flags,&camera_fit,&mut scratch);
                let serial_models=collect_local_light_models(request,&scene.placed_lights[request.candidate.source],owner.view_flags,&vp,&model_inputs);
                if bits(&serial)!=bits(&result.0) || serial_models!=result.1 {return Err(format!("parallel local preparation differs from serial source helpers for light{}",request.candidate.source));}
            }
            println!("Native local preparation parallel verification: {} projections and caster lists bitwise equal to serial original helpers",requests.len());
            cpu.verified_parallel=true;
        }
        let mut output=Vec::with_capacity(requests.len());
        for (request,(projected,model_casters)) in requests.into_iter().zip(projected.into_iter().flat_map(|(_,results)|results)) {
            let light=&scene.placed_lights[request.candidate.source];
            //333570/332f20 reserve every face before doing any projection.
            //One failed reservation releases the entire light's partial bank.
            let mut allocations=Vec::with_capacity(request.side_count);
            for _ in 0..request.side_count {
                let Some(allocation)=owner.atlas.allocate(request.resolution[0] as u32,request.resolution[1] as u32) else {break;};
                allocations.push(allocation);
            }
            if allocations.len()!=request.side_count {
                for allocation in allocations {owner.atlas.release(allocation.id);}
                continue;
            }
            let (clips,crops)=projected?;
            let id=u16::try_from(request.candidate.source).map_err(|_|"native local light registry exceeds16-bit identities")?;
            let mut bounds=[[f32::from_bits(0x7149f2ca);3],[f32::from_bits(0xf149f2ca);3]];
            let mut sides=Vec::with_capacity(request.side_count);
            let mut receiver_sides=Vec::with_capacity(request.side_count);
            for (side,allocation) in allocations.into_iter().enumerate() {
                receiver_sides.push((allocation,render_shadow::local_shadow_side_atlas(clips[side],allocation,
                    [settings.atlas[0],settings.atlas[1]])));
                let Some(geometry)=render_shadow::local_shadow_side_geometry(clips[side],vp,allocation,
                    [settings.atlas[0],settings.atlas[1]])? else {
                    owner.atlas.release(allocation.id);sides.push(None);continue;
                };
                for axis in 0..3 {
                    if !(bounds[0][axis]<geometry.bounds[0][axis]) {bounds[0][axis]=geometry.bounds[0][axis];}
                    if !(bounds[1][axis]>geometry.bounds[1][axis]) {bounds[1][axis]=geometry.bounds[1][axis];}
                }
                let slot=render_shadow::local_shadow_cache_slot(request.candidate.kind,request.side_count,side);
                let key=native_local_shadow_cache::cache_key(id,slot as u32);
                let cache_source=native_local_shadow_cache::LocalShadowCacheSource {position:light.position,
                    forward:light.rotation[0],up:light.rotation[2],crop:crops[side],width:request.resolution[0] as u32};
                let cached=owner.cache.lookup(0,key,&cache_source,frame.0).cloned();let cache_match=cached.is_some();
                let cached=cached.or_else(||owner.cache.insert(0,key,cache_source,request.resolution[1] as u32,frame.0).cloned());
                sides.push(Some(NativeLocalShadowSide {allocation,geometry,cache_key:key,cache:cached,cache_match}));
            }
            let precomputed=owner.precomputed_world_casters.get(&request.candidate.source);
            let world_casters=sides.iter().enumerate().map(|(side,geometry)| {
                if geometry.is_none() {return Vec::new();}
                let source_side=if request.candidate.kind==2 {0}else{side};
                if let Some(ids)=precomputed.and_then(|faces|faces.get(source_side)).filter(|ids|!ids.is_empty()) {
                    ids.clone()
                } else {
                    //32cea0 uses aggregate light bounds and this side's clip
                    //only when the retained list for that face is empty.
                    owner.world_query.query(bounds,&clips[side]).iter().enumerate().filter_map(|(id,&value)|(value!=0).then_some(id as u32)).collect()
                }
            }).collect();
            // Allocation/cache side effects retain native order, including
            // deferring a failed whole-light model query until this boundary.
            let model_casters=model_casters?;
            let delta:[f32;3]=std::array::from_fn(|i|light.position[i]-camera_position[i]);
            let squared=(delta[1]*delta[1]+delta[0]*delta[0])+delta[2]*delta[2];
            let skip_dynamic=request.priority<=quality.local_partial_dynamic_threshold
                && quality.local_partial_squared_distance<squared;
            let refresh_dynamic=request.candidate.flags[0]&0x40!=0 || !skip_dynamic;
            output.push(NativeLocalProjectedLight {entity:sources[&request.candidate.source],request,clips,crops,sides,receiver_sides,bounds,world_casters,model_casters,refresh_dynamic});
        }
        //34fbf0's captured cache contains the current frame's lookup stamps
        //already. It evicts unused entries after preparation, preserving all
        //current hits; evicting before lookups would destroy cross-frame reuse.
        if !additional_view {owner.cache.evict_before(0,frame.0);}
        Ok(output)
    })();
    match result {
        Ok(lights)=> {
            if cpu.error.take().is_some() || !cpu.reported {println!("Native local shadow CPU projections online: {} lights, {} sides; GPU submission pending",lights.len(),lights.iter().map(|v|v.clips.len()).sum::<usize>());cpu.reported=true;}
            cpu.current=Some((frame.0,lights));
        }
        Err(error)=>if cpu.error.as_ref()!=Some(&error) {eprintln!("Native local shadow CPU projections pending: {error}");cpu.error=Some(error);},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore="requires private native replays of captured sparse portal masks"]
    fn installed_local_portal_raster_evidence() {
        let raw=fs::read_to_string("artifacts/shadow-resume-local-native-remaining-raster-cases.txt").unwrap();
        let mut checked=0;
        for (case,line) in raw.lines().enumerate() {
            let mut words=line.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap());
            let mut next=||words.next().unwrap();
            let source_rect=std::array::from_fn(|_|next() as u8);let source_len=next() as usize;
            let mut source=tome::PortalTileMask::new(source_rect).unwrap();
            source.bits=(0..source_len).map(|_|next()).collect();
            let target_rect=std::array::from_fn(|_|next() as u8);let target_len=next() as usize;
            let mut target=tome::PortalTileMask::new(target_rect).unwrap();
            target.bits=(0..target_len).map(|_|next()).collect();
            let pixels=std::array::from_fn(|_|next() as i32);
            let projected=std::array::from_fn(|_|std::array::from_fn(|_|f32::from_bits(next())));
            let face=std::array::from_fn(|_|std::array::from_fn(|_|f32::from_bits(next())));
            let changed=next()!=0;let expected=(0..target_len).map(|_|next()).collect::<Vec<_>>();
            let actual=target.rasterize_projected_box(&source,pixels,projected,face).unwrap();
            assert_eq!((actual,&target.bits),(changed,&expected),"native captured raster{case}");checked+=1;
        }
        assert!(checked>0);
    }
    #[test]
    #[ignore="requires installed Tome and694 private Steam-verified completed light queries"]
    fn installed_all_local_tome_queries_evidence() {
        let bytes=fs::read("artifacts/eotw_p_lowchaos.tome").unwrap();
        let tree=tome::SpatialTree::parse(&bytes).unwrap();
        let data=tome::DirectTomeQueryData::parse(&bytes,&tree).unwrap();
        let gates=tome::initial_gate_states(&bytes).unwrap();
        let raw=fs::read_to_string("artifacts/shadow-resume-local-all-tome-cases.txt").unwrap();
        let mut failures=0;let mut checked=0;
        for (case,line) in raw.lines().enumerate() {
            let columns=line.split('|').collect::<Vec<_>>();
            let words=columns[0].split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let rows=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(words[r*4+c])));
            let input=tome::OrdinaryDirectQueryInput {camera_point:std::array::from_fn(|i|f32::from_bits(words[16+i])),
                render_rows:rows,cropped_rows:rows,initial_tile_rect:[0,0,8,16],camera_imprecision:[0.;3],object_imprecision:0.,
                minimum_area_metric:0.,global_distance_threshold:70.,object_distance_scale:f32::MIN_POSITIVE,
                winding_flip:tome::query_matrix_winding_flip(rows),gate_states:Some(&gates),output_capacity:tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY};
            let jobs=(0..7).map(|job|tome::ordinary_direct_tome_render_job(&tree,&bytes,&data,&input,job,7)).collect::<Vec<_>>();
            let output=tome::merge_render_query_outputs(&jobs).unwrap().unwrap();
            let actual=output.indices().iter().copied().collect::<HashSet<_>>();
            let expected=columns[2].split_whitespace().map(|v|v.parse::<u32>().unwrap()).collect::<HashSet<_>>();
            if actual!=expected {
                failures+=1;eprintln!("Native query{case}: actual{} original{} extra{:?} missing{:?}",actual.len(),expected.len(),actual.difference(&expected).collect::<Vec<_>>(),expected.difference(&actual).collect::<Vec<_>>());
                for job in &jobs {if let Some(error)=&job.diagnostics.error {eprintln!("Native query{case} job{} error: {error}",job.diagnostics.job_index);}}
                for &id in expected.difference(&actual) {eprintln!("Native query{case} missing{id} point_baked{} bounds{:?}",tome::shadow_caster::native_point_occlusion_visible(&bytes,id,input.camera_point,0.).unwrap(),tome::object_bounds(&bytes,id).unwrap());}
            }
            checked+=1;
        }
        assert_eq!(checked,694);assert_eq!(failures,0,"full native local-light Tome corpus");
    }
    #[test]
    #[ignore="requires installed Tome and Steam-verified private portal capture"]
    fn installed_local_portal_caller_evidence() {
        let bytes=fs::read("artifacts/eotw_p_lowchaos.tome").unwrap();
        let path=env::var("DH2_LOCAL_PORTAL_CASES").unwrap_or_else(|_|"artifacts/shadow-resume-local-native-light73-portals.txt".into());
        let raw=fs::read_to_string(path).unwrap();
        let mut count=0;
        for (case,line) in raw.lines().enumerate() {
            let w=line.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let link=std::array::from_fn(|i|w[3+i]);let record=w[0];let winding=w[2]!=0;
            let direction=tome::LinkQueryDirection::Volume {center:std::array::from_fn(|i|w[7+i] as i32),tolerance:std::array::from_fn(|i|w[11+i] as i32)};
            let matrix=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(w[15+r*4+c])));
            let point=std::array::from_fn(|i|f32::from_bits(w[31+i]));
            let face=tome::admitted_link_portal_face(&bytes,record,link,direction,point,None).unwrap();
            assert_eq!(face,Some(w[1] as u8),"native portal face case{case}");
            let bounds=if let Some(geometry)=tome::packed_link_geometry(link) {tome::packed_link_query_bounds(&bytes,record,geometry).unwrap()}
                else{tome::gated_link_query_bounds(&bytes,record,link).unwrap().unwrap()};
            let (projected,face_vectors)=tome::portal_raster_inputs_from_bounds(bounds,face.unwrap(),matrix,winding).unwrap();
            let actual=projected.iter().flatten().chain(face_vectors.iter().flatten()).map(|v|v.to_bits()).collect::<Vec<_>>();
            assert_eq!(actual,&w[38..66],"native portal corner/face vectors case{case}");
            let rect=std::array::from_fn(|i|w[66+i] as u8);let incoming=tome::PortalTileMask::full(rect).unwrap();
            let planes=std::array::from_fn::<_,6,_>(|r|std::array::from_fn::<_,4,_>(|c|f32::from_bits(w[70+r*4+c])));
            let setup=tome::setup_spatial_record_query(&bytes,record,point,[0.;3],&planes).unwrap();
            let viewport=[i32::from(rect[0])*8,i32::from(rect[1])*4,i32::from(rect[2])*8,i32::from(rect[3])*4];
            let actual=tome::projected_link_portal_transition(&bytes,record,link,face,matrix,winding,&setup.active_planes,viewport,&incoming).unwrap()
                .unwrap_or_else(||panic!("native portal case{case} rejected: face{face:?} bounds{bounds:?} matrix{matrix:?} projected{projected:?} viewport{viewport:?}")).0;
            assert_eq!(actual,std::array::from_fn(|i|w[34+i] as i32),"native portal pixel rectangle case{case}");count+=1;
        }
        assert!(count>0);
    }
    #[test]
    #[ignore="requires installed EOTW Tome/BWM and Steam-verified local caster capture"]
    fn installed_local_light_precomputed_caster_evidence() {
        let bytes=fs::read("artifacts/eotw_p_lowchaos.tome").unwrap();
        let tree=tome::SpatialTree::parse(&bytes).unwrap();
        let data=tome::DirectTomeQueryData::parse(&bytes,&tree).unwrap();
        let gates=tome::initial_gate_states(&bytes).unwrap();
        let bwm=world_mesh::parse(&fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap()).unwrap();
        let raw=fs::read_to_string("artifacts/shadow-resume-local-light-casters.txt").unwrap();
        let native_rows=fs::read_to_string("artifacts/shadow-resume-local-native-query-rows.txt").unwrap().lines().map(|line| {
            let words=line.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            (std::array::from_fn::<_,3,_>(|i|words[16+i]),std::array::from_fn::<_,4,_>(|r|std::array::from_fn::<_,4,_>(|c|f32::from_bits(words[r*4+c]))))
        }).collect::<Vec<_>>();
        let mut matched=0;
        let mut mismatched=0;
        let capture=fs::read_to_string("artifacts/shadow-resume-local-tome-case.txt").unwrap();
        let (input,expected)=capture.split_once('|').unwrap();
        let w=input.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
        let rows=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(w[r*4+c])));
        let position=std::array::from_fn(|i|f32::from_bits(w[16+i]));
        {
        let geometry_capture=fs::read_to_string("artifacts/shadow-resume-local-native-light6-query.txt").unwrap();
        let geometry_words=geometry_capture.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
        let rows=std::array::from_fn(|r|std::array::from_fn(|c|f32::from_bits(geometry_words[r*4+c])));
        let native_cells=fs::read_to_string("artifacts/shadow-resume-local-native-light6-cell-geometry.txt").unwrap();
        for line in native_cells.lines() {
            let bits=line.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let viewport=std::array::from_fn(|i|bits[i] as i32);
            let cropped=tome::crop_query_rows(rows,viewport).unwrap();
            let (planes,bounds)=tome::query_plane_geometry_from_rows(rows,cropped,[0.;3]).unwrap();
            let actual=planes.iter().flatten().chain(bounds.iter().flatten()).map(|v|v.to_bits()).collect::<Vec<_>>();
            assert_eq!(actual,&bits[4..],"native local query geometry viewport{viewport:?}");
        }
        }
        let input=tome::OrdinaryDirectQueryInput {camera_point:position,render_rows:rows,cropped_rows:rows,
            initial_tile_rect:[0,0,8,16],camera_imprecision:[0.;3],object_imprecision:0.,
            minimum_area_metric:0.,global_distance_threshold:70.,object_distance_scale:f32::MIN_POSITIVE,
            winding_flip:tome::query_matrix_winding_flip(rows),gate_states:Some(&gates),output_capacity:tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY};
        let jobs=(0..7).map(|job|tome::ordinary_direct_tome_render_job(&tree,&bytes,&data,&input,job,7)).collect::<Vec<_>>();
        for job in &jobs {eprintln!("Native replay job{} cells{:?}",job.diagnostics.job_index,job.diagnostics.progress.processed_object_cells);}
        let output=tome::merge_render_query_outputs(&jobs).unwrap().unwrap();let mut mask=vec![false;bwm.visibility.len()];
        let native_ids=fs::read_to_string("artifacts/shadow-resume-local-tome-object-indices.txt").unwrap()
            .split_whitespace().map(|v|v.parse::<u32>().unwrap()).collect::<HashSet<_>>();
        let actual_ids=output.indices().iter().copied().collect::<HashSet<_>>();
        eprintln!("Native Tome objects actual{} original{} extra{:?} missing{:?}",actual_ids.len(),native_ids.len(),actual_ids.difference(&native_ids).collect::<Vec<_>>(),native_ids.difference(&actual_ids).collect::<Vec<_>>());
        for &id in output.indices() {for &instance in &bwm.tome_object_instances[id as usize] {mask[instance]=true;}}
        for &instance in &bwm.spatial_query_instances {mask[instance]=tome::render_query_aabb_visible_without_depth(output.state.as_ref(),bwm.visibility[instance].bounds).unwrap();}
        let actual=mask.iter().enumerate().filter_map(|(i,&v)|v.then_some(i)).collect::<Vec<_>>();
        let expected=expected.split_whitespace().map(|v|v.parse::<usize>().unwrap()).collect::<Vec<_>>();
        eprintln!("Exact native light query: actual{} original{}",actual.len(),expected.len());
        for &id in actual.iter().filter(|id|!expected.contains(id)) {
            eprintln!("EXTRA {id} spatial={} record={:?}",bwm.spatial_query_instances.contains(&id),bwm.visibility[id]);
        }
        assert_eq!(actual,expected,"Steam-captured light-query descriptor");
        let caster_queries=native_local_world_query::StaticLightCasterQueries {bytes:&bytes,tree:&tree,data:&data,
            gates:&gates,world:&bwm,job_count:7,object_imprecision:0.,accurate_threshold:70.};
        for (light,line) in raw.lines().enumerate() {
            let (hex,expected)=line.split_once('|').unwrap();let hex=hex.trim();
            let record=(0..hex.len()).step_by(2).map(|at|u8::from_str_radix(&hex[at..at+2],16).unwrap()).collect::<Vec<_>>();
            let word=|at|u32::from_le_bytes(record[at..at+4].try_into().unwrap());let value=|at|f32::from_bits(word(at));
            let kind=word(0x5c);if kind>2 || record[0]&2==0 || word(0x68)!=0 {continue;}
            let position=std::array::from_fn(|axis|value(0x28+axis*4));
            let clips=if kind==2 {vec![render_shadow::unfitted_spot_projection(position,
                std::array::from_fn(|r|std::array::from_fn(|c|value(4+r*12+c*4))),value(0x90),value(0x84),value(0x80))]}
            else {let mut crops=[[-1.,-1.,1.,1.];6];render_shadow::prepare_omni_projections(position,value(0x84),value(0x80),0,None,&mut crops,&mut render_shadow::ProjectionFitScratch::default()).unwrap().to_vec()};
            let caster_lists=caster_queries.query_light(position,&clips).unwrap();
            for (side,(clip,expected)) in clips.iter().zip(expected.split(';')).enumerate() {
                let captured=native_rows.iter().filter(|(p,_)|*p==position.map(f32::to_bits)).map(|(_,rows)|rows).collect::<Vec<_>>();
                if !captured.is_empty() {
                    assert!(captured.iter().any(|rows|clip.map(|r|r.map(f32::to_bits))==rows.map(|r|r.map(f32::to_bits))),"native map-load light{light} side{side} projection");
                }
                let expected=expected.split_whitespace().map(|v|v.parse::<usize>().unwrap()).collect::<Vec<_>>();
                let actual=caster_lists[side].iter().map(|&id|id as usize).collect::<Vec<_>>();
                let extra=actual.iter().filter(|id|!expected.contains(id)).count();let missing=expected.iter().filter(|id|!actual.contains(id)).count();
                eprintln!("Native local light{light} side{side}: actual{} original{} extra{extra} missing{missing}",actual.len(),expected.len());
                if actual!=expected {
                    mismatched+=1;
                    eprintln!("DIFFERENCE light{light} side{side}: extra{:?} missing{:?}",actual.iter().filter(|id|!expected.contains(id)).collect::<Vec<_>>(),expected.iter().filter(|id|!actual.contains(id)).collect::<Vec<_>>());
                }
                matched+=1;
            }
        }
        assert!(matched>=100);
        assert_eq!(mismatched,0,"{matched} native precomputed local faces checked");
    }
    #[test]
    #[ignore="requires installed declarations, local EOTW entities and Steam-verified local prepare capture"]
    fn installed_local_shadow_source_and_gather_order_evidence() {
        let catalog=archive::Catalog::open(std::path::Path::new(r"D:\Programs\Steam\steamapps\common\Dishonored2\base")).unwrap();
        let document=entities::parse(&fs::read_to_string("artifacts/eotw_p_lowchaos.entities").unwrap()).unwrap();
        let lights=entities::placed_lights_with_inheritance(&document,|path|Ok(catalog.read(path).unwrap()
            .map(|bytes|entities::native_declaration_text(&bytes).into_owned()))).unwrap();
        let raw=fs::read_to_string("artifacts/shadow-resume-local-original-prepare.json").unwrap();
        assert!(raw.contains("\"steam_launch_verified\": true"));
        let input=raw.split("\"prepared_lights\"").next().unwrap();
        let mut ordered=Vec::new();let mut unmatched=0;let mut mismatches=0;
        for part in input.split("\"record_hex\": \"").skip(1) {
            let hex=part.split('"').next().unwrap();let record=(0..hex.len()).step_by(2).map(|at|u8::from_str_radix(&hex[at..at+2],16).unwrap()).collect::<Vec<_>>();
            let word=|at|u32::from_le_bytes(record[at..at+4].try_into().unwrap());
            let kind=word(0x5c);let position:[f32;3]=std::array::from_fn(|i|f32::from_bits(word(0x28+i*4)));
            let hash=word(0x58);
            let matches=lights.iter().enumerate().filter(|(_,light)| {
                let actual_kind=match light.kind {entities::LightType::Point=>0,entities::LightType::HalfPoint=>1,entities::LightType::Spot=>2,entities::LightType::Area=>3,entities::LightType::Parallel=>4};
                actual_kind==kind && light.position.iter().zip(position).all(|(&a,b)|(a-b).abs()<0.001)
            }).collect::<Vec<_>>();
            let selected=matches.iter().find(|(_,light)|light.definition_name_hash==hash).or_else(||(matches.len()==1).then(||&matches[0]));
            if let Some((index,light))=selected {
                ordered.push(*index);
                let flags=[u8::from(!light.no_shadows && kind!=3)*2|u8::from(light.cast_volumetric_shadows)*32|u8::from(light.dont_alter_volumetric_dynamic_shadow)*64,u8::from(light.scatter_only)];
                if record[0]&0x62!=flags[0] || record[1]&1!=flags[1] {mismatches+=1;eprintln!("FLAGS {} source={flags:?} original={:02x?}",light.id,&record[..2]);}
                let culling=u32::from(light.culling==entities::LightCulling::DynamicPosition);
                if word(0x68)!=culling {mismatches+=1;eprintln!("CULLING {} source{culling} original{}",light.id,word(0x68));}
            } else {unmatched+=1;eprintln!("UNMATCHED kind{kind} position{position:?} name_hash{hash:08x} candidates{}",matches.len());}
        }
        eprintln!("Local capture: {} mapped lights, {unmatched} unmatched; consumed flag mismatches{mismatches}; declaration order monotonic={}; indices={ordered:?}",ordered.len(),ordered.windows(2).all(|w|w[0]<w[1]));
        assert!(ordered.len()>100);assert_eq!(mismatches,0);
        assert!(ordered.windows(2).all(|w|w[0]<w[1]),"native authored light gather order changed");
    }
}
