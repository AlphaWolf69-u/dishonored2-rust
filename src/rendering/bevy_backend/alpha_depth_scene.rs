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
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AlphaDepthMaterial {
    pub state: [u32; 9],
    pub flags49: u8,
    pub full: bool,
    pub reduced: bool,
}
impl AlphaDepthMaterial {
    ///The actual enabled depth slots, shared by visible work and admission.
    fn pass_enabled(&self,pass:usize)->bool {
        [self.full,self.reduced,self.full&&self.flags49&1!=0][pass]
    }
}
/// 14031dce0 draws dynamicAlpha (model bucket 0x37) and worldAlpha (world
/// list key 0) through writePixel: weighted-blended accumulation when r_OIT
/// is nonzero, otherwise the material blend into hdrBuffer.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AlphaLitRoute {
    pub state: crate::render_state::RenderState,
    pub accumulate: bool,
}
/// One rendered instance's alpha-list routes: the depth-alpha slots, the
/// hair list (bucket 0x36) and the ALPHA_LIT list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AlphaSurface {
    pub depth: Result<AlphaDepthMaterial, String>,
    pub hair: bool,
    pub lit: Result<Option<AlphaLitRoute>, String>,
}
#[derive(Resource, Default)]
pub struct AlphaDepthSources(pub HashMap<MainEntity, AlphaSurface>);
/// Forward-list partitioning must finish before alpha-depth clones its batches.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlphaDepthPreparation;
/// 14031dce0's model lists: the hair lit pass, hair OIT pass and hair
/// resolve, then dynamicAlpha/worldAlpha, before the particle draws and the
/// alpha list's resolve.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlphaLitPass;
const HAIR_RESOLVE: &str = include_str!("../../../assets/shaders/dh2_hair_resolve.wgsl");
#[derive(Resource, Default)]
pub(crate) struct AlphaGpu {
    /// [lit with depth writes, OIT] per view.
    hair_phases: HashMap<RetainedViewEntity, [SortedRenderPhase<Transparent3d>; 2]>,
    hair_pipelines: HashMap<(CachedRenderPipelineId, u8), CachedRenderPipelineId>,
    /// RGBA16F hair accumulation, R8 hair revealage.
    hair_targets: Option<(FogAttachment, FogAttachment)>,
    hair_resolve: Option<(TextureFormat, BindGroupLayout, RenderPipeline)>,
    /// dynamicAlpha/worldAlpha per view: [accumulation, material blend].
    lit_phases: HashMap<RetainedViewEntity, [SortedRenderPhase<Transparent3d>; 2]>,
    lit_pipelines:
        HashMap<(CachedRenderPipelineId, AlphaLitRoute, TextureFormat, TextureFormat), CachedRenderPipelineId>,
    /// The alpha list's RGBA16F accumulation and RG16F revealage, shared
    /// with the particle draws (36d0c0's packed formats).
    lit_targets: Option<(FogAttachment, FogAttachment)>,
    /// The view whose model draws cleared and accumulated into lit_targets.
    lit_drawn: Mutex<Option<RetainedViewEntity>>,
    opacity: Option<FogAttachment>,
    phases: HashMap<RetainedViewEntity, [SortedRenderPhase<Transparent3d>; 3]>,
    pipelines:
        HashMap<(CachedRenderPipelineId, [u32; 9], u8, TextureFormat), CachedRenderPipelineId>,
    completed: Mutex<Option<(Entity, u32, bool)>>,
}
impl AlphaGpu {
    /// Shared by real draws and asset admission. The latter requests this exact
    /// descriptor before an actor exists; it never manufactures a visible draw.
    pub(crate) fn prepare_lit_pipeline(
        &mut self, cache: &PipelineCache, base: CachedRenderPipelineId,
        route: AlphaLitRoute, colour: TextureFormat, depth: TextureFormat, role: &'static str,
    ) -> Result<Option<CachedRenderPipelineId>, String> {
        let key = (base, route, colour, depth);
        if let Some(id) = self.lit_pipelines.get(&key) { return Ok(Some(*id)); }
        if cache.get_render_pipeline(base).is_none() { return Ok(None); }
        let id = cache.queue_render_pipeline(lit_descriptor(cache, base, route, colour, depth)?);
        self.lit_pipelines.insert(key, id);
        trace_pipeline_queued(role,"lit",base,id,&key);
        Ok(Some(id))
    }

    ///Same selected source pass, bindings and descriptor as visible prepare.
    fn prepare_depth_pipeline(&mut self,cache:&PipelineCache,base:CachedRenderPipelineId,
        material:&AlphaDepthMaterial,pass:u8,depth_format:TextureFormat,role:&'static str)
        ->Result<CachedRenderPipelineId,String>{
        let state=crate::rendering::render_model::depth_alpha_draw_state(material.state,material.flags49,pass)?.state;
        let key = (base, state, pass, depth_format);
        let pipeline = if let Some(id) = self.pipelines.get(&key) {
            *id
        } else {
            let mut descriptor =
                cache.get_render_pipeline_descriptor(base).clone();
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
            self.pipelines.insert(key, id);
            trace_pipeline_queued(role,"depth",base,id,&key);
            id
        };
        Ok(pipeline)
    }
    fn prepare_hair_pipeline(&mut self,cache:&PipelineCache,base:CachedRenderPipelineId,kind:u8,role:&'static str)
        ->Result<Option<CachedRenderPipelineId>,String>{
        let key = (base, kind);
        let pipeline = if let Some(id) = self.hair_pipelines.get(&key) {
            *id
        } else {
            if cache.get_render_pipeline(base).is_none() { return Ok(None); }
            let mut descriptor = cache.get_render_pipeline_descriptor(base).clone();
            let depth = descriptor.depth_stencil.as_mut().ok_or("hair pipeline has no depth state")?;
            // The lit pass writes depth (capture 10249); OIT only tests it.
            depth.depth_write_enabled = Some(kind == 0);
            if kind == 1 {
                descriptor.label = Some("hair oit mesh".into());
                let fragment = descriptor.fragment.as_mut().ok_or("hair MAIN has no fragment")?;
                fragment.shader_defs.extend(["DH2_OIT".into(), "DH2_HAIR_LIST".into()]);
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
            self.hair_pipelines.insert(key, id);
            trace_pipeline_queued(role,"hair",base,id,&key);
            id
        };
        Ok(Some(pipeline))
    }
    ///Program admission consumes exact already-admitted route metadata; no
    ///draw, bindings, textures, targets or material permutations are created.
    pub(crate) fn prepare_surface_pipelines(&mut self,cache:&PipelineCache,base:CachedRenderPipelineId,
        surface:&AlphaSurface,colour:TextureFormat,depth:TextureFormat,oit:bool)
        ->Result<Option<Vec<CachedRenderPipelineId>>,String>{
        if cache.get_render_pipeline(base).is_none(){return Ok(None);}
        let material=surface.depth.as_ref().map_err(Clone::clone)?;
        let mut ids=Vec::new();
        for pass in 0..3u8 {
            if !material.pass_enabled(pass as usize){continue;}
            ids.push(self.prepare_depth_pipeline(cache,base,material,pass,
                if pass==2{depth}else{TextureFormat::Depth32FloatStencil8},"preload")?);
        }
        if surface.hair {
            for kind in 0..2u8 {
                let Some(id)=self.prepare_hair_pipeline(cache,base,kind,"preload")? else{return Ok(None);};
                ids.push(id);
            }
        }else if let Some(route)=surface.lit.as_ref().map_err(Clone::clone)? {
            let Some(id)=self.prepare_lit_pipeline(cache,base,AlphaLitRoute{accumulate:oit,..*route},colour,depth,"preload")? else{return Ok(None);};
            ids.push(id);
        }
        Ok(Some(ids))
    }

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
        gpu.lit_phases.clear();
        gpu.lit_targets = None;
        *gpu.completed.get_mut().unwrap_or_else(|error| error.into_inner()) = None;
    }

    /// The alpha list's targets for the particle draws and resolve.
    pub(super) fn alpha_list_targets(
        &self,
        view: RetainedViewEntity,
    ) -> Option<super::particle_pass::AlphaListTargets<'_>> {
        let (accumulation, opacity) = self.lit_targets.as_ref()?;
        Some(super::particle_pass::AlphaListTargets {
            accumulation: &accumulation.view,
            opacity: &opacity.view,
            drawn: *self.lit_drawn.lock().unwrap() == Some(view),
        })
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
                draw_alpha_lit
                    .in_set(Core3dSystems::MainPass)
                    .in_set(AlphaLitPass)
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
    *gpu.lit_drawn.get_mut().unwrap() = None;
    gpu.phases.clear();
    gpu.hair_phases.clear();
    gpu.lit_phases.clear();
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
                    if let Some(material) = sources.0.get(&item.entity.1).map(|s| &s.depth) {
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
                                if !material.pass_enabled(pass)
                                {
                                    continue;
                                }
                                let depth_format = if pass == 2 {
                                    main_depth.texture.format()
                                } else {
                                    TextureFormat::Depth32FloatStencil8
                                };
                                let pipeline = gpu.prepare_depth_pipeline(&cache, item.pipeline,
                                    material, pass as u8, depth_format,"visible")?;
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
                let is_hair = sources.0.get(&item.entity.1).is_some_and(|surface| surface.hair);
                if is_hair {
                    for kind in 0..2u8 {
                        let Some(pipeline) = gpu.prepare_hair_pipeline(&cache, item.pipeline, kind,"visible")? else {
                            pending = true;
                            break;
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
            // 14033b350: dynamicAlpha (bucket 0x37) and worldAlpha leave the
            // ordinary transparent phase for 14031dce0's alpha list.
            let extent = UVec2::new(target.main_texture().width(), target.main_texture().height());
            if gpu.lit_targets.as_ref().is_none_or(|(accumulation, _)|
                accumulation.texture.width() != extent.x || accumulation.texture.height() != extent.y) {
                gpu.lit_targets = Some((
                    FogAttachment::new(&device, "alpha list accumulation", extent, TextureFormat::Rgba16Float),
                    FogAttachment::new(&device, "alpha list revealage", extent, TextureFormat::Rg16Float),
                ));
            }
            let mut lit: [SortedRenderPhase<Transparent3d>; 2] = std::array::from_fn(|_| Default::default());
            let oit_mode = crate::platform::console::shared().read().cvars.int("r_OIT").unwrap_or(1);
            let mut lit_members = HashSet::new();
            let mut index = 0;
            while index < source.items.len() {
                let item = &source.items[index];
                let count = item.batch_range.len().max(1);
                let members = index..(index + count).min(source.items.len());
                let route = match sources.0.get(&item.entity.1).map(|surface| &surface.lit) {
                    // 31dce0 tests r_OIT for every draw of the list.
                    Some(Ok(route)) => route.map(|route| AlphaLitRoute { accumulate: oit_mode != 0, ..route }),
                    Some(Err(error)) => {
                        if unsupported.insert(error.clone()) {
                            warn!("Alpha list route pending: {error}");
                        }
                        None
                    }
                    None => None,
                };
                if let Some(route) = route.filter(|_| !hair_members.contains(&index)) {
                    let format = target.main_texture_format();
                    // 31dce0 tests zBuffer, the view depth.
                    let depth_format = main_depth.texture.format();
                    let pipeline = gpu.prepare_lit_pipeline(&cache, item.pipeline, route, format, depth_format,"visible")?;
                    // A compiling pipeline keeps its surfaces out of both paths
                    // for that frame, as the ordinary pipelines do.
                    if let Some(pipeline) = pipeline.filter(|id| cache.get_render_pipeline(*id).is_some()) {
                        for member in members.clone() {
                            lit[usize::from(!route.accumulate)]
                                .add_retained(copy_item(&source.items[member], pipeline));
                        }
                    }
                    lit_members.extend(members);
                }
                index += count;
            }
            if lit.iter().any(|phase| !phase.items.is_empty()) {
                info_once!(members = lit_members.len(), "Alpha list model surfaces queued");
                gpu.lit_phases.insert(view.retained_view_entity, lit);
            }
            if !hair_members.is_empty() || !lit_members.is_empty() {
                // The transparent phase is retained across frames: its items
                // stay queued, and this frame's batches of the rerouted
                // surfaces are emptied so the ordinary pass skips them.
                // Pending pipelines keep them out of both paths for that
                // frame, as the ordinary pipelines do while compiling.
                if let Some(phase) = phases.get_mut(&view.retained_view_entity) {
                    for (position, item) in phase.items.values_mut().enumerate() {
                        if hair_members.contains(&position) || lit_members.contains(&position) {
                            item.batch_range.end = item.batch_range.start;
                        }
                    }
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

/// The pipeline of one dynamicAlpha/worldAlpha surface: its material state
/// with 31dce0's overrides, writePixel's two targets when accumulating.
fn lit_descriptor(
    cache: &PipelineCache,
    base: CachedRenderPipelineId,
    route: AlphaLitRoute,
    colour: TextureFormat,
    depth: TextureFormat,
) -> Result<RenderPipelineDescriptor, String> {
    let (state, _) = crate::render_state::alpha_surface_draw_state(route.state, route.accumulate, false);
    let blends = crate::render_state::blend_state(&state);
    let mut descriptor = cache.get_render_pipeline_descriptor(base).clone();
    descriptor.label = Some(if route.accumulate { "alpha list accumulation mesh" } else { "alpha list mesh" }.into());
    let fragment = descriptor.fragment.as_mut().ok_or("alpha list MAIN has no fragment")?;
    let formats = if route.accumulate {
        fragment.shader_defs.push("DH2_OIT".into());
        vec![TextureFormat::Rgba16Float, TextureFormat::Rg16Float]
    } else {
        vec![colour]
    };
    fragment.targets = formats
        .into_iter()
        .enumerate()
        .map(|(index, format)| {
            let blend = blends.targets[index].ok_or("alpha list colour target absent")?;
            Ok(Some(ColorTargetState {
                format,
                blend: super::pipeline_state::blend_target(blend)?,
                write_mask: ColorWrites::from_bits_truncate(u32::from(blend.write_mask)),
            }))
        })
        .collect::<Result<_, String>>()?;
    descriptor.depth_stencil = Some(super::pipeline_state::depth_stencil_gpu(&state, depth)?);
    descriptor.multisample.count = 1;
    descriptor.multisample.alpha_to_coverage_enabled = false;
    Ok(descriptor)
}

/// 14031dce0's model lists: hair, then dynamicAlpha/worldAlpha against
/// zBuffer with stencil reference 0x44, as the particle draws.
fn draw_alpha_lit(
    world: &World,
    view: ViewQuery<(&MainEntity, &ExtractedView, &ViewTarget, &ViewDepthTexture)>,
    gpu: Res<AlphaGpu>,
    fog: Res<FogGpu>,
    water: Res<WaterSceneFrame>,
    device: Res<RenderDevice>,
    mut context: RenderContext,
    mut reported: Local<bool>,
) {
    let entity = view.entity();
    let (main, extracted, target, main_depth) = view.into_inner();
    draw_hair(world, entity, extracted, target, main_depth, &gpu, &device, &mut context);
    let Some(phases) = gpu.lit_phases.get(&extracted.retained_view_entity) else {
        return;
    };
    if water.0.as_ref().is_none_or(|w| w.camera != main.id()) {
        return;
    }
    let Some((accumulation, revealage)) = gpu.lit_targets.as_ref() else {
        return;
    };
    if !main_depth.texture.format().has_stencil_aspect()
        || main_depth.texture.width() != accumulation.texture.width()
        || main_depth.texture.height() != accumulation.texture.height()
    {
        warn_once!("Alpha list depth {:?} does not match its targets", main_depth.texture.size());
        return;
    }
    for (index, phase) in phases.iter().enumerate() {
        if phase.items.is_empty() {
            continue;
        }
        let accumulate = index == 0;
        let colours = if accumulate {
            vec![
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
            ]
        } else {
            vec![Some(target.get_color_attachment())]
        };
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some(if accumulate { "alpha list accumulation" } else { "alpha list" }),
            color_attachments: &colours,
            // 31dce0 binds zBuffer, the view depth, for models and
            // particles alike: its stencil carries the depth stage's
            // underwater marks (2fdea0, 0x40) that reference 0x44 tests.
            depth_stencil_attachment: Some(main_depth.get_attachment(StoreOp::Store)),
            ..Default::default()
        });
        // 31dce0 passes reference 0x44 for draws without model+1c1c bit 3.
        pass.set_stencil_reference(0x44);
        if let Err(error) = phase.render(&mut pass, world, entity) {
            error!("Alpha list submission failed: {error:?}");
            return;
        }
        if accumulate {
            *gpu.lit_drawn.lock().unwrap() = Some(extracted.retained_view_entity);
        }
    }
    if !*reported {
        info!(accumulated = phases[0].items.len(), blended = phases[1].items.len(),
            "Alpha list model submission active");
        *reported = true;
    }
}

/// arkHair's lit pass (blend SRC_ALPHA, depth writes), its OIT pass into the
/// hair accumulation and revealage, and the hair resolve into the view.
#[allow(clippy::too_many_arguments)]
fn draw_hair(
    world: &World,
    entity: Entity,
    extracted: &ExtractedView,
    target: &ViewTarget,
    main_depth: &ViewDepthTexture,
    gpu: &AlphaGpu,
    device: &RenderDevice,
    context: &mut RenderContext,
) {
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

///No history or new observer: a receipt only when the canonical cache queues
///a new derived program. Cache-hit rendering does not call this function.
fn trace_pipeline_queued(role:&'static str,kind:&'static str,
    base:CachedRenderPipelineId,derived:CachedRenderPipelineId,key:&impl std::fmt::Debug){
    static TRACE:std::sync::OnceLock<bool>=std::sync::OnceLock::new();
    if *TRACE.get_or_init(||std::env::var_os("DH2_TRACE_WORLD_FX").is_some()){
        eprintln!("FX_ALPHA_PIPELINE role={role} kind={kind} base={base:?} derived={derived:?} key={key:?}");
    }
}
