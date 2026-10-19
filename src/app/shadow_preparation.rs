//! Shadow preparation.
use super::{
    archive, bmd6, edge_skeleton, entities, game_clock, game_clock_runtime, game_settings,
    game_to_view, local_shadow_updates, material, placed_direction, placed_vertex, render_model,
    render_program, render_shadow, render_state, retain_native_vegetation_channels,
    retain_native_world_shadow_channels, screen_shadow_pass, shader_cache, shadow_pass,
    shadow_pipeline, shadow_sampling, shadow_updates, source_mesh, surface_gpu, world_mesh, Arc,
    ExtendedMaterial, HashMap, MaterialExtension, MaterialHandle, NativeShadowImageAccess,
    RenderAssetUsages, SceneData, SceneRenderQueue, ShaderBuffer, DH2_BITANGENT,
};
use bevy::prelude::*;

/// Resolved once beside the real material/mesh association; the native CPU
/// shadow owner consumes this rather than reparsing declarations every frame.
#[derive(Clone, Debug)]
pub(super) struct NativeShadowMaterialInputs {
    pub(super) source: material::ShadowSourceInputs,
    /// Actual slot-10 row-zero source identification. This is not the complete
    /// native pointer bank, draw-state override, or a ready shadow frame.
    pub(super) rigid_base_source:
        Result<Option<std::sync::Arc<shader_cache::CachedRigidShadowSource>>, String>,
    /// Native material +0x48 bit0 tests bank0 slot10 (+0xf0), regardless
    /// of whether that program's rigid ABI has been ported. An unsupported
    /// shader-class error must not be mistaken for an absent native pointer.
    pub(super) base_slot10_present: Result<bool, String>,
    /// Actual bank2 slot10 (+0x250), not a rigid-source substitute.
    pub(super) world_source:
        Result<Option<std::sync::Arc<shader_cache::CachedWorldShadowSource>>, String>,
    pub(super) world_parameters: Result<Option<NativeShadowMaterialParameters>, String>,
    pub(super) main_program: Option<String>,
    pub(super) alpha_test: bool,
    pub(super) world_uv_set: Option<u32>,
    pub(super) diffuse_image_slot: Option<usize>,
    pub(super) parameters: Result<Option<NativeShadowMaterialParameters>, String>,
    pub(super) cull_mode: Option<bevy::render::render_resource::Face>,
}

impl NativeShadowMaterialInputs {
    /// Only the two original material flags consumed by FUN_1402996f0.
    /// Slot presence comes from +0xf0; needsWind uses the MAIN fallback
    /// (1402d565a..1402d567e), not the specialized shadow shader's defaults.
    pub(super) fn world_gather_flags48(&self) -> Result<u8, String> {
        Ok(u8::from(self.base_slot10_present.clone()?) | (u8::from(self.source.needs_wind) << 6))
    }
}

/// Literal material overrides and registered defaults remain separate from the
/// draw owner's optional fallback. In particular, neither a forward material's
/// UV values nor guessed wind/clock values become native shadow constants.
#[derive(Clone, Debug)]
pub(super) struct NativeShadowMaterialParameters {
    overrides: render_program::SerializedParameterBlock,
    defaults: HashMap<Vec<u8>, [u32; 4]>,
    resolved: bevy::platform::collections::HashMap<Vec<u8>, Result<[u32; 4], String>>,
}

impl NativeShadowMaterialParameters {
    pub(super) fn new(
        overrides: render_program::SerializedParameterBlock,
        defaults: HashMap<Vec<u8>, [u32; 4]>,
    ) -> Self {
        // These serialized declarations are immutable. Keep raw words and
        // deferred errors; gameplay clocks and per-model fallback blocks are
        // still resolved at their original call sites.
        let resolved = defaults
            .iter()
            .map(|(name, default)| {
                (
                    name.clone(),
                    overrides.vector_words(name, 0x30, *default, None),
                )
            })
            .collect();
        Self {
            overrides,
            defaults,
            resolved,
        }
    }

    #[cfg(test)]
    pub(super) fn declarations(
        &self,
    ) -> (
        &render_program::SerializedParameterBlock,
        &HashMap<Vec<u8>, [u32; 4]>,
    ) {
        (&self.overrides, &self.defaults)
    }

    pub(super) fn vector(
        &self,
        name: &[u8],
        fallback: Option<&render_program::SerializedParameterBlock>,
    ) -> Result<[f32; 4], String> {
        if fallback.is_none() {
            return self
                .resolved
                .get(name)
                .ok_or_else(|| {
                    format!(
                        "unresolved native shadow Uniform {}",
                        String::from_utf8_lossy(name)
                    )
                })?
                .as_ref()
                .map(|words| words.map(f32::from_bits))
                .map_err(Clone::clone);
        }
        let default = *self.defaults.get(name).ok_or_else(|| {
            format!(
                "unresolved native shadow Uniform {}",
                String::from_utf8_lossy(name)
            )
        })?;
        self.overrides
            .vector_words(name, 0x30, default, fallback)
            .map(|words| words.map(f32::from_bits))
    }

    pub(super) fn vegetation(
        &self,
        fallback: Option<&render_program::SerializedParameterBlock>,
    ) -> Result<shadow_pass::NativeVegetationShadowParameters, String> {
        Ok(shadow_pass::NativeVegetationShadowParameters {
            main_variation: self.vector(b"vegetanim/mainbendingvar", fallback)?,
            main_frequency: self.vector(b"vegetanim/mainbendingfreq", fallback)?,
            main_amplitude: self.vector(b"vegetanim/mainbendingamplitude", fallback)?,
            minimum_wind: self.vector(b"vegetanim/leafminwindstrength", fallback)?,
            leaf_frequency: self.vector(b"vegetanim/leaffreq", fallback)?,
            leaf_amplitude: self.vector(b"vegetanim/leafamplitude", fallback)?,
            branch_amplitude: self.vector(b"vegetanim/branchamplitude", fallback)?,
        })
    }

    /// Resolve the real material vectors and original model-selected gameplay
    /// clock into VS14200's exact 208-byte record. The draw owner still supplies
    /// fallback, native model flags/offset, MVP and depth multiplier explicitly.
    pub(super) fn vegetation_instance_cb(
        &self,
        fallback: Option<&render_program::SerializedParameterBlock>,
        clocks: &game_clock::GameplayViewClocks,
        model_offset: f32,
        model_flags_1c31: u8,
        mvp_rows: &[[f32; 4]; 4],
        model_depth_multiplier: f32,
    ) -> Result<[u8; shadow_pass::NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES], String> {
        let local_time = clocks
            .model_local_time(model_offset, model_flags_1c31)
            .ok_or_else(|| {
                format!(
                    "unavailable native vegetation clock group {}",
                    render_model::model_shader_time_group(model_flags_1c31)
                )
            })?;
        Ok(shadow_pass::native_vegetation_shadow_instance_cb(
            self.vector(b"layer0_scalebias", fallback)?,
            &self.vegetation(fallback)?,
            mvp_rows,
            local_time,
            model_depth_multiplier,
        ))
    }
}

#[derive(Resource, Default)]
pub(super) struct NativeShadowMaterials(
    pub(super)  bevy::platform::collections::HashMap<
        MaterialHandle,
        Result<NativeShadowMaterialInputs, String>,
    >,
);

impl NativeShadowMaterials {
    pub(super) fn skin_influences(&self, material: &MaterialHandle) -> u8 {
        self.0
            .get(material)
            .and_then(|inputs| inputs.as_ref().ok())
            .and_then(|inputs| inputs.rigid_base_source.as_ref().ok())
            .and_then(Option::as_ref)
            .map_or(0, |program| program.skin_influences)
    }
    pub(super) fn uses_vegetation(&self, material: &MaterialHandle) -> bool {
        let Some(Ok(inputs)) = self.0.get(material) else {
            return false;
        };
        let Ok(Some(source)) = &inputs.rigid_base_source else {
            return false;
        };
        source.class == shader_cache::CachedRigidShadowClass::Vegetation
    }
}

/// Original vertex inputs retained only for meshes whose resolved native source
/// consumes them. Errors remain available to the shadow owner, not synthetic
/// normals/colors or a silently substituted rigid program.
#[derive(Resource, Default)]
pub(super) struct NativeVegetationMeshInputs(
    pub(super) HashMap<bevy::asset::AssetId<Mesh>, Result<usize, String>>,
);

/// Original BWM draw-object ownership kept beside the shared forward meshes.
/// The native world-shadow owner addresses object IDs and matrix ranges, while
/// the forward scene may use per-matrix baked meshes for exact affine shear.
#[derive(Resource, Default)]
pub(super) struct NativeWorldShadowMeshSources {
    pub(super) objects: Vec<NativeWorldShadowMeshSource>,
    pub(super) shared_mesh_count: usize,
}

#[derive(Clone)]
pub(super) struct NativeWorldShadowMeshSource {
    pub(super) object_id: usize,
    pub(super) mesh_index: usize,
    pub(super) matrix_start: usize,
    pub(super) matrix_end: usize,
    pub(super) lod: usize,
    pub(super) flipped: bool,
    pub(super) mesh: Handle<Mesh>,
    pub(super) raw_vertex_count: Result<usize, String>,
    /// The actual material resolved for this BWM object's forward instances.
    /// Native world records likewise own one material pointer at +0x18.
    /// None means no binding was resolved; it must not imply no shadows.
    pub(super) material: Option<MaterialHandle>,
    pub(super) material_error: Option<String>,
    /// Native slot admission survives an intentionally absent color binding.
    /// In particular noShadow/screen-gate/invisible materials must be tested
    /// by the native pointer-bank rule before requiring a GPU draw material.
    pub(super) gather_flags48: Option<Result<u8, String>>,
}

impl NativeWorldShadowMeshSources {
    pub(super) fn prepare(world: &world_mesh::WorldMesh, meshes: &mut Assets<Mesh>) -> Self {
        let mut shared = HashMap::<(usize, bool), (Handle<Mesh>, Result<usize, String>)>::new();
        let mut objects = Vec::with_capacity(world.objects.len());
        for (object_id, object) in world.objects.iter().enumerate() {
            let source = &world.meshes[object.mesh_index];
            let key = (object.mesh_index, object.flipped);
            let (mesh, raw_vertex_count) = if let Some((mesh, count)) = shared.get(&key) {
                (mesh.clone(), count.clone())
            } else {
                let mut forward = source_mesh(
                    &source.positions,
                    &source.uvs,
                    &source.indices,
                    &source.packed_channels,
                    object.flipped,
                );
                let raw_vertex_count = retain_native_world_shadow_channels(
                    &mut forward,
                    &source.positions,
                    &source.packed_channels,
                );
                // Per-frame native program admission validates the original
                // vertex ABI after render extraction as well as at startup.
                forward.asset_usage |= RenderAssetUsages::MAIN_WORLD;
                let mesh = meshes.add(forward);
                shared.insert(key, (mesh.clone(), raw_vertex_count.clone()));
                (mesh, raw_vertex_count)
            };
            objects.push(NativeWorldShadowMeshSource {
                object_id,
                mesh_index: object.mesh_index,
                matrix_start: object.matrix_start,
                matrix_end: object.matrix_end,
                lod: object.lod,
                flipped: object.flipped,
                mesh,
                raw_vertex_count,
                material: None,
                material_error: None,
                gather_flags48: None,
            });
        }
        Self {
            objects,
            shared_mesh_count: shared.len(),
        }
    }

    pub(super) fn object(&self, object_id: usize) -> Option<&NativeWorldShadowMeshSource> {
        self.objects
            .get(object_id)
            .filter(|source| source.object_id == object_id)
    }

    pub(super) fn bind_material(
        &mut self,
        object_id: usize,
        material: &MaterialHandle,
    ) -> Result<(), String> {
        let source = self
            .objects
            .get_mut(object_id)
            .filter(|source| source.object_id == object_id)
            .ok_or("native BWM material binding has no object owner")?;
        if let Some(existing) = &source.material {
            if existing != material {
                return Err(format!(
                    "native BWM object {object_id} resolved conflicting material owners"
                ));
            }
        } else {
            source.material = Some(material.clone());
            source.material_error = None;
        }
        Ok(())
    }

    pub(super) fn record_material_error(
        &mut self,
        object_id: usize,
        error: String,
    ) -> Result<(), String> {
        let source = self
            .objects
            .get_mut(object_id)
            .filter(|source| source.object_id == object_id)
            .ok_or("native BWM material error has no object owner")?;
        if source.material.is_none() {
            source.material_error = Some(error);
        }
        Ok(())
    }
}

impl NativeVegetationMeshInputs {
    pub(super) fn prepare<const N: usize>(
        &mut self,
        required: bool,
        handle: &Handle<Mesh>,
        meshes: &mut Assets<Mesh>,
        positions: &[[f32; 3]],
        packed: &[[u8; N]],
    ) {
        if !required || self.0.contains_key(&handle.id()) {
            return;
        }
        let result = meshes
            .get_mut(handle)
            .ok_or_else(|| "native vegetation source Mesh is unavailable".to_string())
            .and_then(|mut mesh| retain_native_vegetation_channels(&mut mesh, positions, packed))
            .map(|()| positions.len());
        if let Err(error) = &result {
            eprintln!(
                "Native vegetation Mesh {:?} unavailable: {error}",
                handle.id()
            );
        }
        self.0.insert(handle.id(), result);
    }
}

pub(super) fn retain_native_model_shadow_position(
    meshes: &mut Assets<Mesh>,
    handle: &Handle<Mesh>,
    positions: &[[f32; 3]],
) {
    let mut mesh = meshes
        .get_mut(handle)
        .expect("new native model Mesh retains its startup owner");
    if !mesh.contains_attribute(shadow_pass::NATIVE_VEGETATION_POSITION) {
        mesh.insert_attribute(shadow_pass::NATIVE_VEGETATION_POSITION, positions.to_vec());
    }
}

/// Installed synchronous program-cache owner. Data streams stay lazy and
/// read-only; this is source data, NOT a fabricated executable shadow bank.
/// Keeping it separate from SceneData avoids invalidating visibility resources
/// when an on-demand shader read advances a file cursor.
#[derive(Resource)]
pub(super) struct NativeProgramCacheSource(
    pub(super) Result<shader_cache::NativeProgramLibrary, String>,
);

pub(super) fn native_rest_skinning(
    catalog: &archive::Catalog,
    resource: &str,
    model: &bmd6::ModelInfo,
    buffers: &mut Assets<ShaderBuffer>,
) -> Result<Arc<shadow_updates::NativeRestSkinning>, String> {
    let binding = bmd6::resolve_skeleton_binding(resource, |path| {
        catalog
            .read(path)
            .ok()
            .flatten()
            .map(|bytes| entities::native_declaration_text(&bytes).into_owned())
    })?;
    let name = binding
        .as_ref()
        .map(|binding| binding.name.as_str())
        .filter(|name| !name.is_empty())
        .or_else(|| (!model.skeleton_name.is_empty()).then_some(model.skeleton_name.as_str()))
        .ok_or("native MD6 init has no actual skeleton binding")?;
    let stem = name
        .strip_suffix(".md6skl")
        .ok_or("unsupported native skeleton resource suffix")?;
    let path = format!("generated/skeleton/{stem}.edgeskl");
    let bytes = catalog
        .read(&path)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("native skeleton {path} is absent"))?;
    let skeleton = edge_skeleton::parse(&bytes)?;
    let root = edge_skeleton::JointRestPose {
        rotation: [0.0, 0.0, 0.0, 1.0],
        translation: binding
            .as_ref()
            .and_then(|binding| binding.offset)
            .unwrap_or([0.0; 3]),
        scale: [1.0; 3],
    };
    let current = edge_skeleton::native_pose_matrices(&skeleton.rest, &skeleton.hierarchy, &root)?;
    let matrices =
        edge_skeleton::native_skin_palette(&current, &skeleton.inverse_bind[..current.len()])?;
    if matrices.is_empty()
        || matrices
            .iter()
            .flatten()
            .flatten()
            .any(|value| !value.is_finite())
    {
        return Err("native rest skin palette is empty or non-finite".into());
    }
    let gpu = matrices
        .iter()
        .map(|matrix| matrix.map(Vec4::from_array))
        .collect::<Vec<_>>();
    Ok(Arc::new(shadow_updates::NativeRestSkinning {
        palette: buffers.add(ShaderBuffer::from(gpu)),
        matrices,
    }))
}

pub(super) fn retain_native_skin_inputs(mesh: &mut Mesh, surface: &bmd6::SurfaceInfo) {
    use bevy::mesh::VertexAttributeValues;
    use shadow_pass::*;
    for (attribute, values, start) in [
        (NATIVE_SKIN_INDICES_0, &surface.skin_indices, 0),
        (NATIVE_SKIN_INDICES_1, &surface.skin_indices, 4),
    ] {
        mesh.insert_attribute(
            attribute,
            VertexAttributeValues::Uint8x4(
                values
                    .iter()
                    .map(|v| v[start..start + 4].try_into().unwrap())
                    .collect(),
            ),
        );
    }
    for (attribute, values, start) in [
        (NATIVE_SKIN_WEIGHTS_0, &surface.skin_weights, 0),
        (NATIVE_SKIN_WEIGHTS_1, &surface.skin_weights, 4),
    ] {
        mesh.insert_attribute(
            attribute,
            VertexAttributeValues::Unorm8x4(
                values
                    .iter()
                    .map(|v| v[start..start + 4].try_into().unwrap())
                    .collect(),
            ),
        );
    }
}

/// The rewrite currently owns a static rest pose. Use that actual palette in
/// both color geometry and shadow geometry; raw bind inputs stay separate.
pub(super) fn apply_native_rest_pose(
    mesh: &mut Mesh,
    surface: &bmd6::SurfaceInfo,
    palette: &[[[f32; 4]; 3]],
    influences: u8,
    placement: &entities::PlacedModel,
    world_baked: bool,
) -> Result<(), String> {
    if surface.skin_indices.len() != surface.positions.len()
        || surface.skin_weights.len() != surface.positions.len()
    {
        return Err("native skin vertex streams have different lengths".into());
    }
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tangents = Vec::new();
    let mut bitangents = Vec::new();
    let direction = |v| {
        if world_baked {
            placed_direction(placement, v)
        } else {
            game_to_view(v.to_array())
        }
    };
    for i in 0..surface.positions.len() {
        let matrix = edge_skeleton::native_vertex_skin_matrix(
            palette,
            surface.matrix_offset,
            surface.skin_indices[i],
            surface.skin_weights[i],
            influences,
        )?;
        let transform = |v: Vec3, w: f32| {
            Vec3::from_array(matrix.map(|r| Vec4::from_array(r).dot(v.extend(w))))
        };
        let p = transform(Vec3::from_array(surface.positions[i]), 1.0);
        positions.push(if world_baked {
            placed_vertex(placement, p.to_array()).to_array()
        } else {
            game_to_view(p.to_array()).to_array()
        });
        let packed = surface.packed_channels[i];
        let snorm = |b: u8| ((b as i8 as f32) / 127.0).max(-1.0);
        let n = transform(
            Vec3::new(snorm(packed[4]), snorm(packed[5]), snorm(packed[6])),
            0.0,
        );
        let t = transform(
            Vec3::new(snorm(packed[8]), snorm(packed[9]), snorm(packed[10])),
            0.0,
        );
        let sign = snorm(packed[11]);
        normals.push(direction(n).to_array());
        tangents.push(direction(t).extend(sign).to_array());
        bitangents.push(direction(n.cross(t) * sign).to_array());
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    mesh.insert_attribute(DH2_BITANGENT, bitangents);
    Ok(())
}

#[derive(Clone)]
pub(super) struct CachedMaterial {
    pub(super) handle: MaterialHandle,
    pub(super) diffuse_image_slot: Option<usize>,
    pub(super) images: Vec<(usize, surface_gpu::ImageRole)>,
    pub(super) native_vertex_lighting: bool,
}

/// Persistent native directional target and level-owned preparation. This is
/// separate from NativeShadowSceneFrame: allocating the atlas must never turn
/// unresolved per-frame caster/receiver inputs into a fabricated Ready frame.
///
/// Whole-view caster submission is NOT a Tome/color-visible-ID query:
/// FUN_14032de00 (analysis/decompiled/chunk_000120.c:2646) builds world draw
/// masks in 4096-record chunks using object LOD low bits and group parents.
/// WorldMesh::for_each_whole_view_shadow_batch preserves its native overlap
/// arithmetic. The first masks below belong to initial publication, not a
/// cross-frame cache; later world submissions must run the same rule again.
/// Material/program packet submission is still separate from mask selection.
/// FUN_1402996f0 (analysis/decompiled/chunk_000107.c:3544) gathers selected
/// record IDs using material +0x48 masks0x01/0x40, binds structured world buffers,
/// then resolves shadow program +0x250. Its packet uses the first record's
/// serialized final u16: the loader maps FILE +28 to RUNTIME +0x1e, leaving
/// separate +0x1c for the nonserialized wind-center index. World shader
/// remapping and the bank2 source are resolved separately from rigid models.
#[derive(Resource)]
pub(super) struct NativeParallelShadowOwner {
    pub(super) target_image: Handle<Image>,
    pub(super) dimensions: shadow_pass::NativeParallelShadowTargetDimensions,
    pub(super) whole_view: render_shadow::DirectionalCascade,
    pub(super) whole_view_initial_masks: Vec<Vec<u8>>,
    pub(super) hardware_bias: [f32; 3],
    pub(super) shader_bias: [f32; 3],
    pub(super) sort_shadows_ccw: bool,
    /// Source definition byte+4 copied into view+0x39d4. This selects the
    /// actual world `$localTime`; it is not a render-model clock selector.
    pub(super) source_flag_04: u8,
    pub(super) cascade_controls: game_settings::ShadowCascadeControls,
    /// Parsed r_shadowCascadeFars CVar; None retains its executable default.
    pub(super) cascade_fars: [f32; 4],
}

impl NativeParallelShadowOwner {
    pub(super) fn world_draw_settings(&self) -> NativeWorldShadowDrawSettings {
        NativeWorldShadowDrawSettings {
            dimensions: self.dimensions,
            hardware_bias: self.hardware_bias,
            sort_shadows_ccw: self.sort_shadows_ccw,
            source_flag_04: self.source_flag_04,
            exclude_lod: false,
            depth_clip: 0,
        }
    }

    pub(super) fn from_scene(
        scene: &SceneData,
        images: &mut Assets<Image>,
    ) -> Result<Self, String> {
        let dimensions = scene
            .shadow_target_dimensions
            .ok_or("native shadow target settings are unresolved")?;
        let environment = scene
            .environment_lighting
            .ok_or("native shadow environment is unresolved")?;
        let parameters = scene
            .environment_render_parameters
            .as_ref()
            .ok_or("native shadow environment render parameters are unresolved")?;
        let authored = scene
            .far_shadow_bounds
            .ok_or("native world m_farShadowsArea default/override is unresolved")?;
        let bounds = if render_shadow::whole_view_shadow_uses_world_bounds(authored) {
            scene
                .world
                .static_query_bounds()
                .ok_or("native static-world shadow query has no root")?
        } else {
            authored
        };
        let toward_sun = render_shadow::environment_sun_direction(
            environment
                .sun_theta_degrees
                .ok_or("native sun theta is unresolved")?,
            environment
                .sun_phi_degrees
                .ok_or("native sun phi is unresolved")?,
        );
        let ray = toward_sun.map(|value| f32::from_bits(value.to_bits() ^ 0x8000_0000));
        let near = render_shadow::directional_shadow_near(environment.parallel_light_extend)?;
        let whole_view = render_shadow::whole_view_shadow_projection(
            bounds,
            ray,
            near,
            dimensions.width as i32,
        )?;
        let hardware_bias = render_shadow::parallel_hardware_bias(
            parameters,
            dimensions.depth_32_bits,
            scene
                .shadow_depth_bias_clamp
                .ok_or("native r_depthBiasClamp is unresolved")?,
        )?;
        let shader_bias =
            render_shadow::parallel_shader_bias(parameters, dimensions.depth_32_bits)?;
        let sort_shadows_ccw = scene
            .shadow_sort_ccw
            .ok_or("native r_sortShadowsCCW is unresolved")?;
        let cascade_controls = scene
            .shadow_cascade_controls
            .ok_or("native cascade update controls are unresolved")?;
        let cascade_fars = scene.shadow_cascade_fars.unwrap_or([4.0, 12.0, 30.0, 75.0]);
        let mut whole_view_initial_masks = Vec::new();
        scene.world.for_each_whole_view_shadow_batch(|mask| {
            whole_view_initial_masks.push(mask.to_vec());
        })?;
        let format = match render_shadow::parallel_shadow_depth_format(dimensions.depth_32_bits)? {
            render_shadow::ParallelShadowDepthFormat::Depth16 => {
                bevy::render::render_resource::TextureFormat::Depth16Unorm
            }
            render_shadow::ParallelShadowDepthFormat::Depth24Stencil8 => {
                bevy::render::render_resource::TextureFormat::Depth24PlusStencil8
            }
        };
        // Resolve all source-dependent inputs before mutating Assets: an error
        // must not leave a new, unused atlas behind on every initialization.
        let target_image = images.add(shadow_pipeline::shadow_depth_image(
            bevy::render::render_resource::Extent3d {
                width: dimensions.width,
                height: dimensions.height,
                depth_or_array_layers: dimensions.layers,
            },
            format,
        )?);
        Ok(Self {
            target_image,
            dimensions,
            whole_view,
            whole_view_initial_masks,
            hardware_bias,
            shader_bias,
            sort_shadows_ccw,
            source_flag_04: game_clock::RESET_VIEW_LOCAL_TIME_FLAG,
            cascade_controls,
            cascade_fars,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NativeWorldShadowMaskPlan {
    pub(super) instance_ids: Vec<u32>,
    pub(super) auxiliary: Vec<[u32; 8]>,
    pub(super) instances: Vec<NativeWorldShadowPreparedInstance>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NativeWorldShadowPreparedInstance {
    pub(super) mesh: Handle<Mesh>,
    pub(super) constants: NativeWorldShadowPreparedConstants,
    pub(super) program: shadow_pass::NativeShadowProgram,
    pub(super) alpha_material: Option<shadow_pass::NativeShadowAlphaMaterial>,
    pub(super) front_face: bevy::render::render_resource::FrontFace,
    pub(super) cull_mode: Option<bevy::render::render_resource::Face>,
    pub(super) draw_state: [u32; 9],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NativeWorldShadowPreparedConstants {
    World {
        bytes: [u8; shadow_pass::NATIVE_WORLD_SHADOW_INSTANCE_BYTES],
        instance_count: u32,
    },
    Vegetation {
        bytes: [u8; shadow_pass::NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES],
        instance_count: u32,
        uses_wind_buffer: bool,
    },
}

/// Resolve every initial native mask to CPU-owned draw inputs before creating
/// any ShaderBuffer assets. A missing off-camera alpha image is queued through
/// the ordinary texture streamer and retried; no caster is silently dropped.
pub(super) fn prepare_native_whole_view_masks(
    owner: &NativeParallelShadowOwner,
    world: &world_mesh::WorldMesh,
    sources: &NativeWorldShadowMeshSources,
    materials: &NativeShadowMaterials,
    meshes: &Assets<Mesh>,
    images: &Assets<Image>,
    queue: &mut super::scene_queue::ShadowTextureRequests<'_>,
    local_time: Option<[f32; 2]>,
    clock_error: Option<&str>,
) -> Result<Vec<NativeWorldShadowMaskPlan>, String> {
    prepare_native_world_shadow_masks(
        owner,
        world,
        sources,
        materials,
        meshes,
        images,
        queue,
        &owner.whole_view_initial_masks,
        &owner.whole_view.clip,
        local_time,
        clock_error,
    )
}

pub(super) fn prepare_native_world_shadow_masks(
    owner: &NativeParallelShadowOwner,
    world: &world_mesh::WorldMesh,
    sources: &NativeWorldShadowMeshSources,
    materials: &NativeShadowMaterials,
    meshes: &Assets<Mesh>,
    images: &Assets<Image>,
    queue: &mut super::scene_queue::ShadowTextureRequests<'_>,
    masks: &[Vec<u8>],
    clip: &render_shadow::ShadowMatrix,
    local_time: Option<[f32; 2]>,
    clock_error: Option<&str>,
) -> Result<Vec<NativeWorldShadowMaskPlan>, String> {
    prepare_native_world_shadow_masks_with_settings(
        &owner.world_draw_settings(),
        world,
        sources,
        materials,
        meshes,
        images,
        queue,
        masks,
        clip,
        local_time,
        clock_error,
    )
}

pub(super) struct NativeWorldShadowDrawSettings {
    pub(super) dimensions: shadow_pass::NativeParallelShadowTargetDimensions,
    pub(super) hardware_bias: [f32; 3],
    pub(super) sort_shadows_ccw: bool,
    pub(super) source_flag_04: u8,
    pub(super) exclude_lod: bool,
    pub(super) depth_clip: u8,
}

/// Immutable inputs shared only within one frame. Borrowing prevents source
/// mutation during reuse; every next frame revalidates the current associations.
pub(super) struct NativeWorldShadowAdmission<'a> {
    world: &'a world_mesh::WorldMesh,
    sources: &'a NativeWorldShadowMeshSources,
    materials: &'a NativeShadowMaterials,
    flags: [std::sync::OnceLock<Result<Vec<u8>, String>>; 2],
}

impl<'a> NativeWorldShadowAdmission<'a> {
    pub(super) fn new(
        world: &'a world_mesh::WorldMesh,
        sources: &'a NativeWorldShadowMeshSources,
        materials: &'a NativeShadowMaterials,
    ) -> Self {
        Self {
            world,
            sources,
            materials,
            flags: Default::default(),
        }
    }

    fn flags(&self, exclude_lod: bool) -> Result<&[u8], String> {
        self.flags[usize::from(exclude_lod)]
            .get_or_init(|| {
                let (world, sources, materials) = (self.world, self.sources, self.materials);
                let mut flags48 = Vec::with_capacity(world.objects.len());
                for (object_id, object) in world.objects.iter().enumerate() {
                    let source = sources.object(object_id).ok_or_else(|| {
                        format!("native BWM object {object_id} has no retained mesh owner")
                    })?;
                    if source.mesh_index != object.mesh_index
                        || source.matrix_start != object.matrix_start
                        || source.matrix_end != object.matrix_end
                        || source.lod != object.lod
                        || source.flipped != object.flipped
                    {
                        return Err(format!(
                            "native BWM object {object_id} source association changed"
                        ));
                    }
                    if let Some(flags) = &source.gather_flags48 {
                        flags48.push(flags.clone().map_err(|error| {
                            format!("native BWM object {object_id} admission: {error}")
                        })?);
                        continue;
                    }
                    let Some(material) = source.material.as_ref() else {
                        if object.matrix_start == object.matrix_end {
                            // Empty record ranges cannot contribute a native packet.
                            flags48.push(0);
                            continue;
                        }
                        return Err(source.material_error.clone().unwrap_or_else(|| {
                            format!(
                                "native BWM object {object_id} has no resolved material +0x18 owner"
                            )
                        }));
                    };
                    let inputs = materials
                        .0
                        .get(material)
                        .ok_or_else(|| {
                            format!(
                                "native BWM object {object_id} material has no native input record"
                            )
                        })?
                        .as_ref()
                        .map_err(|error| {
                            format!("native BWM object {object_id} material input: {error}")
                        })?;
                    flags48.push(inputs.world_gather_flags48().map_err(|error| {
                        format!("native BWM object {object_id} +0x48 flags: {error}")
                    })?);
                }

                if exclude_lod {
                    //2a2490/2a3080/2a4850 reject native object+c bit0 before gathering.
                    for (flags, object) in flags48.iter_mut().zip(&world.objects) {
                        if object.lod & 1 != 0 {
                            *flags = 0;
                        }
                    }
                }

                Ok(flags48)
            })
            .as_ref()
            .map(Vec::as_slice)
            .map_err(Clone::clone)
    }
}

pub(super) fn prepare_native_world_shadow_masks_with_settings(
    owner: &NativeWorldShadowDrawSettings,
    world: &world_mesh::WorldMesh,
    sources: &NativeWorldShadowMeshSources,
    materials: &NativeShadowMaterials,
    meshes: &Assets<Mesh>,
    images: &Assets<Image>,
    queue: &mut super::scene_queue::ShadowTextureRequests<'_>,
    masks: &[Vec<u8>],
    clip: &render_shadow::ShadowMatrix,
    local_time: Option<[f32; 2]>,
    clock_error: Option<&str>,
) -> Result<Vec<NativeWorldShadowMaskPlan>, String> {
    let admission = NativeWorldShadowAdmission::new(world, sources, materials);
    prepare_world_shadow_masks_with_admission(
        owner,
        &admission,
        meshes,
        images,
        queue,
        masks,
        clip,
        local_time,
        clock_error,
    )
}

pub(super) fn prepare_world_shadow_masks_with_admission(
    owner: &NativeWorldShadowDrawSettings,
    admission: &NativeWorldShadowAdmission<'_>,
    meshes: &Assets<Mesh>,
    images: &Assets<Image>,
    queue: &mut super::scene_queue::ShadowTextureRequests<'_>,
    masks: &[Vec<u8>],
    clip: &render_shadow::ShadowMatrix,
    local_time: Option<[f32; 2]>,
    clock_error: Option<&str>,
) -> Result<Vec<NativeWorldShadowMaskPlan>, String> {
    let (world, sources, materials) = (admission.world, admission.sources, admission.materials);
    if sources.objects.len() != world.objects.len() {
        return Err("native BWM mesh/material owner count differs from world records".into());
    }
    if owner.dimensions.width == 0 || owner.dimensions.height == 0 {
        return Err("native whole-view shadow target has zero dimensions".into());
    }
    if clip.iter().flatten().any(|value| !value.is_finite())
        || owner.hardware_bias.iter().any(|value| !value.is_finite())
    {
        return Err("native whole-view projection or hardware bias is non-finite".into());
    }
    if queue.diffuse_image_handles.len() != queue.diffuse_image_paths.len() {
        return Err("native shadow diffuse image handles and paths have different domains".into());
    }

    let flags48 = admission.flags(owner.exclude_lod)?;

    let mut plans = Vec::with_capacity(masks.len());
    for (mask_index, mask) in masks.iter().enumerate() {
        let mut gathered = world
            .gather_native_shadow_packets(mask, flags48)
            .map_err(|error| format!("native whole-view mask {mask_index}: {error}"))?;
        gathered.sort_for_shadow_draw(owner.sort_shadows_ccw);
        for &id in &gathered.instance_ids {
            if id > 0x00ff_ffff || id as usize >= world.matrices.len() {
                return Err(format!(
                    "native whole-view mask {mask_index} contains invalid low-24-bit matrix ID {id}"
                ));
            }
            if world.matrices[id as usize]
                .iter()
                .any(|value| !value.is_finite())
            {
                return Err(format!(
                    "native whole-view matrix {id} contains a non-finite affine value"
                ));
            }
        }
        if gathered.auxiliary.iter().flatten().any(|word| *word != 0) {
            return Err(format!(
                "native whole-view mask {mask_index} auxiliary seed differs from the verified zero source"
            ));
        }

        let mut plan = NativeWorldShadowMaskPlan {
            instance_ids: gathered.instance_ids,
            auxiliary: gathered.auxiliary,
            instances: Vec::with_capacity(gathered.packets.len()),
        };
        for packet in gathered.packets {
            let count = packet
                .end
                .checked_sub(packet.start)
                .filter(|count| *count != 0)
                .ok_or_else(|| {
                    format!(
                        "native whole-view packet for object {} has an empty or reversed range",
                        packet.object
                    )
                })?;
            if packet.end as usize > plan.instance_ids.len() {
                return Err(format!(
                    "native whole-view packet for object {} exceeds its mask ID buffer",
                    packet.object
                ));
            }
            let object = world.objects.get(packet.object).ok_or_else(|| {
                format!(
                    "native whole-view packet references absent BWM object {}",
                    packet.object
                )
            })?;
            let source = sources.object(packet.object).ok_or_else(|| {
                format!(
                    "native whole-view packet object {} has no retained mesh",
                    packet.object
                )
            })?;
            let material = source.material.as_ref().ok_or_else(|| {
                source.material_error.clone().unwrap_or_else(|| {
                    format!(
                        "native whole-view caster object {} has no material owner",
                        packet.object
                    )
                })
            })?;
            let inputs = materials
                .0
                .get(material)
                .ok_or_else(|| {
                    format!(
                        "native whole-view caster object {} has no material inputs",
                        packet.object
                    )
                })?
                .as_ref()
                .map_err(|error| {
                    format!(
                        "native whole-view caster object {} material: {error}",
                        packet.object
                    )
                })?;
            let world_source = inputs
                .world_source
                .as_ref()
                .map_err(|error| {
                    format!(
                        "native world bank-2 source for object {}: {error}",
                        packet.object
                    )
                })?
                .as_ref()
                .ok_or_else(|| {
                    format!(
                        "native world bank-2 slot +0x250 absent for caster object {}",
                        packet.object
                    )
                })?;
            let class = world_source.class;
            let raw_vertex_count = *source.raw_vertex_count.as_ref().map_err(|error| {
                format!("native world source Mesh object {}: {error}", packet.object)
            })?;
            if raw_vertex_count == 0 {
                return Err(format!(
                    "native world caster object {} has no source vertices",
                    packet.object
                ));
            }
            let source_mesh = meshes.get(&source.mesh).ok_or_else(|| {
                format!(
                    "native world source Mesh object {} is unavailable",
                    packet.object
                )
            })?;
            if source_mesh.count_vertices() != raw_vertex_count
                || source_mesh.indices().is_none()
                || source_mesh
                    .attribute(shadow_pass::NATIVE_VEGETATION_POSITION)
                    .is_none()
            {
                return Err(format!(
                    "native world source Mesh object {} lacks its indexed source-position ABI",
                    packet.object
                ));
            }
            if matches!(
                class,
                shader_cache::CachedWorldShadowClass::Vegetation
                    | shader_cache::CachedWorldShadowClass::VegetationFixed
            ) && (source_mesh
                .attribute(shadow_pass::NATIVE_VEGETATION_NORMAL)
                .is_none()
                || source_mesh
                    .attribute(shadow_pass::NATIVE_VEGETATION_COLOR)
                    .is_none())
            {
                return Err(format!(
                    "native world vegetation Mesh object {} lacks authored NORMAL/COLOR channels",
                    packet.object
                ));
            }
            let (program, constants, alpha_material) = match class {
                shader_cache::CachedWorldShadowClass::Opaque => (
                    shadow_pass::NativeShadowProgram::WorldOpaque,
                    NativeWorldShadowPreparedConstants::World {
                        bytes: shadow_pass::native_world_shadow_view_cb(clip, packet.start),
                        instance_count: count,
                    },
                    None,
                ),
                shader_cache::CachedWorldShadowClass::AlphaTested
                | shader_cache::CachedWorldShadowClass::VistaAlpha
                | shader_cache::CachedWorldShadowClass::Vegetation
                | shader_cache::CachedWorldShadowClass::VegetationFixed => {
                    let params = inputs
                        .world_parameters
                        .as_ref()
                        .map_err(|error| {
                            format!(
                                "native world parameters for object {}: {error}",
                                packet.object
                            )
                        })?
                        .as_ref()
                        .ok_or_else(|| {
                            format!(
                                "native world material parameters are absent for object {}",
                                packet.object
                            )
                        })?;
                    let uv_scale_bias =
                        params.vector(b"layer0_scalebias", None).map_err(|error| {
                            format!(
                                "native Layer0 UV parameters for object {}: {error}",
                                packet.object
                            )
                        })?;
                    if uv_scale_bias.iter().any(|value| !value.is_finite()) {
                        return Err(format!(
                            "native Layer0 UV parameters for object {} are non-finite",
                            packet.object
                        ));
                    }
                    let uv_set = if class == shader_cache::CachedWorldShadowClass::VistaAlpha {
                        // The installed VistaAlpha fingerprint uses raw TEXCOORD0.
                        0
                    } else {
                        inputs.world_uv_set.ok_or_else(|| format!(
                            "native world alpha material object {} has no resolved Layer0 UV selector", packet.object
                        ))?
                    };
                    if uv_set > 1 {
                        return Err(format!(
                            "native world alpha material object {} selects unsupported UV set {uv_set}",
                            packet.object
                        ));
                    }
                    let uv_attribute = if uv_set == 0 {
                        Mesh::ATTRIBUTE_UV_0
                    } else {
                        Mesh::ATTRIBUTE_UV_1
                    };
                    if source_mesh.attribute(uv_attribute).is_none() {
                        return Err(format!(
                            "native world alpha Mesh object {} lacks authored UV set {uv_set}",
                            packet.object
                        ));
                    }
                    let slot = inputs.diffuse_image_slot.ok_or_else(|| {
                        format!(
                            "native world alpha material object {} has no diffuse image slot",
                            packet.object
                        )
                    })?;
                    if slot >= queue.diffuse_image_paths.len() {
                        return Err(format!(
                            "native world alpha material object {} references absent diffuse slot {slot}",
                            packet.object
                        ));
                    }
                    queue.request_shadow_diffuse(slot);
                    let path = &queue.diffuse_image_paths[slot];
                    let diffuse = queue.diffuse_image_handles.get(slot)
                        .and_then(Option::as_ref)
                        .ok_or_else(|| format!("native whole-view alpha image pending through texture streamer: slot {slot} ({path})"))?;
                    if images.get(diffuse).is_none() {
                        return Err(format!(
                            "native whole-view alpha image asset is unavailable: slot {slot} ({path})"
                        ));
                    }
                    let alpha_material = shadow_pass::NativeShadowAlphaMaterial {
                        diffuse: diffuse.clone(),
                        uv_set,
                        uv_scale_bias,
                    };
                    match class {
                        shader_cache::CachedWorldShadowClass::AlphaTested => (
                            shadow_pass::NativeShadowProgram::WorldAlphaTested,
                            NativeWorldShadowPreparedConstants::World {
                                bytes: shadow_pass::native_world_alpha_shadow_view_cb(
                                    clip,
                                    packet.start,
                                    uv_scale_bias,
                                ),
                                instance_count: count,
                            },
                            Some(alpha_material),
                        ),
                        shader_cache::CachedWorldShadowClass::VistaAlpha => (
                            shadow_pass::NativeShadowProgram::WorldVistaAlpha,
                            NativeWorldShadowPreparedConstants::World {
                                bytes: shadow_pass::native_world_alpha_shadow_view_cb(
                                    clip,
                                    packet.start,
                                    uv_scale_bias,
                                ),
                                instance_count: count,
                            },
                            Some(alpha_material),
                        ),
                        shader_cache::CachedWorldShadowClass::Vegetation
                        | shader_cache::CachedWorldShadowClass::VegetationFixed => {
                            let time = local_time.ok_or_else(|| format!(
                                "native world vegetation `$localTime` unavailable for source flag+0x04={}; {}",
                                owner.source_flag_04,
                                clock_error.unwrap_or("native gameplay-view clock is absent")
                            ))?;
                            if time.iter().any(|value| !value.is_finite()) {
                                return Err(
                                    "native world vegetation `$localTime` is non-finite".into()
                                );
                            }
                            let wind = params.vegetation(None).map_err(|error| {
                                format!(
                                    "native vegetation parameters for object {}: {error}",
                                    packet.object
                                )
                            })?;
                            let vectors = [
                                wind.main_variation,
                                wind.main_frequency,
                                wind.main_amplitude,
                                wind.minimum_wind,
                                wind.leaf_frequency,
                                wind.leaf_amplitude,
                                wind.branch_amplitude,
                            ];
                            if vectors.iter().flatten().any(|value| !value.is_finite()) {
                                return Err(format!(
                                    "native vegetation parameters for object {} are non-finite",
                                    packet.object
                                ));
                            }
                            let uses_wind_buffer =
                                class == shader_cache::CachedWorldShadowClass::Vegetation;
                            let wind_offset = if uses_wind_buffer {
                                let offset = packet.auxiliary_start.ok_or_else(|| format!(
                                    "native wind-buffer shader object {} has no gathered auxiliary offset", packet.object
                                ))?;
                                let end = usize::from(offset)
                                    .checked_add(count as usize)
                                    .ok_or("native wind auxiliary range overflow")?;
                                if end > plan.auxiliary.len() {
                                    return Err(format!(
                                        "native wind auxiliary range {offset}..{end} exceeds mask buffer",
                                    ));
                                }
                                u32::from(offset)
                            } else {
                                u32::from(packet.auxiliary_start.unwrap_or(0))
                            };
                            if uses_wind_buffer && plan.auxiliary.is_empty() {
                                return Err(format!(
                                    "native wind-buffer shader object {} has no auxiliary buffer",
                                    packet.object
                                ));
                            }
                            (
                                if uses_wind_buffer {
                                    shadow_pass::NativeShadowProgram::WorldVegetation
                                } else {
                                    shadow_pass::NativeShadowProgram::WorldVegetationFixed
                                },
                                NativeWorldShadowPreparedConstants::Vegetation {
                                    bytes: shadow_pass::native_world_vegetation_shadow_view_cb(
                                        clip,
                                        uv_scale_bias,
                                        &wind,
                                        time,
                                        packet.start,
                                        wind_offset,
                                    ),
                                    instance_count: count,
                                    uses_wind_buffer,
                                },
                                Some(alpha_material),
                            )
                        }
                        shader_cache::CachedWorldShadowClass::Opaque => unreachable!(),
                    }
                }
            };
            let main_render_state = inputs.source.main_render_state.ok_or_else(|| {
                format!(
                    "native main material draw state +0x2c8 is absent for caster object {}",
                    packet.object
                )
            })?;
            let state = render_state::world_shadow_draw_state(
                main_render_state,
                u8::from(object.flipped),
                owner.hardware_bias[0],
                owner.hardware_bias[1],
                owner.hardware_bias[2],
                owner.depth_clip,
            );
            let (front_face, cull_mode) =
                shadow_pipeline::native_shadow_mesh_raster(&state, !source.flipped);
            plan.instances.push(NativeWorldShadowPreparedInstance {
                mesh: source.mesh.clone(),
                constants,
                program,
                alpha_material,
                front_face,
                cull_mode,
                draw_state: state,
            });
        }
        plans.push(plan);
    }
    Ok(plans)
}

pub(super) fn build_native_whole_view_layer(
    plans: Vec<NativeWorldShadowMaskPlan>,
    world: &world_mesh::WorldMesh,
    dimensions: shadow_pass::NativeParallelShadowTargetDimensions,
    shader_buffers: &mut Assets<ShaderBuffer>,
) -> shadow_pass::NativeShadowLayer {
    let has_instances = plans.iter().any(|plan| !plan.instances.is_empty());
    let matrices =
        has_instances.then(|| shader_buffers.add(ShaderBuffer::from(world.matrices.clone())));
    build_native_world_layer(plans, matrices.as_ref(), dimensions, shader_buffers)
}

/// Upload slots for a cascade. Live slots are mutable; retained generations
/// use separate slots and share storage only when every encoded byte matches.
#[derive(Default)]
pub(super) struct WorldShadowUpload {
    ids: Option<(Handle<ShaderBuffer>, usize)>,
    winds: Option<(Handle<ShaderBuffer>, usize)>,
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    #[test]
    fn immutable_shadow_upload_shares_only_identical_bytes_and_preserves_old_generations() {
        let mut buffers = Assets::<ShaderBuffer>::default();
        let mut slot = None;
        let first = reuse_immutable_shadow_values(vec![7u32, 11, 13], &mut slot, &mut buffers);
        let original = buffers.get(&first).unwrap().data.clone();
        let same = reuse_immutable_shadow_values(vec![7u32, 11, 13], &mut slot, &mut buffers);
        assert_eq!(first.id(), same.id());
        assert_eq!(
            original,
            ShaderBuffer::from(vec![7u32, 11, 13]).data,
            "immutable uploads keep their exact length"
        );
        let changed = reuse_immutable_shadow_values(vec![19u32, 23, 29], &mut slot, &mut buffers);
        assert_ne!(changed.id(), first.id());
        assert_eq!(buffers.get(&first).unwrap().data, original);
        let changed_bytes = buffers.get(&changed).unwrap().data.clone();
        let grown = reuse_immutable_shadow_values(vec![31u32; 5], &mut slot, &mut buffers);
        assert_ne!(grown.id(), changed.id());
        assert_eq!(buffers.get(&changed).unwrap().data, changed_bytes);
        buffers.get_mut(&grown).unwrap().set_data(vec![99u32; 5]);
        let repaired = reuse_immutable_shadow_values(vec![31u32; 5], &mut slot, &mut buffers);
        assert_ne!(
            grown.id(),
            repaired.id(),
            "another writer invalidates reuse without overwriting its asset"
        );
        assert_eq!(
            buffers.get(&grown).unwrap().data,
            ShaderBuffer::from(vec![99u32; 5]).data
        );
    }

    #[test]
    fn live_shadow_upload_reuses_capacity_and_replaces_every_value() {
        let mut buffers = Assets::<ShaderBuffer>::default();
        let mut cached = None;
        let first = upload_shadow_values(vec![7u32, 11, 13], &mut cached, &mut buffers);
        assert_eq!(
            buffers.get(&first).unwrap().data.as_ref().unwrap(),
            ShaderBuffer::from(vec![7u32, 11, 13, 0])
                .data
                .as_ref()
                .unwrap()
        );
        let data_pointer = buffers.get(&first).unwrap().data.as_ref().unwrap().as_ptr();
        let unchanged = upload_shadow_values(vec![7u32, 11, 13], &mut cached, &mut buffers);
        assert_eq!(unchanged.id(), first.id());
        assert_eq!(
            buffers
                .get(&unchanged)
                .unwrap()
                .data
                .as_ref()
                .unwrap()
                .as_ptr(),
            data_pointer
        );
        // Another producer changing the actual asset must invalidate reuse,
        // even when this caller submits the same values as last time.
        buffers.get_mut(&first).unwrap().set_data(vec![99u32; 4]);
        let restored = upload_shadow_values(vec![7u32, 11, 13], &mut cached, &mut buffers);
        assert_eq!(
            buffers.get(&restored).unwrap().data,
            ShaderBuffer::from(vec![7u32, 11, 13, 0]).data
        );
        let next = upload_shadow_values(vec![19u32, 23], &mut cached, &mut buffers);
        assert_eq!(first.id(), next.id());
        assert_eq!(
            buffers.get(&next).unwrap().data.as_ref().unwrap(),
            ShaderBuffer::from(vec![19u32, 23, 0, 0])
                .data
                .as_ref()
                .unwrap()
        );
        let grown = upload_shadow_values(vec![29u32; 5], &mut cached, &mut buffers);
        assert_ne!(grown.id(), next.id());
        assert_eq!(
            buffers.get(&next).unwrap().data.as_ref().unwrap(),
            ShaderBuffer::from(vec![19u32, 23, 0, 0])
                .data
                .as_ref()
                .unwrap()
        );
        assert_eq!(cached.as_ref().unwrap().1, 8);
    }
}

fn upload_shadow_values<T: Default + Clone>(
    mut values: Vec<T>,
    cached: &mut Option<(Handle<ShaderBuffer>, usize)>,
    buffers: &mut Assets<ShaderBuffer>,
) -> Handle<ShaderBuffer>
where
    Vec<T>: bevy::render::render_resource::ShaderType
        + bevy::render::render_resource::encase::internal::WriteInto,
{
    let required = values.len().max(1).next_power_of_two();
    let capacity = cached
        .as_ref()
        .map_or(required, |(_, capacity)| (*capacity).max(required));
    values.resize(capacity, T::default());
    let mut encoded = ShaderBuffer::default();
    encoded.set_data(values);
    if let Some((handle, old_capacity)) = cached {
        if *old_capacity == capacity {
            if let Some(buffer) = buffers.get(&*handle) {
                // An unchanged remap/wind buffer needs no asset event, extract
                // clone or GPU upload. Compare every encoded byte; never infer
                // equality from a stationary camera or matching capacity.
                if buffer.data == encoded.data {
                    return handle.clone();
                }
                buffers.get_mut(&*handle).unwrap().data = encoded.data;
                return handle.clone();
            }
        }
    }
    let handle = buffers.add(encoded);
    *cached = Some((handle.clone(), capacity));
    handle
}

/// Share identical immutable generation data; changed data gets a new asset
/// so every older retained snapshot keeps its original bytes.
fn reuse_immutable_shadow_values<T>(
    values: Vec<T>,
    cached: &mut Option<(Handle<ShaderBuffer>, usize)>,
    buffers: &mut Assets<ShaderBuffer>,
) -> Handle<ShaderBuffer>
where
    Vec<T>: bevy::render::render_resource::ShaderType
        + bevy::render::render_resource::encase::internal::WriteInto,
{
    let length = values.len();
    let encoded = ShaderBuffer::from(values);
    if let Some((handle, _)) = cached {
        if buffers
            .get(&*handle)
            .is_some_and(|buffer| buffer.data == encoded.data)
        {
            return handle.clone();
        }
    }
    let handle = buffers.add(encoded);
    *cached = Some((handle.clone(), length));
    handle
}

pub(super) fn build_native_world_layer(
    plans: Vec<NativeWorldShadowMaskPlan>,
    matrices: Option<&Handle<ShaderBuffer>>,
    dimensions: shadow_pass::NativeParallelShadowTargetDimensions,
    shader_buffers: &mut Assets<ShaderBuffer>,
) -> shadow_pass::NativeShadowLayer {
    build_world_shadow_layer(plans, matrices, dimensions, shader_buffers, None, false)
}

pub(super) fn build_world_shadow_layer(
    plans: Vec<NativeWorldShadowMaskPlan>,
    matrices: Option<&Handle<ShaderBuffer>>,
    dimensions: shadow_pass::NativeParallelShadowTargetDimensions,
    shader_buffers: &mut Assets<ShaderBuffer>,
    mut uploads: Option<&mut Vec<WorldShadowUpload>>,
    immutable_uploads: bool,
) -> shadow_pass::NativeShadowLayer {
    let mut instances = Vec::new();
    for (index, plan) in plans.into_iter().enumerate() {
        if plan.instances.is_empty() {
            continue;
        }
        let upload = uploads.as_deref_mut().map(|uploads| {
            if uploads.len() <= index {
                uploads.resize_with(index + 1, WorldShadowUpload::default);
            }
            &mut uploads[index]
        });
        let (visible_indices, winds) = if let Some(upload) = upload {
            let ids = if immutable_uploads {
                reuse_immutable_shadow_values(plan.instance_ids, &mut upload.ids, shader_buffers)
            } else {
                upload_shadow_values(plan.instance_ids, &mut upload.ids, shader_buffers)
            };
            let winds = (!plan.auxiliary.is_empty()).then(|| {
                if immutable_uploads {
                    reuse_immutable_shadow_values(plan.auxiliary, &mut upload.winds, shader_buffers)
                } else {
                    upload_shadow_values(plan.auxiliary, &mut upload.winds, shader_buffers)
                }
            });
            (ids, winds)
        } else {
            (
                shader_buffers.add(ShaderBuffer::from(plan.instance_ids)),
                (!plan.auxiliary.is_empty())
                    .then(|| shader_buffers.add(ShaderBuffer::from(plan.auxiliary))),
            )
        };
        let matrices = matrices
            .expect("nonempty native world has matrices")
            .clone();
        for prepared in plan.instances {
            let per_instance_cb = match prepared.constants {
                NativeWorldShadowPreparedConstants::World {
                    bytes,
                    instance_count,
                } => shadow_pass::NativeShadowInstanceConstants::World {
                    constants: bytes,
                    visible_indices: visible_indices.clone(),
                    matrices: matrices.clone(),
                    instance_count,
                },
                NativeWorldShadowPreparedConstants::Vegetation {
                    bytes,
                    instance_count,
                    uses_wind_buffer,
                } => shadow_pass::NativeShadowInstanceConstants::WorldVegetation {
                    constants: bytes,
                    visible_indices: visible_indices.clone(),
                    matrices: matrices.clone(),
                    winds: if uses_wind_buffer {
                        Some(
                            winds
                                .as_ref()
                                .expect("preflight validated native wind buffer")
                                .clone(),
                        )
                    } else {
                        None
                    },
                    instance_count,
                },
            };
            instances.push(shadow_pass::NativeShadowInstance {
                mesh: prepared.mesh,
                per_instance_cb,
                program: prepared.program,
                alpha_material: prepared.alpha_material,
                front_face: prepared.front_face,
                cull_mode: prepared.cull_mode,
                draw_state: prepared.draw_state,
                stencil_reference: 0,
                scissor: shadow_pipeline::NativeShadowScissor {
                    x: 0,
                    y: 0,
                    width: dimensions.width,
                    height: dimensions.height,
                },
            });
        }
    }
    shadow_pass::NativeShadowLayer {
        ops: shadow_pass::NativeShadowLayerOps {
            depth: shadow_pass::NativeShadowLoad::Clear(1.0),
            stencil: shadow_pass::NativeStencilLoad::Clear(0),
        },
        cache: shadow_pass::NativeShadowCacheOp::None,
        instances,
    }
}

pub(super) fn publish_initial_native_whole_view_depth(
    owner: Option<Res<NativeParallelShadowOwner>>,
    scene: Option<Res<SceneData>>,
    sources: Option<Res<NativeWorldShadowMeshSources>>,
    materials: Option<Res<NativeShadowMaterials>>,
    meshes: Option<Res<Assets<Mesh>>>,
    images: Option<Res<Assets<Image>>>,
    gameplay_clock: Option<Res<game_clock_runtime::NativeGameplayViewClock>>,
    queue: Option<Res<SceneRenderQueue>>,
    mut commands: Commands,
    mut shader_buffers: Option<ResMut<Assets<ShaderBuffer>>>,
    completion: Option<Res<shadow_pass::NativeShadowSceneCompletion>>,
    mut frame: ResMut<shadow_pass::NativeShadowSceneFrame>,
    mut last_error: Local<Option<String>>,
) {
    if matches!(
        &frame.state,
        shadow_pass::NativeShadowSceneState::WholeViewDepth { .. }
            | shadow_pass::NativeShadowSceneState::Ready { .. }
    ) {
        return;
    }
    let Some(owner) = owner else { return };
    if completion
        .as_ref()
        .is_some_and(|done| done.whole_view_completed(&owner.target_image, owner.dimensions))
    {
        return;
    }
    let revision = match frame.revision.checked_add(1) {
        Some(revision) => revision,
        None => {
            let error = "native whole-view frame revision overflow".to_string();
            if last_error.as_ref() != Some(&error) {
                eprintln!("Native whole-view depth unavailable: {error}");
                *last_error = Some(error.clone());
            }
            frame.state = shadow_pass::NativeShadowSceneState::Unavailable(error);
            return;
        }
    };
    let (local_time, clock_error) = match gameplay_clock {
        Some(clock) => match clock.view_shader_times(owner.source_flag_04) {
            Ok(times) => (Some(times.local_time), None),
            Err(error) => (None, Some(error)),
        },
        None => (
            None,
            Some("native gameplay-view clock resource is unavailable".into()),
        ),
    };
    let attempt = (|| -> Result<shadow_pass::NativeShadowLayer, String> {
        let scene = scene
            .as_deref()
            .ok_or("native SceneData dependency is unavailable")?;
        let sources = sources
            .as_deref()
            .ok_or("native world mesh owners are unavailable")?;
        let materials = materials
            .as_deref()
            .ok_or("native shadow material inputs are unavailable")?;
        let meshes = meshes
            .as_deref()
            .ok_or("native Mesh assets are unavailable")?;
        let images = images
            .as_deref()
            .ok_or("native Image assets are unavailable")?;
        let queue = queue
            .as_deref()
            .ok_or("native render queue is unavailable")?;
        let shader_buffers = shader_buffers
            .as_deref_mut()
            .ok_or("native ShaderBuffer asset table is unavailable")?;
        let mut shadow_textures = super::scene_queue::ShadowTextureRequests::new(queue);
        let plans = prepare_native_whole_view_masks(
            &owner,
            &scene.world,
            sources,
            materials,
            meshes,
            images,
            &mut shadow_textures,
            local_time,
            clock_error.as_deref(),
        );
        shadow_textures.submit(&mut commands);
        let plans = plans?;
        Ok(build_native_whole_view_layer(
            plans,
            &scene.world,
            owner.dimensions,
            shader_buffers,
        ))
    })();
    match attempt {
        Ok(layer) => {
            frame.revision = revision;
            frame.state = shadow_pass::NativeShadowSceneState::WholeViewDepth {
                target_image: owner.target_image.clone(),
                dimensions: owner.dimensions,
                layer,
            };
            *last_error = None;
        }
        Err(error) => {
            let message = format!("native initial whole-view depth: {error}");
            if last_error.as_ref() != Some(&message) {
                eprintln!("Native whole-view depth unavailable: {error}");
                *last_error = Some(message.clone());
            }
            frame.state = shadow_pass::NativeShadowSceneState::Unavailable(message);
        }
    }
}

pub(super) fn initialize_native_parallel_shadow_owner(
    mut commands: Commands,
    scene: Res<SceneData>,
    mut images: ResMut<Assets<Image>>,
    mut frame: ResMut<shadow_pass::NativeShadowSceneFrame>,
) {
    match NativeParallelShadowOwner::from_scene(&scene, &mut images) {
        Ok(owner) => {
            println!(
                "Native parallel shadow target: {}x{}, {} layers; whole-view projection and environment bias resolved",
                owner.dimensions.width, owner.dimensions.height, owner.dimensions.layers
            );
            commands.insert_resource(owner);
            frame.state = shadow_pass::NativeShadowSceneState::Unavailable(
                "native target owner initialized; initial native whole-view world depth publication pending".into(),
            );
        }
        Err(error) => {
            eprintln!("Native parallel shadow owner unavailable: {error}");
            commands.remove_resource::<NativeParallelShadowOwner>();
            frame.state = shadow_pass::NativeShadowSceneState::Unavailable(error);
        }
    }
}

pub(super) fn native_shadow_target_dimensions(
    slice_size: u32,
    quality: &entities::ShadowQualitySettings,
) -> Result<shadow_pass::NativeParallelShadowTargetDimensions, String> {
    let target = render_shadow::ParallelShadowTargetDimensions::from_settings(
        slice_size,
        quality.parallel_resolution_divider,
        quality.parallel_slice_count,
    )?;
    Ok(shadow_pass::NativeParallelShadowTargetDimensions {
        width: target.side,
        height: target.side,
        layers: target.layers,
        depth_32_bits: quality.depth_32_bits,
    })
}

/// Changing the atlas association is rare; moving the camera only updates the
/// shared receiver buffer, not every material or its bind group.
pub(super) fn sync_native_shadow_target<E: MaterialExtension + NativeShadowImageAccess>(
    frame: Res<shadow_pass::NativeShadowSceneFrame>,
    local_owner: Option<Res<local_shadow_updates::NativeLocalShadowOwner>>,
    sampling: Option<Res<shadow_sampling::NativeParallelSamplingBuffer>>,
    screen: Option<Res<screen_shadow_pass::NativeScreenShadowFrame>>,
    mut materials: ResMut<Assets<ExtendedMaterial<StandardMaterial, E>>>,
    mut added_materials: MessageReader<
        bevy::asset::AssetEvent<ExtendedMaterial<StandardMaterial, E>>,
    >,
    mut current: Local<
        Option<(
            Option<Handle<Image>>,
            Option<Handle<Image>>,
            Option<Handle<Image>>,
            Option<Handle<Image>>,
        )>,
    >,
) {
    let Some(sampling) = sampling else {
        return;
    };
    let target = match &frame.state {
        shadow_pass::NativeShadowSceneState::Ready { target_image, .. } => {
            Some(target_image.clone())
        }
        _ => Some(sampling.unavailable_depth.clone()),
    };
    let cloud = screen.as_ref().map(|s| s.cloud.clone());
    let screen = screen.map(|s| s.output.clone());
    let local_target = Some(
        local_owner
            .map(|o| o.atlas_image.clone())
            .unwrap_or_else(|| sampling.unavailable_depth.clone()),
    );
    let association = Some((
        target.clone(),
        screen.clone(),
        cloud.clone(),
        local_target.clone(),
    ));
    if *current == association {
        // New streamed materials need the current atlas even when the owner
        // hasn't changed targets. Process additions, not every material/frame.
        for event in added_materials.read() {
            if let bevy::asset::AssetEvent::Added { id } = event {
                if let Some(mut material) = materials.get_mut(*id) {
                    material
                        .extension
                        .sync_shadow_images(&target, &screen, &cloud, &local_target);
                }
            }
        }
        return;
    }
    added_materials.clear();
    for (_, material) in materials.iter_mut() {
        material
            .extension
            .sync_shadow_images(&target, &screen, &cloud, &local_target);
    }
    *current = association;
}
