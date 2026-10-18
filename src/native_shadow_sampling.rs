//! Receiver transport for the original directional depth-shadow algorithm.
//!
//! Original row vectors are stored as WGSL columns (the shader dots each row).
//! This is a stable shared ShaderBuffer, not one buffer per mesh/material/frame.
//! It transports SampleDirectionalShadowMap(ComputeShadowOffset(...)); the
//! atmosphere/cloud attenuation path remains separate and is not substituted.

use bevy::{
    app::App,
    asset::Handle,
    ecs::prelude::Resource,
    image::Image,
    math::{IVec4, Mat4, UVec4, Vec4},
    render::{
        extract_resource::ExtractResource, render_resource::ShaderType, storage::ShaderBuffer,
    },
    shader::Shader,
};

const SHADOW_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("1d67e2c0-a515-4bfe-9b55-276e82430c06");
const RECEIVER_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("8351debf-491b-4d7d-ad83-b03a15260af9");
const LOCAL_RECEIVER_SHADER:Handle<Shader>=bevy::asset::uuid_handle!("833663b0-dcef-4acd-ad9b-399058840ba1");

pub fn register_shaders(app: &mut App) {
    bevy::asset::load_internal_asset!(app,LOCAL_RECEIVER_SHADER,
        "../assets/shaders/dh2_local_receiver.wgsl",Shader::from_wgsl);
    bevy::asset::load_internal_asset!(
        app,
        SHADOW_SHADER,
        "../assets/shaders/dh2_shadow.wgsl",
        |source: &str, path| Shader::from_wgsl(
            format!("#define_import_path dh2_shadow\n{source}"),
            path
        )
    );
    bevy::asset::load_internal_asset!(
        app,
        RECEIVER_SHADER,
        "../assets/shaders/dh2_parallel_receiver.wgsl",
        Shader::from_wgsl
    );
}

/// Resolved original shadowCommon compile options and render parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeShadowOffsetParameters {
    pub sun_direction: [f32; 3],
    pub use_normal_offset: bool,
    pub use_slope_scale: bool,
    pub use_slope_bias: bool,
    pub slope_post_projection: bool,
    /// The original omitted-argument UV-normal-offset caller is still untraced.
    pub use_uv_normal_offset: bool,
    pub normal_offset: f32,
    pub bias: f32,
    pub slope_bias: f32,
}

/// Authored GPU transport layout, not a claim about the original constant-buffer ABI.
#[derive(Clone, Debug, ShaderType)]
pub struct NativeParallelSamplingUniform {
    pub world_to_shadow: [Mat4; 5],
    pub view_row2: Vec4,
    pub cascade_fars: Vec4,
    pub sun_direction: Vec4,
    pub shadow_size: Vec4,
    pub bias: Vec4,
    /// Path: 0=unavailable, 1=direct atlas, 2=screen producer; sun index, options, matrix count.
    pub config: UVec4,
    /// original light quality, original shadowFilteringQuality, unused, unused.
    pub quality: IVec4,
    /// Shared native cloud owner, populated by the same-frame screen pass.
    pub cloud_motion:Vec4,
    pub cloud_density:Vec4,
    pub camera_time:Vec4,
}

impl Default for NativeParallelSamplingUniform {
    fn default() -> Self {
        Self {
            world_to_shadow: [Mat4::ZERO; 5],
            view_row2: Vec4::ZERO,
            cascade_fars: Vec4::ZERO,
            sun_direction: Vec4::ZERO,
            shadow_size: Vec4::ZERO,
            bias: Vec4::ZERO,
            config: UVec4::ZERO,
            quality: IVec4::ZERO,
            cloud_motion:Vec4::ZERO,cloud_density:Vec4::ZERO,camera_time:Vec4::ZERO,
        }
    }
}

impl NativeParallelSamplingUniform {
    /// Atlas production is shared by the original direct and screen-space
    /// consumers. Route the receiver explicitly; a ScreenSpace frame must not
    /// enable the direct forward hook or be rejected before producing its atlas.
    pub fn from_owner_receiver(
        receiver: &crate::native_shadow_scene::NativeParallelReceiverVars,
    ) -> Result<Self, String> {
        match receiver.path {
            crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace =>
                Self::from_screen_receiver(receiver),
            _ => Self::from_receiver(receiver),
        }
    }

    pub fn from_receiver(
        receiver: &crate::native_shadow_scene::NativeParallelReceiverVars,
    ) -> Result<Self, String> {
        if receiver.path != crate::render_shadow::ParallelShadowReceiverPath::DirectVisibility {
            return Err(format!(
                "original parallel receiver path {:?} is not implemented by the scalar atlas transport",
                receiver.path,
            ));
        }
        Self::pack_receiver(receiver)
    }

    /// The original screenShadows program calls ComputeParallelShadow with a
    /// ZERO vertex normal and explicitly disables vertex-normal offset. Its
    /// transport must not accidentally enable the direct-atlas opaque hook.
    pub fn from_screen_receiver(
        receiver: &crate::native_shadow_scene::NativeParallelReceiverVars,
    ) -> Result<Self, String> {
        if receiver.path != crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace {
            return Err(
                "screen-shadow producer requires the original ScreenSpace receiver path".into(),
            );
        }
        let mut value = Self::pack_receiver(receiver)?;
        value.config.x = 2;
        value.config.z &= !1;
        Ok(value)
    }

    fn pack_receiver(
        receiver: &crate::native_shadow_scene::NativeParallelReceiverVars,
    ) -> Result<Self, String> {
        let count = receiver.world_to_shadow.len();
        if count == 0
            || count > 5
            || receiver
                .world_to_shadow
                .iter()
                .flatten()
                .flatten()
                .any(|v| !v.is_finite())
        {
            return Err("native parallel receiver needs 1..=5 finite owner matrices".into());
        }
        if receiver.offset.use_uv_normal_offset {
            return Err("original UV-normal-offset compiled caller remains unsupported".into());
        }
        if receiver.offset.sun_direction.iter().any(|v| !v.is_finite())
            || [
                receiver.offset.normal_offset,
                receiver.offset.bias,
                receiver.offset.slope_bias,
            ]
            .iter()
            .any(|v| !v.is_finite())
            || receiver
                .view_row2
                .iter()
                .chain(&receiver.cascade_fars)
                .any(|v| !v.is_finite())
            || receiver
                .shadow_size
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err(
                "nonfinite native parallel sampling parameters or invalid shadow size".into(),
            );
        }
        let mut value = Self::default();
        for (destination, rows) in value
            .world_to_shadow
            .iter_mut()
            .zip(&receiver.world_to_shadow)
        {
            *destination = Mat4::from_cols_array_2d(rows);
        }
        let flags = u32::from(receiver.offset.use_normal_offset)
            | (u32::from(receiver.offset.use_slope_scale) << 1)
            | (u32::from(receiver.offset.use_slope_bias) << 2)
            | (u32::from(receiver.offset.slope_post_projection) << 3);
        value.view_row2 = Vec4::from_array(receiver.view_row2);
        value.cascade_fars = Vec4::from_array(receiver.cascade_fars);
        value.sun_direction = Vec4::new(
            receiver.offset.sun_direction[0],
            receiver.offset.sun_direction[1],
            receiver.offset.sun_direction[2],
            0.0,
        );
        value.shadow_size = Vec4::new(receiver.shadow_size[0], receiver.shadow_size[1], 0.0, 0.0);
        value.bias = Vec4::new(
            receiver.offset.normal_offset,
            receiver.offset.bias,
            receiver.offset.slope_bias,
            0.0,
        );
        value.config = UVec4::new(1, receiver.directional_light_index, flags, count as u32);
        value.quality = IVec4::new(receiver.light_quality, receiver.filtering_quality, 0, 0);
        Ok(value)
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct NativeParallelSamplingBuffer {
    pub buffer: Handle<ShaderBuffer>,
    pub unavailable_depth: Handle<Image>,
}

/// Shared local receiver transport. The light map follows the renderer's
/// actual entity-to-GPU-index table; shadow records retain the native144-byte
/// layout. Capacity is allocated from the scene registry, never guessed IDs.
#[derive(Resource,Clone,ExtractResource)]
pub struct NativeLocalSamplingBuffer {
    pub buffer:Handle<ShaderBuffer>,pub light_capacity:u32,
}

pub fn local_sampling_word_count(light_capacity:u32)->usize {
    16+light_capacity as usize*16+crate::tiled_lighting::SHADOW_BUFFER_CAPACITY*36
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore="requires GPU and the private live local receiver capture; checks projection-plane non-finite behavior"]
    fn captured_local_receiver_returns_finite_visibility_gpu() {
        use bevy::{prelude::*,render::{RenderApp,render_resource::*,renderer::{RenderDevice,RenderQueue}}};
        let path=std::env::var("DH2_LOCAL_RECEIVER_TEST_CAPTURE")
            .unwrap_or_else(|_|"artifacts/shadow-resume-local-receiver-words.bin".into());
        let raw=std::fs::read(path).unwrap();
        let words=raw.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
        let float=|i|f32::from_bits(words[i]);let mut cases=Vec::new();
        for light in 0..words[1] as usize {
            let at=16+light*16;if words[at+1]==u32::MAX {continue;}
            let origin=[float(at+4),float(at+5),float(at+6)];
            for offset in [[0.0,0.0,0.0],[0.0,0.0,0.001],[1.0,0.0,0.0],[0.0,1.0,0.0],[-10.0,30.0,-10.0],
                [-1.0,0.0,0.0],[0.0,-1.0,0.0],[0.0,0.0,1.0],[0.0,0.0,-1.0]] {
                let p:[f32;3]=std::array::from_fn(|i|origin[i]+offset[i]);
                for flags in 0..16 {for filter in 0..4 {for normal in [[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]] {
                    let mut c=vec![p[0].to_bits(),p[2].to_bits(),(-p[1]).to_bits(),0];
                    c.extend([normal[0],normal[2],-normal[1]].map(f32::to_bits));c.push(0);
                    c.extend([0,flags,filter,2,light as u32,0,0,0]);cases.extend(c);
                }}}
            }
        }
        let count=cases.len()/16;assert!(count>1000);
        let mut app=App::new();app.add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {
            primary_window:None,exit_condition:bevy::window::ExitCondition::DontExit,..default()})
            .disable::<bevy::winit::WinitPlugin>().disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>());
        app.finish();app.cleanup();let world=app.sub_app_mut(RenderApp).world_mut();
        let device=world.resource::<RenderDevice>();let queue=world.resource::<RenderQueue>();
        let atlas=crate::native_shadow_gpu::NativeShadowDepthArray::new_with_format(device,
            Extent3d {width:16,height:16,depth_or_array_layers:8},TextureFormat::Depth24PlusStencil8,"captured local receiver diagnostic").unwrap();
        let source=device.create_buffer_with_data(&BufferInitDescriptor {label:None,contents:&raw,usage:BufferUsages::STORAGE});
        let input=device.create_buffer_with_data(&BufferInitDescriptor {label:None,contents:&cases.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>(),usage:BufferUsages::STORAGE});
        let size=count as u64*4;let output=device.create_buffer_with_data(&BufferInitDescriptor {label:None,
            contents:&(0..count).flat_map(|_|f32::NAN.to_le_bytes()).collect::<Vec<_>>(),usage:BufferUsages::STORAGE|BufferUsages::COPY_SRC});
        let staging=device.create_buffer(&BufferDescriptor {label:None,size,usage:BufferUsages::COPY_DST|BufferUsages::MAP_READ,mapped_at_creation:false});
        let source_code=|source:&str|source.lines().filter(|l|!l.starts_with('#')).collect::<Vec<_>>().join("\n").replace("#{MATERIAL_BIND_GROUP}","0");
        let mut variants=vec![("current",source_code(include_str!("../assets/shaders/dh2_local_receiver.wgsl")))];
        if let Some(path)=std::env::var_os("DH2_LOCAL_RECEIVER_REFERENCE_SHADER") {
            variants.push(("reference",source_code(&std::fs::read_to_string(path).unwrap())));
        }
        let fill_shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:None,source:ShaderSource::Wgsl(r#"
            @vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
                let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
            }
            @fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {return 0.125+floor(p.x)/32.0+floor(p.y)/64.0;}
        "#.into())});
        let fill=device.create_render_pipeline(&RawRenderPipelineDescriptor {label:None,layout:None,
            vertex:RawVertexState {module:&fill_shader,entry_point:Some("vertex"),compilation_options:default(),buffers:&[]},
            fragment:Some(RawFragmentState {module:&fill_shader,entry_point:Some("fragment"),compilation_options:default(),targets:&[]}),
            primitive:default(),depth_stencil:Some(DepthStencilState {format:TextureFormat::Depth24PlusStencil8,
                depth_write_enabled:Some(true),depth_compare:Some(CompareFunction::Always),stencil:default(),bias:default()}),
            multisample:default(),multiview_mask:None,cache:None});
        let mut reference:Option<Vec<u32>>=None;
        for (label,local) in variants {
            let code=format!("{}\n{}\n{}",include_str!("../assets/shaders/dh2_shadow.wgsl"),local,r#"
                struct Case {position:vec4<f32>,normal:vec4<f32>,route:vec4<u32>,ids:vec4<u32>}
                @group(0) @binding(129) var<storage,read> cases:array<Case>;
                @group(0) @binding(130) var<storage,read_write> results:array<f32>;
                @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) i:vec3<u32>) {
                    let c=cases[i.x];results[i.x]=native_local_visibility(c.ids.x,c.position.xyz,c.normal.xyz,c.route);
                }
            "#);
            let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:None,source:ShaderSource::Wgsl(code.into())});
            let pipeline=device.wgpu_device().create_compute_pipeline(&RawComputePipelineDescriptor {label:None,layout:None,module:&shader,entry_point:Some("main"),compilation_options:default(),cache:None});
            let sampler=crate::native_shadow_gpu::native_shadow_comparison_sampler(device);
            let group=device.wgpu_device().create_bind_group(&BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[
                BindGroupEntry{binding:126,resource:source.as_entire_binding()},BindGroupEntry{binding:127,resource:BindingResource::TextureView(&atlas.sampled)},
                BindGroupEntry{binding:128,resource:BindingResource::Sampler(&sampler)},BindGroupEntry{binding:129,resource:input.as_entire_binding()},BindGroupEntry{binding:130,resource:output.as_entire_binding()}]});
            let mut encoder=device.create_command_encoder(&CommandEncoderDescriptor::default());
            for view in &atlas.layers {let mut pass=encoder.begin_render_pass(&RenderPassDescriptor {label:None,color_attachments:&[],depth_stencil_attachment:Some(RenderPassDepthStencilAttachment {
                view,depth_ops:Some(Operations {load:LoadOp::Clear(1.0),store:StoreOp::Store}),stencil_ops:Some(Operations {load:LoadOp::Clear(0),store:StoreOp::Store})}),..default()});pass.set_pipeline(&fill);pass.draw(0..3,0..1);}
            {let mut pass=encoder.begin_compute_pass(&ComputePassDescriptor::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups(count as u32,1,1);}
            encoder.copy_buffer_to_buffer(&output,0,&staging,0,size);queue.submit([encoder.finish()]);
            let (sender,receiver)=std::sync::mpsc::channel();staging.slice(..).map_async(MapMode::Read,move|r|sender.send(r).unwrap());
            device.wgpu_device().poll(PollType::Wait{submission_index:None,timeout:None}).unwrap();receiver.recv().unwrap().unwrap();
            let mapped=staging.slice(..).get_mapped_range();let mut failures=Vec::new();
            for (i,b) in mapped.chunks_exact(4).enumerate() {
                let v=f32::from_le_bytes(b.try_into().unwrap());if !v.is_finite() || !(0.0..=1.0).contains(&v) {failures.push((i,v));}
            }
            assert!(failures.is_empty(),"{} non-finite/out-of-range local visibility cases, first {:?}",failures.len(),&failures[..failures.len().min(12)]);
            let actual=mapped.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
            if let Some(reference)=&reference {
                let differences=actual.iter().zip(reference).enumerate().filter(|(_, (a,b))|a!=b).collect::<Vec<_>>();
                assert!(differences.is_empty(),"local receiver transport differs in {} cases, first {:?}",differences.len(),&differences[..differences.len().min(10)]);
            }
            else {reference=Some(actual);}
            eprintln!("{label}: {count} live local projection-plane/normal/option/filter cases returned finite visibility");drop(mapped);staging.unmap();
        }
    }

    #[test]
    #[ignore="requires GPU; compares local shadowCommon structured-matrix coordinates against original HLSL"]
    fn local_shadow_coordinates_match_original_structured_buffer_gpu() {
        use bevy::{prelude::*,render::{RenderApp,render_resource::*,renderer::{RenderDevice,RenderQueue}}};
        let cases=include_str!("../analysis/original_local_shadow_coordinates.txt").lines().map(|line|
            line.split_whitespace().map(|w|u32::from_str_radix(w,16).unwrap()).collect::<Vec<_>>()).collect::<Vec<_>>();
        let mut app=App::new();
        app.add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {primary_window:None,exit_condition:bevy::window::ExitCondition::DontExit,..default()})
            .disable::<bevy::winit::WinitPlugin>().disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>());
        app.finish();app.cleanup();let world=app.sub_app_mut(RenderApp).world_mut();
        let device=world.resource::<RenderDevice>();let queue=world.resource::<RenderQueue>();
        let input=device.create_buffer_with_data(&BufferInitDescriptor {label:None,
            contents:&cases.iter().flatten().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>(),usage:BufferUsages::STORAGE});
        let size=cases.len() as u64*16;
        let output=device.create_buffer(&BufferDescriptor {label:None,size,usage:BufferUsages::STORAGE|BufferUsages::COPY_SRC,mapped_at_creation:false});
        let staging=device.create_buffer(&BufferDescriptor {label:None,size,usage:BufferUsages::COPY_DST|BufferUsages::MAP_READ,mapped_at_creation:false});
        let source=format!("{}\n{}",include_str!("../assets/shaders/dh2_shadow.wgsl"),r#"
            struct Case { matrix:array<vec4<f32>,4>,position:vec4<f32>,expected:vec4<f32> }
            @group(0) @binding(0) var<storage,read> cases:array<Case>;
            @group(0) @binding(1) var<storage,read_write> results:array<vec4<f32>>;
            @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) i:vec3<u32>) {
                let c=cases[i.x];let m=mat4x4<f32>(c.matrix[0],c.matrix[1],c.matrix[2],c.matrix[3]);
                results[i.x]=native_shadow_position(m,c.position.xyz,false);
            }
        "#);
        let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:None,source:ShaderSource::Wgsl(source.into())});
        let pipeline=device.wgpu_device().create_compute_pipeline(&RawComputePipelineDescriptor {label:None,layout:None,module:&shader,entry_point:Some("main"),compilation_options:default(),cache:None});
        let group=device.wgpu_device().create_bind_group(&BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[
            BindGroupEntry {binding:0,resource:input.as_entire_binding()},BindGroupEntry {binding:1,resource:output.as_entire_binding()}]});
        let mut encoder=device.create_command_encoder(&CommandEncoderDescriptor::default());
        {let mut pass=encoder.begin_compute_pass(&ComputePassDescriptor::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups(cases.len() as u32,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&staging,0,size);queue.submit([encoder.finish()]);
        let (sender,receiver)=std::sync::mpsc::channel();staging.slice(..).map_async(MapMode::Read,move|v|sender.send(v).unwrap());
        device.wgpu_device().poll(PollType::Wait {submission_index:None,timeout:None}).unwrap();receiver.recv().unwrap().unwrap();
        let mapped=staging.slice(..).get_mapped_range();
        for (case,(bytes,expected)) in mapped.chunks_exact(16).zip(&cases).enumerate() {
            let actual:[u32;4]=std::array::from_fn(|i|u32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap()));
            assert_eq!(actual,expected[20..],"original local shadow coordinates{case}");
        }
        assert_eq!(cases.len(),256);drop(mapped);staging.unmap();
    }

    #[test]
    fn owner_transport_preserves_direct_vs_screen_consumer() {
        let mut vars = receiver();
        assert_eq!(NativeParallelSamplingUniform::from_owner_receiver(&vars).unwrap().config.x, 1);
        vars.path = crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace;
        vars.offset.use_normal_offset = true;
        let screen = NativeParallelSamplingUniform::from_owner_receiver(&vars).unwrap();
        assert_eq!(screen.config.x, 2);
        assert_eq!(screen.config.z & 1, 0, "screen shader uses zero normal");
        assert!(NativeParallelSamplingUniform::from_receiver(&vars).is_err(),
            "screen route must never become a direct forward receiver");
        vars.path = crate::render_shadow::ParallelShadowReceiverPath::DirectWithCasterDistance;
        assert!(NativeParallelSamplingUniform::from_owner_receiver(&vars).is_err());
    }

    fn receiver() -> crate::native_shadow_scene::NativeParallelReceiverVars {
        crate::native_shadow_scene::NativeParallelReceiverVars {
            path: crate::render_shadow::ParallelShadowReceiverPath::DirectVisibility,
            world_to_shadow: vec![std::array::from_fn(|r| {
                std::array::from_fn(|c| (r * 4 + c) as f32)
            })],
            view_row2: [0.0, 0.0, -1.0, 0.0],
            cascade_fars: [4.0, 12.0, 30.0, 75.0],
            shadow_size: [256.0, 256.0],
            light_quality: 2,
            filtering_quality: 3,
            directional_light_index: 0,
            offset: NativeShadowOffsetParameters {
                sun_direction: [0.0, 0.0, -1.0],
                use_normal_offset: true,
                use_slope_scale: false,
                use_slope_bias: true,
                slope_post_projection: false,
                use_uv_normal_offset: false,
                normal_offset: 0.01,
                bias: 0.001,
                slope_bias: 0.002,
            },
        }
    }

    #[test]
    fn transport_preserves_original_rows_and_resolved_options() {
        let input = receiver();
        let uniform = NativeParallelSamplingUniform::from_receiver(&input).unwrap();
        assert_eq!(
            uniform.world_to_shadow[0].to_cols_array_2d(),
            input.world_to_shadow[0]
        );
        assert_eq!(uniform.config.to_array(), [1, 0, 5, 1]);
        assert_eq!(uniform.quality.to_array(), [2, 3, 0, 0]);
        assert_eq!(uniform.bias.to_array(), [0.01, 0.001, 0.002, 0.0]);
        let neutral = NativeParallelSamplingUniform::default();
        assert_eq!(neutral.config.x, 0);
        let buffer = ShaderBuffer::from(neutral);
        assert_eq!(buffer.data.unwrap().len(), 480);
    }

    #[test]
    fn opaque_screen_visibility_is_not_replaced_by_direct_atlas_sampling() {
        let mut input = receiver();
        for path in [
            crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace,
            crate::render_shadow::ParallelShadowReceiverPath::DirectWithCasterDistance,
        ] {
            input.path = path;
            assert!(NativeParallelSamplingUniform::from_receiver(&input)
                .unwrap_err()
                .contains("scalar atlas transport"));
        }
    }

    #[test]
    fn screen_producer_transport_keeps_rows_but_disables_vertex_normal_offset() {
        let mut input = receiver();
        assert!(NativeParallelSamplingUniform::from_screen_receiver(&input).is_err());
        input.path = crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace;
        let uniform = NativeParallelSamplingUniform::from_screen_receiver(&input).unwrap();
        assert_eq!(uniform.config.to_array(), [2, 0, 4, 1]);
        assert_eq!(
            uniform.world_to_shadow[0].to_cols_array_2d(),
            input.world_to_shadow[0]
        );
        assert_eq!(uniform.bias.to_array(), [0.01, 0.001, 0.002, 0.0]);
        assert!(NativeParallelSamplingUniform::from_receiver(&input).is_err());
        input.offset.use_uv_normal_offset = true;
        assert!(NativeParallelSamplingUniform::from_screen_receiver(&input).is_err());
    }

    #[test]
    fn unresolved_offset_branch_is_not_silently_substituted() {
        let mut input = receiver();
        input.offset.use_uv_normal_offset = true;
        assert!(NativeParallelSamplingUniform::from_receiver(&input)
            .unwrap_err()
            .contains("unsupported"));
        input.offset.use_uv_normal_offset = false;
        input.offset.bias = f32::NAN;
        assert!(NativeParallelSamplingUniform::from_receiver(&input).is_err());
        input.offset.bias = 0.0;
        input.world_to_shadow.clear();
        assert!(NativeParallelSamplingUniform::from_receiver(&input).is_err());
    }
}
