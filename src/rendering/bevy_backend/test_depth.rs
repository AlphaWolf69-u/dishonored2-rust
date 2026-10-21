//! Numeric depth fixtures shared by GPU shader comparisons.
use bevy::{
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
pub struct DepthUploader {
    pipeline: RenderPipeline,
}
impl DepthUploader {
    pub fn new(device: &RenderDevice) -> Self {
        let fill = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("refraction numeric depth upload"),
            source: ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var<storage,read> values:array<f32>;
@group(0) @binding(1) var<uniform> shape:vec4<u32>;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((i<<1u)&2u),f32(i&2u));return vec4(uv*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
    return values[shape.z*shape.x*shape.y+u32(p.y)*shape.x+u32(p.x)];
}
"#
                .into(),
            ),
        });
        let fill_pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: RawVertexState {
                module: &fill,
                entry_point: Some("vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &fill,
                entry_point: Some("fragment"),
                compilation_options: default(),
                targets: &[],
            }),
            primitive: default(),
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Always),
                stencil: default(),
                bias: default(),
            }),
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline: fill_pipeline,
        }
    }
    pub fn upload(
        &self,
        device: &RenderDevice,
        queue: &RenderQueue,
        width: u32,
        height: u32,
        layers: u32,
        values: Vec<f32>,
    ) -> Texture {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("refraction depth fixture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let input = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &values
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
            usage: BufferUsages::STORAGE,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        for layer in 0..layers {
            let shape = device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents: &[width, height, layer, 0]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
                usage: BufferUsages::UNIFORM,
            });
            let binding = device.create_bind_group(
                None,
                &self.pipeline.get_bind_group_layout(0).into(),
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: input.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: shape.as_entire_binding(),
                    },
                ],
            );
            let view = texture.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..default()
            });
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        texture
    }
}
