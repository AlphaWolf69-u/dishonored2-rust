//! Optional numerical samples of the actual submitted sun/local atlases.
//! Input: DH2_TRACE_SHADOW_RECEIVERS names a text file of game-space XYZ rows.
//! Output: .bin contains vec4 results, with (14 + light capacity) rows per point:
//! selection, five sun visibility/UV/depth rows, five stored-depth rows, then
//! each local light's visibility with offset flag banks 0/1/2/15, then screen
//! visibility/packed channels and main-depth/projected-point comparison and HDR color.
use super::*;
use bevy::render::render_resource::*;

#[derive(Resource, Default)]
struct Trace { tick:u32, staging:Option<Buffer>, mapped:bool, path:Option<std::path::PathBuf> }

pub(super) fn install(app:&mut bevy::app::SubApp) {
    app.init_resource::<Trace>()
        .add_systems(Core3d, sample.after(Core3dSystems::MainPass).before(Core3dSystems::PostProcess))
        .add_systems(Render, map.in_set(RenderSystems::Cleanup));
}

fn sample(mut trace:ResMut<Trace>,local:Res<LocalGpu>,sun:Res<NativeShadowSceneGpu>,local_frame:Res<NativeLocalShadowSceneFrame>,
    frame:Res<NativeShadowSceneFrame>,readiness:Res<NativeShadowCaptureReadiness>,
    sampling:Option<Res<crate::native_shadow_sampling::NativeLocalSamplingBuffer>>,
    buffers:Res<RenderAssets<GpuShaderBuffer>>,device:Res<RenderDevice>,mut context:RenderContext,
    screen:Option<Res<crate::native_screen_shadow_scene::NativeScreenShadowFrame>>,
    images:Res<RenderAssets<GpuImage>>,
    views:Query<(&bevy::render::view::ViewDepthTexture,&bevy::render::view::ExtractedView,&bevy::render::view::ViewTarget),With<bevy::render::camera::ExtractedCamera>>) {
    let Some(path)=std::env::var_os("DH2_TRACE_SHADOW_RECEIVERS") else {return;};
    if trace.staging.is_some() || !local.receiver_reported || readiness.0.load(std::sync::atomic::Ordering::Acquire)<30 {return;}
    trace.tick+=1;if trace.tick<120 {return;}
    let NativeShadowSceneState::Ready {target_image,receiver,..}=&frame.state else {return;};
    if !sun.completed_revision(frame.revision) {return;}
    let (Some(target),Some((atlas,_)),Some(sampling))=(sun.targets.get(&target_image.id()),local.arrays.as_ref(),sampling) else {return;};
    let Some(local_buffer)=buffers.get(sampling.buffer.id()) else {return;};
    let Some(screen)=screen.as_ref().and_then(|s|images.get(s.output.id())) else {return;};
    let Ok((depth,view,color))=views.single() else {return;};
    let result=(||->Result<(),String>{
        let path=std::path::PathBuf::from(path);
        let values=std::fs::read_to_string(&path).map_err(|e|e.to_string())?
            .split_whitespace().map(|v|v.parse::<f32>().map_err(|e|e.to_string())).collect::<Result<Vec<_>,_>>()?;
        if values.len()%3!=0 {return Err("receiver trace needs XYZ triples".into());}
        let points=values.chunks_exact(3).map(|p|[p[0],p[1],p[2]]).collect::<Vec<_>>();
        if points.is_empty() || points.len()>4096 || points.iter().flatten().any(|v|!v.is_finite()) {return Err("invalid receiver trace points".into());}
        let uniform=crate::native_shadow_sampling::NativeParallelSamplingUniform::from_owner_receiver(receiver)?;
        let parallel=ShaderBuffer::from(uniform);
        let parallel=device.create_buffer_with_data(&BufferInitDescriptor {label:Some("shadow trace sun"),contents:parallel.data.as_ref().unwrap(),usage:BufferUsages::STORAGE});
        let point_bytes=points.iter().flat_map(|p|[p[0],p[1],p[2],1.0]).flat_map(f32::to_le_bytes).collect::<Vec<_>>();
        let input=device.create_buffer_with_data(&BufferInitDescriptor {label:None,contents:&point_bytes,usage:BufferUsages::STORAGE});
        let count=points.len() as u32*(14+sampling.light_capacity);let size=u64::from(count)*16;
        let clip=view.clip_from_world.unwrap_or_else(||view.clip_from_view*view.world_from_view.to_matrix().inverse());
        let clip=device.create_buffer_with_data(&BufferInitDescriptor {label:None,contents:&clip.to_cols_array().into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>(),usage:BufferUsages::UNIFORM});
        let depth_view=depth.texture.create_view(&TextureViewDescriptor {aspect:TextureAspect::DepthOnly,..Default::default()});
        let output=device.create_buffer(&BufferDescriptor {label:None,size,usage:BufferUsages::STORAGE|BufferUsages::COPY_SRC,mapped_at_creation:false});
        let staging=device.create_buffer(&BufferDescriptor {label:None,size,usage:BufferUsages::COPY_DST|BufferUsages::MAP_READ,mapped_at_creation:false});
        let strip=|s:&str|s.lines().filter(|l|!l.starts_with('#')).collect::<Vec<_>>().join("\n").replace("#{MATERIAL_BIND_GROUP}","0");
        let parallel_source=include_str!("../../../assets/shaders/dh2_parallel_receiver.wgsl").split("fn native_parallel_directional_index").next().unwrap();
        let source=format!("{}\n{}\n{}\n{}",include_str!("../../../assets/shaders/dh2_shadow.wgsl"),strip(parallel_source),strip(include_str!("../../../assets/shaders/dh2_local_receiver.wgsl")),r#"
            @group(0) @binding(129) var<storage,read> points:array<vec4<f32>>;
            @group(0) @binding(130) var<storage,read_write> results:array<vec4<f32>>;
            @group(0) @binding(131) var screen:texture_2d<f32>;
            @group(0) @binding(132) var depth:texture_depth_2d;
            @group(0) @binding(133) var<uniform> clip:mat4x4<f32>;
            @group(0) @binding(134) var color:texture_2d<f32>;
            @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
                if id.x>=arrayLength(&results) {return;}
                let stride=14u+native_local[1];let p=points[id.x/stride].xyz;let lane=id.x%stride;
                if lane>=11u+native_local[1] {
                    let c=clip*vec4(p.x,p.z,-p.y,1.0);let ndc=c.xyz/c.w;
                    let xy=vec2<i32>((ndc.xy*vec2(0.5,-0.5)+vec2(0.5))*vec2<f32>(textureDimensions(screen)));
                    if lane==11u+native_local[1] {results[id.x]=textureLoad(screen,xy,0);}
                    else if lane==12u+native_local[1] {let z=textureLoad(depth,xy,0);results[id.x]=vec4(z,ndc.z,z-ndc.z,f32(xy.x));}
                    else {results[id.x]=textureLoad(color,xy,0);}
                    return;
                }
                if lane==0u {results[id.x]=vec4(f32(native_parallel_shadow_slice(p,native_parallel.view_depth_row,native_parallel.cascade_fars)),
                    -dot(vec4(p,1.0),native_parallel.view_depth_row),f32(native_parallel.config.x),0.0);return;}
                if lane<=10u {
                    let side=(lane-1u)%5u;let options=native_parallel.config.z;
                    let c=native_shadow_offset(native_parallel.world_to_shadow[side],p,vec3(0.0),native_parallel.sun_direction.xyz,true,
                        (options&1u)!=0u,(options&2u)!=0u,(options&4u)!=0u,(options&8u)!=0u,native_parallel.bias.x,native_parallel.bias.y,native_parallel.bias.z);
                    if lane<=5u {let v=native_sample_directional_shadow(native_parallel_depth,native_parallel_comparison,c,i32(side),native_parallel.shadow_size.xy,native_parallel.quality.x,native_parallel.quality.y-1);
                        results[id.x]=vec4(v,c.xyz);}
                    else {let xy=vec2<i32>(c.xy*native_parallel.shadow_size.xy);
                        results[id.x]=vec4(textureLoad(native_parallel_depth,xy,i32(side),0),c.z,0.0,0.0);}
                    return;
                }
                let light=lane-11u;
                if native_local[16u+16u*light+1u]==0xffffffffu {results[id.x]=vec4(-1.0);return;}
                let host=vec3(p.x,p.z,-p.y);let n=vec3(0.0,1.0,0.0);
                results[id.x]=vec4(native_local_visibility(light,host,n,vec4<u32>(0u,0u,2u,2u)),
                    native_local_visibility(light,host,n,vec4<u32>(0u,1u,2u,2u)),
                    native_local_visibility(light,host,n,vec4<u32>(0u,2u,2u,2u)),
                    native_local_visibility(light,host,n,vec4<u32>(0u,15u,2u,2u)));
            }
        "#);
        let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:Some("numerical shadow receiver trace"),source:ShaderSource::Wgsl(source.into())});
        let pipeline=device.wgpu_device().create_compute_pipeline(&RawComputePipelineDescriptor {label:None,layout:None,module:&shader,entry_point:Some("main"),compilation_options:Default::default(),cache:None});
        let sampler=crate::native_shadow_gpu::native_shadow_comparison_sampler(&device);
        let group=device.wgpu_device().create_bind_group(&BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[
            BindGroupEntry {binding:120,resource:parallel.as_entire_binding()},BindGroupEntry {binding:121,resource:BindingResource::TextureView(&target.array.sampled)},BindGroupEntry {binding:122,resource:BindingResource::Sampler(&sampler)},
            BindGroupEntry {binding:126,resource:local_buffer.buffer.as_entire_binding()},BindGroupEntry {binding:127,resource:BindingResource::TextureView(&atlas.sampled)},BindGroupEntry {binding:128,resource:BindingResource::Sampler(&sampler)},
            BindGroupEntry {binding:129,resource:input.as_entire_binding()},BindGroupEntry {binding:130,resource:output.as_entire_binding()},
            BindGroupEntry {binding:131,resource:BindingResource::TextureView(&screen.texture_view)},BindGroupEntry {binding:132,resource:BindingResource::TextureView(&depth_view)},BindGroupEntry {binding:133,resource:clip.as_entire_binding()},BindGroupEntry {binding:134,resource:BindingResource::TextureView(color.main_texture_view())}]});
        let encoder=context.command_encoder();
        {let mut pass=encoder.begin_compute_pass(&ComputePassDescriptor::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups(count.div_ceil(64),1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&staging,0,size);
        std::fs::write(format!("{}.meta.txt",path.display()),format!("points={} stride={}\n{receiver:#?}",points.len(),14+sampling.light_capacity)).map_err(|e|e.to_string())?;
        if let Some(local_frame)=&local_frame.0 {
            std::fs::write(format!("{}.local.txt",path.display()),format!("{:#?}",local_frame.receiver)).map_err(|e|e.to_string())?;
        }
        trace.path=Some(path);trace.staging=Some(staging);Ok(())
    })();
    if let Err(error)=result {bevy::log::error!("numerical shadow receiver trace: {error}");trace.tick=0;}
}

fn map(mut trace:ResMut<Trace>) {
    if trace.mapped {return;}
    let (Some(staging),Some(path))=(trace.staging.clone(),trace.path.clone()) else {return;};
    trace.mapped=true;let callback=staging.clone();
    staging.slice(..).map_async(MapMode::Read,move|result| {
        if let Err(error)=result {bevy::log::error!("shadow receiver trace mapping: {error}");return;}
        let data=callback.slice(..).get_mapped_range();
        if let Err(error)=std::fs::write(format!("{}.bin",path.display()),&data) {bevy::log::error!("shadow receiver trace write: {error}");}
        drop(data);callback.unmap();
    });
}
