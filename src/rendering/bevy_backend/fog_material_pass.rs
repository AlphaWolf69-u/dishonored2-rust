//! Host adapter for fx/depthFull: immutable opaque depth for alpha materials.
//! Transparent rendering retains its own camera depth attachment, so sampling
//! this snapshot never aliases Bevy's writable depth/stencil attachment.
use bevy::{
    prelude::default,
    render::{render_resource::*, renderer::RenderDevice},
};

pub struct FogDepthSnapshot {
    pipeline: RenderPipeline,
    source: Option<(TextureViewId, BindGroup)>,
}
impl FogDepthSnapshot {
    pub fn new(device: &RenderDevice) -> Self {
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("fog opaque-depth snapshot"),
            source: ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var source:texture_depth_2d;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return vec4(uv*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
    return textureLoad(source,vec2<i32>(p.xy),0);
}
"#
                .into(),
            ),
        });
        Self {
            pipeline: device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("fog opaque-depth snapshot"),
                layout: None,
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
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
            }),
            source: None,
        }
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        target: &TextureView,
    ) {
        if self
            .source
            .as_ref()
            .is_none_or(|(id, _)| *id != source.id())
        {
            let bind = device.create_bind_group(
                None,
                &BindGroupLayout::from(self.pipeline.get_bind_group_layout(0)),
                &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(source),
                }],
            );
            self.source = Some((source.id(), bind));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("fog opaque-depth snapshot"),
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: target,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.source.as_ref().unwrap().1, &[]);
        pass.draw(0..3, 0..1);
    }
}
