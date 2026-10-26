//! Submit the admitted alpha-depth slots with the forward mesh's real bindings.
use super::{fog_scene::FogGpu, fog_targets::FogAttachment, water_scene::WaterSceneFrame};
use bevy::{
    core_pipeline::{core_3d::Transparent3d, Core3d, Core3dSystems},
    prelude::*,
    render::{
        render_phase::{SortedRenderPhase, ViewSortedRenderPhases},
        render_resource::*,
        renderer::{RenderContext, RenderDevice, ViewQuery},
        sync_world::MainEntity,
        view::{ExtractedView, RetainedViewEntity, ViewDepthTexture, ViewTarget},
        Render, RenderApp, RenderSystems,
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

const SHADER: Handle<Shader> = bevy::asset::uuid_handle!("e3a49cd7-5e51-4226-a2b9-d0a1084c1481");
#[derive(Clone, PartialEq, Eq)]
pub struct AlphaDepthMaterial {
    pub state: [u32; 9],
    pub flags49: u8,
    pub full: bool,
    pub reduced: bool,
}
#[derive(Resource, Default)]
pub struct AlphaDepthSources(pub HashMap<MainEntity, Result<AlphaDepthMaterial, String>>);
#[derive(Resource, Default)]
pub(super) struct AlphaGpu {
    opacity: Option<FogAttachment>,
    phases: HashMap<RetainedViewEntity, [SortedRenderPhase<Transparent3d>; 3]>,
    pipelines:
        HashMap<(CachedRenderPipelineId, [u32; 9], u8, TextureFormat), CachedRenderPipelineId>,
    completed: Mutex<Option<(Entity, u32, bool)>>,
}
impl AlphaGpu {
    pub(super) fn opacity_texture(&self, camera: Entity, frame: u32) -> Option<&Texture> {
        self.opacity(camera, frame)?;
        Some(&self.opacity.as_ref()?.texture)
    }
    pub(super) fn opacity(&self, camera: Entity, frame: u32) -> Option<(&TextureView, bool)> {
        let (id, number, special) = (*self.completed.lock().unwrap())?;
        (id == camera && number == frame).then(|| (&self.opacity.as_ref().unwrap().view, special))
    }
}
pub fn register(app: &mut App) {
    bevy::asset::load_internal_asset!(
        app,
        SHADER,
        "../../../assets/shaders/dh2_alpha_depth.wgsl",
        Shader::from_wgsl
    );
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<AlphaDepthSources>()
            .init_resource::<AlphaGpu>()
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(
                Core3d,
                draw.in_set(Core3dSystems::MainPass)
                    .after(super::fog_scene::AtmosphereDepthPass)
                    .before(super::fog_scene::AtmospherePass),
            );
    }
}
fn copy_item(item: &Transparent3d, pipeline: CachedRenderPipelineId) -> Transparent3d {
    Transparent3d {
        sorting_info: item.sorting_info,
        distance: item.distance,
        pipeline,
        entity: item.entity,
        draw_function: item.draw_function,
        batch_range: item.batch_range.clone(),
        extra_index: item.extra_index.clone(),
        indexed: item.indexed,
    }
}
fn prepare(
    mut gpu: ResMut<AlphaGpu>,
    sources: Res<AlphaDepthSources>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    display: Res<super::display_scene::DisplaySceneFrame>,
    phases: Res<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<(&ExtractedView, &ViewTarget, &ViewDepthTexture)>,
    mut last_error: Local<Option<String>>,
    mut unsupported: Local<HashSet<String>>,
) {
    *gpu.completed.get_mut().unwrap() = None;
    gpu.phases.clear();
    // Alpha depth is also a fog input, independently of the DoF switch.
    if display.0.is_none() {
        return;
    }
    let result = (|| -> Result<(), String> {
        for (view, target, main_depth) in &views {
            let Some(source) = phases.get(&view.retained_view_entity) else {
                continue;
            };
            let mut selected: [SortedRenderPhase<Transparent3d>; 3] =
                std::array::from_fn(|_| Default::default());
            let mut pending = false;
            let mut index = 0;
            while index < source.items.len() {
                let item = &source.items[index];
                let count = item.batch_range.len().max(1);
                if !item.batch_range.is_empty() {
                    if let Some(material) = sources.0.get(&item.entity.1) {
                        let material = match material {
                            Ok(material) => material,
                            Err(error) => {
                                if unsupported.insert(error.clone()) {
                                    warn!("Alpha depth material pending: {error}");
                                }
                                index += count;
                                continue;
                            }
                        };
                        if cache.get_render_pipeline(item.pipeline).is_some() {
                            for pass in 0..3 {
                                if ![
                                    material.full,
                                    material.reduced,
                                    material.full && material.flags49 & 1 != 0,
                                ][pass]
                                {
                                    continue;
                                }
                                let state = crate::rendering::render_model::depth_alpha_draw_state(
                                    material.state,
                                    material.flags49,
                                    pass as u8,
                                )?
                                .state;
                                let depth_format = if pass == 2 {
                                    main_depth.texture.format()
                                } else {
                                    TextureFormat::Depth32FloatStencil8
                                };
                                let key = (item.pipeline, state, pass as u8, depth_format);
                                let pipeline = if let Some(id) = gpu.pipelines.get(&key) {
                                    *id
                                } else {
                                    let mut descriptor =
                                        cache.get_render_pipeline_descriptor(item.pipeline).clone();
                                    descriptor.label = Some("alpha depth mesh".into());
                                    descriptor.vertex.shader_defs.retain(|value| !matches!(value,bevy::shader::ShaderDefVal::Bool(name,_) if name=="DH2_FOG_VERTEX"));
                                    let fragment = descriptor
                                        .fragment
                                        .as_mut()
                                        .ok_or("alpha depth MAIN has no fragment")?;
                                    fragment.shader_defs.retain(|value| !matches!(value,bevy::shader::ShaderDefVal::Bool(name,_) if name=="DH2_FOG_VERTEX"));
                                    fragment.shader = SHADER;
                                    if material.flags49 & 1 != 0 {
                                        fragment.shader_defs.push("DH2_REFRACTION".into());
                                    }
                                    fragment.entry_point =
                                        Some(if pass != 1 { "full" } else { "reduced" }.into());
                                    fragment.targets = if pass != 1 {
                                        let target = crate::render_state::blend_state(&state)
                                            .targets[0]
                                            .ok_or("alpha depth colour state absent")?;
                                        vec![Some(ColorTargetState {
                                            format: TextureFormat::Rg16Float,
                                            blend: super::pipeline_state::blend_target(target)?,
                                            write_mask: ColorWrites::from_bits_truncate(u32::from(
                                                target.write_mask,
                                            )),
                                        })]
                                    } else {
                                        Vec::new()
                                    };
                                    descriptor.depth_stencil =
                                        Some(super::pipeline_state::depth_stencil_gpu(
                                            &state,
                                            depth_format,
                                        )?);
                                    // Keep the actual mesh's vertex deformation, handedness, culling and bindings.
                                    descriptor.multisample.count = 1;
                                    descriptor.multisample.alpha_to_coverage_enabled = false;
                                    let id = cache.queue_render_pipeline(descriptor);
                                    gpu.pipelines.insert(key, id);
                                    id
                                };
                                if cache.get_render_pipeline(pipeline).is_some() {
                                    for member in index..(index + count).min(source.items.len()) {
                                        selected[pass].add_retained(copy_item(
                                            &source.items[member],
                                            pipeline,
                                        ));
                                    }
                                } else {
                                    pending = true;
                                }
                            }
                        } else {
                            pending = true;
                        }
                    }
                }
                index += count;
            }
            if !pending && selected.iter().any(|p| !p.items.is_empty()) {
                let extent = UVec2::new(
                    target.main_texture().width(),
                    target.main_texture().height(),
                );
                if gpu
                    .opacity
                    .as_ref()
                    .is_none_or(|a| a.texture.width() != extent.x || a.texture.height() != extent.y)
                {
                    gpu.opacity = Some(FogAttachment::with_usage(
                        &device,
                        "opacity buffer",
                        extent,
                        TextureFormat::Rg16Float,
                        TextureUsages::COPY_SRC,
                    ));
                }
                gpu.phases.insert(view.retained_view_entity, selected);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        gpu.phases.clear();
        if last_error.as_ref() != Some(&error) {
            warn!("Alpha depth pending: {error}");
            *last_error = Some(error);
        }
    } else {
        *last_error = None;
    }
}
fn draw(
    world: &World,
    view: ViewQuery<(&MainEntity, &ExtractedView, &ViewDepthTexture)>,
    gpu: Res<AlphaGpu>,
    fog: Res<FogGpu>,
    fog_frame: Res<super::fog::FogSceneFrame>,
    water: Res<WaterSceneFrame>,
    mut context: RenderContext,
) {
    let entity = view.entity();
    let (main, extracted, main_depth) = view.into_inner();
    let Some(water) = water.0.as_ref().filter(|w| w.camera == main.id()) else {
        return;
    };
    let Some(frame) = fog_frame.0.as_ref().filter(|f| f.camera == main.id()) else {
        return;
    };
    if !(1..=2).contains(&frame.settings.downsample) {
        return;
    }
    let Some(phases) = gpu.phases.get(&extracted.retained_view_entity) else {
        return;
    };
    let Some(opacity) = gpu.opacity.as_ref() else {
        return;
    };
    let Some(depth) = fog.alpha_depth(main.id(), water.matrices.frame) else {
        return;
    };
    if depth.alpha.full.format() != TextureFormat::Depth32FloatStencil8
        || !main_depth.texture.format().has_stencil_aspect()
        || main_depth.texture.sample_count() != 1
        || main_depth.texture.width() != opacity.texture.width()
        || main_depth.texture.height() != opacity.texture.height()
    {
        return;
    }
    for (index, phase) in phases.iter().enumerate() {
        let (texture, attachment) = if index != 1 {
            (&depth.alpha.full, &depth.alpha.full_attachment)
        } else {
            (&depth.alpha.reduced, &depth.alpha.reduced_attachment)
        };
        let colors = [Some(RenderPassColorAttachment {
            view: &opacity.view,
            resolve_target: None,
            depth_slice: None,
            ops: Operations {
                load: if index == 0 {
                    LoadOp::Clear(bevy::color::LinearRgba::new(1., 0., 0., 1.).into())
                } else {
                    LoadOp::Load
                },
                store: StoreOp::Store,
            },
        })];
        let depth_attachment = if index == 2 {
            main_depth.get_attachment(StoreOp::Store)
        } else {
            RenderPassDepthStencilAttachment {
                view: attachment,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
            }
        };
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("alpha depth geometry"),
            color_attachments: if index != 1 { &colors } else { &[] },
            depth_stencil_attachment: Some(depth_attachment),
            ..Default::default()
        });
        //321860 divides the full view by the quality shift. The attachment's
        //minimum allocation size does not enlarge the geometry viewport.
        let viewport = if index == 1 {
            UVec2::new(depth.alpha.full.width(), depth.alpha.full.height())
                >> frame.settings.downsample as u32
        } else {
            UVec2::new(texture.width(), texture.height())
        };
        if viewport.min_element() == 0 {
            return;
        }
        pass.set_viewport(
            0.,
            0.,
            viewport.x as f32,
            viewport.y as f32,
            0.,
            1.,
        );
        pass.set_scissor_rect(
            0,
            0,
            viewport.x.min(texture.width()),
            viewport.y.min(texture.height()),
        );
        pass.set_stencil_reference(if index == 2 { 0x80 } else { 2 });
        if let Err(error) = phase.render(&mut pass, world, entity) {
            error!("Alpha depth submission failed: {error:?}");
            return;
        }
    }
    *gpu.completed.lock().unwrap() =
        Some((main.id(), water.matrices.frame, !phases[2].items.is_empty()));
}
