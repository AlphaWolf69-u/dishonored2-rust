//! Live screen-shadow owner. The forward renderer first establishes its exact
//! opaque/alpha depth, then computes native visibility and shades that same
//! geometry again before transparency. Both submissions use the same extracted
//! meshes, transforms, alpha clipping and view; no approximate depth prepass.
use bevy::{prelude::*,core_pipeline::{Core3d,core_3d::{main_opaque_pass_3d,main_transparent_pass_3d}},
    render::{extract_resource::{ExtractResource,ExtractResourcePlugin},render_asset::RenderAssets,
        render_resource::*,renderer::{RenderContext,RenderDevice,RenderQueue},
        texture::GpuImage,storage::GpuShaderBuffer,view::{ViewDepthTexture,ExtractedView},RenderApp}};
use crate::{native_screen_shadows::*,native_shadow_scene::*,native_shadow_sampling::NativeParallelSamplingBuffer};

#[derive(Resource,Clone,ExtractResource)]
pub struct NativeScreenShadowFrame {
    pub output:Handle<Image>,
    pub cloud:Handle<Image>,
    pub current:Option<NativeScreenShadowView>,
    pub produced:std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Clone)]
pub struct NativeScreenShadowView {
    pub revision:u64,
    pub parameters:NativeScreenShadowUniform,
    pub producer_state:[u32;9],
    pub blur_state:[u32;9],
}

pub fn visibility_image(width:u32,height:u32)->Image {
    let mut image=Image::new_uninit(Extent3d {width,height,depth_or_array_layers:1},
        TextureDimension::D2,TextureFormat::Rgba8Unorm,Default::default());
    image.texture_descriptor.usage=TextureUsages::TEXTURE_BINDING|TextureUsages::RENDER_ATTACHMENT|TextureUsages::COPY_SRC;
    image
}

pub struct NativeScreenShadowScenePlugin;
impl Plugin for NativeScreenShadowScenePlugin {
    fn build(&self,app:&mut App) {
        app.add_plugins(ExtractResourcePlugin::<NativeScreenShadowFrame>::default())
            .add_systems(Update,capture_screen_visibility);
        let Some(render)=app.get_sub_app_mut(RenderApp) else {return;};
        render.init_resource::<ScreenGpu>().add_systems(Core3d,(
            clear_view_stencil.before(main_opaque_pass_3d),
            render_screen_shadow.after(main_opaque_pass_3d).before(shade_with_screen_shadow),
            shade_with_screen_shadow.before(main_transparent_pass_3d),
        ));
    }
}

#[derive(Resource,Default)]
struct ScreenGpu {cached:Option<ScreenTargets>,reported:bool}
struct ScreenTargets {
    extent:[u32;2],depth_id:TextureId,output_id:TextureId,cloud_id:TextureId,atlas_id:TextureId,
    state:([u32;9],[u32;9]),
    producer:[NativeScreenShadowPipelines;2],blur:[NativeDwaoBlurPipeline;2],
    parameters:[Buffer;2],bindings:[BindGroup;2],blur_bindings:[BindGroup;2],
    blur_parameters:[BindGroup;2],
    // Keep the intermediate texture and actual read-only view alive.
    _intermediate:Texture,intermediate:TextureView,
}

fn clear_view_stencil(
    frame:Option<Res<NativeScreenShadowFrame>>,
    views:Query<&ViewDepthTexture,With<ExtractedView>>,mut context:RenderContext,
) {
    if frame.is_none() {return;}
    let Ok(depth)=views.single() else {return;};
    if !depth.texture.format().has_stencil_aspect() {return;}
    // Native view depth begins with stencil zero; modelDepthHack owners mark
    // bit0 when their actual geometry is submitted. The rewrite currently has
    // no first-person hand draw. Never synthesize a stencil-1 silhouette.
    let view=depth.texture.create_view(&TextureViewDescriptor::default());
    let _pass=context.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label:Some("native view stencil initialization"),color_attachments:&[],
        depth_stencil_attachment:Some(RenderPassDepthStencilAttachment {view:&view,depth_ops:None,
            stencil_ops:Some(Operations {load:LoadOp::Clear(0),store:StoreOp::Store})}),
        timestamp_writes:None,occlusion_query_set:None,multiview_mask:None,
    });
}

fn typed_buffer(device:&RenderDevice,value:impl Into<bevy::render::storage::ShaderBuffer>)->Buffer {
    let bytes=value.into().data.expect("typed native screen transport");
    device.create_buffer_with_data(&BufferInitDescriptor {label:Some("native screen owner parameters"),
        contents:&bytes,usage:BufferUsages::UNIFORM|BufferUsages::COPY_DST})
}

impl ScreenTargets {
    fn new(device:&RenderDevice,depth:&ViewDepthTexture,output:&GpuImage,cloud:&GpuImage,
        atlas:&GpuImage,parallel:&Buffer,view:&NativeScreenShadowView)->Result<Self,String> {
        let extent=[output.texture.width(),output.texture.height()];
        if depth.texture.sample_count()!=1 || extent!=[depth.texture.width(),depth.texture.height()]
            || output.texture.format()!=TextureFormat::Rgba8Unorm {
            return Err("native screen shadows require matching single-sample current-view depth and RGBA8 output".into());
        }
        let states=screen_producer_states(view.producer_state,TextureFormat::Rgba8Unorm,depth.texture.format())?;
        let producer=states.map(|state|NativeScreenShadowPipelines::new(device,state.color,Some(state.depth),
            ColorTargetState {format:TextureFormat::Rgba16Float,blend:None,write_mask:ColorWrites::ALL}));
        let blur_depth=crate::native_shadow_gpu::native_depth_stencil_gpu(&view.blur_state,depth.texture.format())?;
        let blur=std::array::from_fn(|i|NativeDwaoBlurPipeline::new(device,&producer[0],ColorTargetState {
            format:if i==0 {TextureFormat::Rgba16Float}else{TextureFormat::Rgba8Unorm},blend:None,write_mask:ColorWrites::ALL,
        },blur_depth.clone()));
        // FUN_14032c7f0 allocates a 0x54a intermediate (DXGI10 RGBA16F),
        // while the actual ssaoScreenShadows output is DXGI28 RGBA8.
        let intermediate=device.create_texture(&TextureDescriptor {label:Some("native screen shadow blur intermediate"),
            size:Extent3d {width:extent[0],height:extent[1],depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:TextureDimension::D2,format:TextureFormat::Rgba16Float,
            usage:TextureUsages::TEXTURE_BINDING|TextureUsages::RENDER_ATTACHMENT,view_formats:&[]});
        let intermediate_view=intermediate.create_view(&TextureViewDescriptor::default());
        let depth_view=depth.texture.create_view(&TextureViewDescriptor {aspect:TextureAspect::DepthOnly,..Default::default()});
        let point=device.create_sampler(&SamplerDescriptor {label:Some("native pointClamp"),
            address_mode_u:AddressMode::ClampToEdge,address_mode_v:AddressMode::ClampToEdge,..Default::default()});
        let comparison=crate::native_shadow_gpu::native_shadow_comparison_sampler(device);
        let cloud_sampler=device.create_sampler(&SamplerDescriptor {label:Some("native cloud bilinearWrap"),
            mag_filter:FilterMode::Linear,min_filter:FilterMode::Linear,..Default::default()});
        let parameters=std::array::from_fn(|_|typed_buffer(device,view.parameters.clone()));
        let bind=|index:usize,source:&TextureView|producer[0].bind(device,parallel,&parameters[index],&depth_view,&point,
            &atlas.texture_view,&comparison,&cloud.texture_view,&cloud_sampler,source);
        // Visibility and pack do not read binding8. Bind a distinct real image
        // to avoid a read/write alias with the destination attachment.
        let bindings=std::array::from_fn(|i|bind(i,&intermediate_view));
        let blur_bindings=[bind(1,&output.texture_view),bind(1,&intermediate_view)];
        // Executable 32cd21/32cdcd: threshold=.1, radius=1, X then Y.
        // The centre-weight parameter is not read by PostFX_DWAOBlur.
        let blur_parameters=std::array::from_fn(|i| {
            let params=NativeDwaoBlurUniform::from_owner(extent.map(|x|x as f32),
                if i==0 {[1.0,0.0]}else{[0.0,1.0]},1.0,0.1,0.0).expect("nonzero view extent");
            blur[i].bind(device,&typed_buffer(device,params))
        });
        Ok(Self {extent,depth_id:depth.texture.id(),output_id:output.texture.id(),cloud_id:cloud.texture.id(),atlas_id:atlas.texture.id(),
            state:(view.producer_state,view.blur_state),producer,blur,parameters,bindings,blur_bindings,blur_parameters,
            _intermediate:intermediate,intermediate:intermediate_view})
    }
}

fn render_screen_shadow(
    frame:Option<Res<NativeScreenShadowFrame>>,atlas_frame:Res<NativeShadowSceneFrame>,
    atlas_gpu:Res<NativeShadowSceneGpu>,sampling:Option<Res<NativeParallelSamplingBuffer>>,
    buffers:Res<RenderAssets<GpuShaderBuffer>>,images:Res<RenderAssets<GpuImage>>,
    views:Query<(&ViewDepthTexture,&ExtractedView,&bevy::render::camera::ExtractedCamera)>,
    device:Res<RenderDevice>,queue:Res<RenderQueue>,mut gpu:ResMut<ScreenGpu>,mut context:RenderContext,
    mut last_error:Local<Option<String>>,
) {
    let Some(frame)=frame else {return;};
    let result=(||->Result<(),String>{
        let view=frame.current.as_ref().ok_or("native screen CPU frame pending")?;
        if view.revision!=atlas_frame.revision || !atlas_gpu.completed_revision(view.revision) {
            return Err("native screen pass awaits the same-frame atlas submission".into());
        }
        let NativeShadowSceneState::Ready {target_image,..}=&atlas_frame.state else {return Err("native atlas is unavailable".into());};
        let (depth,_,camera)=views.single().map_err(|error|error.to_string())?;
        if camera.physical_viewport_size!=camera.physical_target_size || camera.viewport.as_ref()
            .is_some_and(|v|v.physical_position!=UVec2::ZERO) {return Err("native screen sub-view owner is unresolved".into());}
        let output=images.get(frame.output.id()).ok_or("native screen target pending GPU upload")?;
        let cloud=images.get(frame.cloud.id()).ok_or("native cloud input pending GPU upload")?;
        let atlas=images.get(target_image.id()).ok_or("native atlas pending GPU upload")?;
        let parallel=sampling.as_ref().and_then(|s|buffers.get(s.buffer.id())).ok_or("native parallel parameters pending GPU upload")?;
        if gpu.cached.as_ref().is_none_or(|old|old.depth_id!=depth.texture.id()||old.output_id!=output.texture.id()
            ||old.cloud_id!=cloud.texture.id()||old.atlas_id!=atlas.texture.id()||old.state!=(view.producer_state,view.blur_state)) {
            gpu.cached=Some(ScreenTargets::new(&device,depth,output,cloud,atlas,&parallel.buffer,view)?);
        }
        let cached=gpu.cached.as_ref().unwrap();
        view.parameters.validate()?;
        let mut cloud=view.parameters.cloud_density;
        cloud.w=view.parameters.options.z as f32;
        // This trailing shared transport extends the atlas prefix without
        // duplicating cloud parameters per material. Queue writes precede this
        // frame's GPU submission, including both opaque geometry submissions.
        let tail=bevy::render::storage::ShaderBuffer::from([
            view.parameters.cloud_motion,cloud,view.parameters.camera_time]);
        queue.write_buffer(&parallel.buffer,432,&tail.data.unwrap());
        for (i,buffer) in cached.parameters.iter().enumerate() {
            let mut value=view.parameters.clone();
            // Native second stencil draw restores modelDepthHack=1 and step=.025.
            if i==1 {value.depth_scale_step_hack.w=1.0;value.depth_scale_step_hack.z=0.025;}
            queue.write_buffer(buffer,0,&bevy::render::storage::ShaderBuffer::from(value).data.unwrap());
        }
        let encoder=context.command_encoder();
        cached.producer[0].pack_depth(encoder,&cached.bindings[1],&output.texture_view);
        let attachment=depth.texture.create_view(&TextureViewDescriptor::default());
        let viewport=[0.0,0.0,cached.extent[0] as f32,cached.extent[1] as f32,0.0,1.0];
        let load=Operations {load:LoadOp::Load,store:StoreOp::Store};
        for i in 0..2 {cached.producer[i].draw(encoder,false,&cached.bindings[i],&output.texture_view,
            Some(&attachment),1,load,viewport);}
        cached.blur[0].draw(encoder,&cached.blur_bindings[0],&cached.blur_parameters[0],&cached.intermediate,
            &attachment,Operations {load:LoadOp::Clear(Default::default()),store:StoreOp::Store},viewport);
        cached.blur[1].draw(encoder,&cached.blur_bindings[1],&cached.blur_parameters[1],&output.texture_view,&attachment,load,viewport);
        Ok(())
    })();
    match result {
        Ok(())=> {
            frame.produced.fetch_add(1,std::sync::atomic::Ordering::Release);
            if !gpu.reported {println!("Native screen shadow producer and X/Y blur online");gpu.reported=true;}*last_error=None;
        }
        Err(error)=> {
            // A failed screen pass must not let final lighting sample an old view.
            if let Some(buffer)=sampling.as_ref().and_then(|s|buffers.get(s.buffer.id())) {
                let neutral=bevy::render::storage::ShaderBuffer::from(crate::native_shadow_sampling::NativeParallelSamplingUniform::default());
                queue.write_buffer(&buffer.buffer,0,&neutral.data.unwrap());
            }
            if last_error.as_ref()!=Some(&error) {warn!("Native screen shadows pending: {error}");*last_error=Some(error);}
        }
    }
}

fn capture_screen_visibility(
    mut commands:Commands,frame:Option<Res<NativeScreenShadowFrame>>,images:Res<Assets<Image>>,
    mut requested:Local<bool>,
) {
    if *requested {return;}
    let Ok(path)=std::env::var("DH2_NATIVE_SHADOW_SCREEN_SNAPSHOT") else {return;};
    let Some(frame)=frame else {return;};
    if frame.produced.load(std::sync::atomic::Ordering::Acquire)<30 {return;}
    let Some(image)=images.get(&frame.output) else {return;};
    let extent=image.texture_descriptor.size;
    *requested=true;
    commands.spawn(bevy::render::gpu_readback::Readback::texture(frame.output.clone())).observe(
        move |event:On<bevy::render::gpu_readback::ReadbackComplete>,mut commands:Commands| {
            let pitch=(extent.width*4).div_ceil(256)*256;
            let mut values=Vec::with_capacity((extent.width*extent.height) as usize);
            for row in event.data.chunks(pitch as usize).take(extent.height as usize) {
                values.extend(row[..extent.width as usize*4].chunks_exact(4).map(|pixel|pixel[1]));
            }
            let min=values.iter().min();let max=values.iter().max();
            let shadowed=values.iter().filter(|&&v|v<255).count();
            println!("Native screen visibility readback: {}x{}, range={min:?}..{max:?}, shadowed={shadowed}/{}",extent.width,extent.height,values.len());
            let result=(||->Result<(),Box<dyn std::error::Error>> {
                let mut encoder=png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path)?),extent.width,extent.height);
                encoder.set_color(png::ColorType::Grayscale);encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header()?.write_image_data(&values)?;Ok(())
            })();
            if let Err(error)=result {error!("native screen diagnostic capture failed: {error}");}
            commands.entity(event.entity).despawn();
        });
}

// A distinct system identity orders the final opaque lighting after screen
// production. The already extracted phase supplies exactly the same geometry,
// depth values, alpha tests and transforms as the first submission.
fn shade_with_screen_shadow(
    world:&World,
    view:bevy::render::renderer::ViewQuery<(
        &bevy::render::camera::ExtractedCamera,&ExtractedView,&bevy::render::view::ViewTarget,&ViewDepthTexture,
        Option<&bevy::core_pipeline::skybox::SkyboxPipelineId>,Option<&bevy::core_pipeline::skybox::SkyboxBindGroup>,
        &bevy::render::view::ViewUniformOffset,Option<&bevy::camera::MainPassResolutionOverride>)>,
    opaque:Res<bevy::render::render_phase::ViewBinnedRenderPhases<bevy::core_pipeline::core_3d::Opaque3d>>,
    alpha:Res<bevy::render::render_phase::ViewBinnedRenderPhases<bevy::core_pipeline::core_3d::AlphaMask3d>>,
    pipelines:Res<PipelineCache>,context:RenderContext,
) {
    if !world.contains_resource::<NativeScreenShadowFrame>() {return;}
    main_opaque_pass_3d(world,view,opaque,alpha,pipelines,context);
}
