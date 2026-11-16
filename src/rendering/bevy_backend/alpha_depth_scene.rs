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
use hashbrown::{HashMap, HashSet};
use std::{sync::Mutex,
};

const SHADER: Handle<Shader> = bevy::asset::uuid_handle!("e3a49cd7-5e51-4226-a2b9-d0a1084c1481");
#[derive(Clone, PartialEq, Eq)]
pub struct AlphaDepthMaterial {
    pub state: [u32; 9],
    pub flags49: u8,
    pub full: bool,
    pub reduced: bool,
    /// arkHair: drawn by the hair pass (lit with depth writes, then OIT)
    /// instead of the ordinary transparent phase.
    pub hair: bool,
}
#[derive(Resource, Default)]
pub struct AlphaDepthSources(pub HashMap<MainEntity, Result<AlphaDepthMaterial, String>>);
/// Forward-list partitioning must finish before alpha-depth clones its batches.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlphaDepthPreparation;
/// The hair lit pass, hair OIT pass and hair resolve, before the particle
/// list's accumulation and resolve.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HairPass;
const HAIR_RESOLVE: &str = include_str!("../../../assets/shaders/dh2_hair_resolve.wgsl");
#[derive(Resource, Default)]
pub(super) struct AlphaGpu {
    /// [lit with depth writes, OIT] per view.
    hair_phases: HashMap<RetainedViewEntity, [SortedRenderPhase<Transparent3d>; 2]>,
    hair_pipelines: HashMap<(CachedRenderPipelineId, u8), CachedRenderPipelineId>,
    /// RGBA16F hair accumulation, R8 hair revealage.
    hair_targets: Option<(FogAttachment, FogAttachment)>,
    hair_resolve: Option<(TextureFormat, BindGroupLayout, RenderPipeline)>,
    opacity: Option<FogAttachment>,
    phases: HashMap<RetainedViewEntity, [SortedRenderPhase<Transparent3d>; 3]>,
    pipelines:
        HashMap<(CachedRenderPipelineId, [u32; 9], u8, TextureFormat), CachedRenderPipelineId>,
    completed: Mutex<Option<(Entity, u32, bool)>>,
}
impl AlphaGpu {
    pub(super) fn ensure_opacity(&mut self, device: &RenderDevice, extent: UVec2) {
        if self.opacity.as_ref().is_none_or(|image| {
            image.texture.width() != extent.x || image.texture.height() != extent.y
        }) {
            self.opacity = Some(FogAttachment::with_usage(
                device, "opacity buffer", extent, TextureFormat::Rg16Float, TextureUsages::COPY_SRC,
            ));
        }
    }

    pub(super) fn release_level(world: &mut World) {
        let Some(mut gpu) = world.get_resource_mut::<Self>() else { return };
        if let Some(opacity) = gpu.opacity.take() {
            if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
                eprintln!("CAMERA_DEPTH_RELEASE owner=opacity extent={:?}", opacity.texture.size());
            }
        }
        gpu.phases.clear();
        gpu.hair_phases.clear();
        gpu.hair_targets = None;
        *gpu.completed.get_mut().unwrap_or_else(|error| error.into_inner()) = None;
    }

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
            .add_systems(
                Render,
                prepare
                    .in_set(RenderSystems::PrepareBindGroups)
                    .in_set(AlphaDepthPreparation),
            )
            .add_systems(
                Core3d,
                // depthAlpha's geometry precedes the TLF gather (eotw capture:
                // alpha draws 26312-27291, gather 28463, main pass 38372).
                draw.in_set(Core3dSystems::MainPass)
                    .after(super::fog_scene::SceneDepthPass)
                    .before(super::fog_scene::SceneLightsPass),
            )
            .add_systems(
                Core3d,
                // Original menu capture: hair lit pass 10249, hair OIT
                // 10536, particle OIT 10626, composite 10659.
                draw_hair
                    .in_set(Core3dSystems::MainPass)
                    .in_set(HairPass)
                    .after(super::fog_scene::AtmospherePass)
                    .before(bevy::core_pipeline::core_3d::main_transparent_pass_3d),
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
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<(&ExtractedView, &ViewTarget, &ViewDepthTexture)>,
    mut last_error: Local<Option<String>>,
    mut unsupported: Local<HashSet<String>>,
) {
    *gpu.completed.get_mut().unwrap() = None;
    gpu.phases.clear();
    gpu.hair_phases.clear();
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
                                    // Programs whose alpha test differs in the
                                    // alpha-depth passes (arkHair: r_hairAlphaTestAlpha).
                                    fragment.shader_defs.push("DH2_ALPHA_DEPTH".into());
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
            // arkHair leaves the ordinary transparent phase for the hair pass.
            let mut hair: [SortedRenderPhase<Transparent3d>; 2] = std::array::from_fn(|_| Default::default());
            let mut hair_members = HashSet::new();
            let mut index = 0;
            while index < source.items.len() {
                let item = &source.items[index];
                let count = item.batch_range.len().max(1);
                let is_hair = sources.0.get(&item.entity.1)
                    .is_some_and(|material| material.as_ref().is_ok_and(|material| material.hair));
                if is_hair {
                    for kind in 0..2u8 {
                        let key = (item.pipeline, kind);
                        let pipeline = if let Some(id) = gpu.hair_pipelines.get(&key) {
                            *id
                        } else {
                            let Some(_) = cache.get_render_pipeline(item.pipeline) else {
                                pending = true;
                                break;
                            };
                            let mut descriptor = cache.get_render_pipeline_descriptor(item.pipeline).clone();
                            let depth = descriptor.depth_stencil.as_mut().ok_or("hair pipeline has no depth state")?;
                            // The lit pass writes depth (capture 10249); OIT only tests it.
                            depth.depth_write_enabled = Some(kind == 0);
                            if kind == 1 {
                                descriptor.label = Some("hair oit mesh".into());
                                let fragment = descriptor.fragment.as_mut().ok_or("hair MAIN has no fragment")?;
                                fragment.shader_defs.push("DH2_HAIR_OIT".into());
                                let add = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One,
                                    operation: BlendOperation::Add };
                                let reveal = BlendComponent { src_factor: BlendFactor::Zero,
                                    dst_factor: BlendFactor::OneMinusSrc, operation: BlendOperation::Add };
                                fragment.targets = vec![
                                    Some(ColorTargetState { format: TextureFormat::Rgba16Float,
                                        blend: Some(BlendState { color: add, alpha: add }), write_mask: ColorWrites::ALL }),
                                    Some(ColorTargetState { format: TextureFormat::R8Unorm,
                                        blend: Some(BlendState { color: reveal, alpha: reveal }), write_mask: ColorWrites::RED }),
                                ];
                            } else {
                                descriptor.label = Some("hair lit mesh".into());
                            }
                            let id = cache.queue_render_pipeline(descriptor);
                            gpu.hair_pipelines.insert(key, id);
                            id
                        };
                        if cache.get_render_pipeline(pipeline).is_none() {
                            pending = true;
                            continue;
                        }
                        for member in index..(index + count).min(source.items.len()) {
                            hair[kind as usize].add_retained(copy_item(&source.items[member], pipeline));
                        }
                    }
                    for member in index..(index + count).min(source.items.len()) {
                        hair_members.insert(member);
                    }
                }
                index += count;
            }
            if !hair_members.is_empty() && !pending {
                let extent = UVec2::new(target.main_texture().width(), target.main_texture().height());
                if gpu.hair_targets.as_ref().is_none_or(|(accumulation, _)|
                    accumulation.texture.width() != extent.x || accumulation.texture.height() != extent.y) {
                    gpu.hair_targets = Some((
                        FogAttachment::new(&device, "hair accumulation", extent, TextureFormat::Rgba16Float),
                        FogAttachment::new(&device, "hair revealage", extent, TextureFormat::R8Unorm),
                    ));
                }
                gpu.hair_phases.insert(view.retained_view_entity, hair);
                ensure_hair_resolve(&mut gpu, &device, target.main_texture_format());
            }
            if !hair_members.is_empty() {
                // Pending hair pipelines keep the surfaces out of both paths
                // for that frame, as the ordinary pipelines do while compiling.
                if let Some(phase) = phases.get_mut(&view.retained_view_entity) {
                    let mut position = 0;
                    phase.items.retain(|_, _| {
                        let keep = !hair_members.contains(&position);
                        position += 1;
                        keep
                    });
                }
            }
            if !pending && selected.iter().any(|p| !p.items.is_empty()) {
                let extent = UVec2::new(
                    target.main_texture().width(),
                    target.main_texture().height(),
                );
                gpu.ensure_opacity(&device, extent);
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
        pass.set_viewport(0., 0., viewport.x as f32, viewport.y as f32, 0., 1.);
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

/// arkHair's lit pass (blend SRC_ALPHA, depth writes), its OIT pass into the
/// hair accumulation and revealage, and the hair resolve into the view.
fn draw_hair(
    world: &World,
    view: ViewQuery<(&ExtractedView, &ViewTarget, &ViewDepthTexture)>,
    gpu: Res<AlphaGpu>,
    device: Res<RenderDevice>,
    mut context: RenderContext,
) {
    let entity = view.entity();
    let (extracted, target, main_depth) = view.into_inner();
    let Some(phases) = gpu.hair_phases.get(&extracted.retained_view_entity) else {
        return;
    };
    if phases[0].items.is_empty() {
        return;
    }
    let Some((accumulation, revealage)) = gpu.hair_targets.as_ref() else {
        return;
    };
    {
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("hair lit"),
            color_attachments: &[Some(target.get_color_attachment())],
            depth_stencil_attachment: Some(main_depth.get_attachment(StoreOp::Store)),
            ..Default::default()
        });
        if let Err(error) = phases[0].render(&mut pass, world, entity) {
            error!("Hair lit submission failed: {error:?}");
            return;
        }
    }
    {
        let colors = [
            Some(RenderPassColorAttachment {
                view: &accumulation.view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations { load: LoadOp::Clear(Default::default()), store: StoreOp::Store },
            }),
            Some(RenderPassColorAttachment {
                view: &revealage.view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Clear(bevy::color::LinearRgba::new(1., 0., 0., 1.).into()),
                    store: StoreOp::Store,
                },
            }),
        ];
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("hair oit"),
            color_attachments: &colors,
            depth_stencil_attachment: Some(main_depth.get_attachment(StoreOp::Store)),
            ..Default::default()
        });
        if let Err(error) = phases[1].render(&mut pass, world, entity) {
            error!("Hair OIT submission failed: {error:?}");
            return;
        }
    }
    let Some((format, layout, pipeline)) = gpu.hair_resolve.as_ref() else {
        return;
    };
    if *format != target.main_texture_format() {
        return;
    }
    let binding = device.create_bind_group(Some("hair resolve"), layout, &[
        BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&accumulation.view) },
        BindGroupEntry { binding: 1, resource: BindingResource::TextureView(&revealage.view) },
    ]);
    let encoder = context.command_encoder();
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("hair resolve"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target.main_texture_view(),
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &binding, &[]);
    pass.draw(0..3, 0..1);
}

fn ensure_hair_resolve(gpu: &mut AlphaGpu, device: &RenderDevice, format: TextureFormat) {
    if gpu.hair_resolve.as_ref().is_none_or(|(old, _, _)| *old != format) {
        let texture = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(Some("hair resolve"), &[texture(0), texture(1)]);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("hair resolve"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("hair resolve"),
            source: ShaderSource::Wgsl(HAIR_RESOLVE.into()),
        });
        // PostFX_CompositeWeightedBlendedOIT: blend ONE SRC_ALPHA, no depth.
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("hair resolve"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState { module: &shader, entry_point: Some("resolve_vertex"),
                compilation_options: default(), buffers: &[] },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("resolve_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState {
                        color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::SrcAlpha,
                            operation: BlendOperation::Add },
                        alpha: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::SrcAlpha,
                            operation: BlendOperation::Add },
                    }),
                    write_mask: ColorWrites::COLOR,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        gpu.hair_resolve = Some((format, layout, pipeline));
    }
}
