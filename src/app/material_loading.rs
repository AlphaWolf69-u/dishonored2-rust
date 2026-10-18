//! Material loading.
use super::{
    archive, entities, env, material, material_surface, probe_lighting, render_program,
    render_state, shader_cache, surface_gpu, CachedMaterial, DecalExtension, DecalMaterial,
    HashMap, MaterialHandle, NativeShadowMaterialInputs, NativeShadowMaterialParameters,
    NativeShadowMaterials, ProbeExtension, ProbeMaterial, ResolvedMaterial, ShaderBuffer,
};
use bevy::prelude::*;

/// Resolve the slot-admission fields from the same dictionaries as 1402d5210.
/// In particular +0x48 bit4 is MAIN-fallback isDecal, not invisible. This is
/// source selection only, not a claim that all other material flags are loaded.
pub(super) fn resolve_native_shadow_source_inputs(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    program: &[u8],
    main_program: Option<&str>,
) -> Result<material::ShadowSourceInputs, String> {
    let mut source = material::shadow_source_inputs(bytes, program)?;
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let main = main_program.ok_or("shadow source owner has no main program")?;
    let state = library
        .definition(main.as_bytes())?
        .ok_or_else(|| format!("missing native main program {main}"))?
        .native_render_state();
    // 1402d662d passes all nine MAIN words to 1402c3bc0. A line scan
    // misses compact state blocks and can mistake nested option names for
    // drawing commands, changing native shadow-bank admission.
    let material_state =
        render_state::material_render_state(&String::from_utf8_lossy(bytes), state)?;
    source.main_blend_state = material_state[0];
    source.main_render_state = Some(material_state);
    // These constructor fields precede per-pass specialization. Read the
    // material dictionary, not a block already overwritten for shadow slot 10.
    let overrides = native_material_option_overrides(library, bytes)?;
    let no_shadow = library.registered_option_value(&overrides, b"noShadow")? != 0;
    let is_decal = library.option_value(main.as_bytes(), &overrides, b"isDecal")? & 1 != 0;
    source.hair_type = library.option_value(main.as_bytes(), &overrides, b"hairType")? & 1 != 0;
    source.sky_type = library.option_value(main.as_bytes(), &overrides, b"skyType")? as u32 & 3;
    source.is_cloth = library.option_value(main.as_bytes(), &overrides, b"isCloth")? & 1 != 0;
    source.needs_wind = library.option_value(main.as_bytes(), &overrides, b"needsWind")? & 1 != 0;
    source.source = material::shadow_program_source(
        false,
        no_shadow,
        is_decal,
        source.override_program.is_some(),
        source.hair_type,
        source.main_blend_state,
        source.is_cloth,
    );
    source.cloth_source = material::shadow_program_source(
        true,
        no_shadow,
        is_decal,
        source.cloth_override_program.is_some(),
        source.hair_type,
        source.main_blend_state,
        source.is_cloth,
    );
    // 1402d6e59..1402d6f39 subsequently clears rigid slot 10 in ALL three
    // banks. An authored shadow override does not bypass this final stage.
    // MAIN resolved above; a HasParm miss is the native implicit-main path.
    let class = material::material_program_class(
        true,
        material::authored_program_override(bytes, "mainProgram").is_none(),
        source.main_blend_state,
        library.option_value(main.as_bytes(), &overrides, b"invisible")? != 0,
    );
    if material::finalize_material_programs(class, source.hair_type, source.sky_type).clear_slots
        & (1 << 10)
        != 0
    {
        source.source = material::ShadowProgramSource::Disabled;
    }
    Ok(source)
}

/// Native material +0x48 bit0 is the finalized base bank's slot10 pointer,
/// not the world bank2 pointer and not the subset of shader ABIs we support.
/// All current level sources are resident; a missing cache row still requires
/// the original runtime compile path and must not become `false` here.
pub(super) fn resolve_native_shadow_slot_presence(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
) -> Result<bool, String> {
    let name = match source.source {
        material::ShadowProgramSource::Disabled => return Ok(false),
        material::ShadowProgramSource::Authored => source.override_program.as_deref(),
        material::ShadowProgramSource::MainProgram => main_program,
    }
    .ok_or("shadow slot owner has no resolved program name")?;
    if name.is_empty() {
        return Ok(false);
    }
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let overrides = native_shadow_variant_overrides(library, bytes, source, main_program, name)?;
    if library
        .serialized_variant(name.as_bytes(), &overrides)?
        .is_none()
    {
        return Err(format!(
            "native base shadow slot needs runtime compilation: {name}"
        ));
    }
    Ok(true)
}

pub(super) fn resolve_native_rigid_shadow_source(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
) -> Result<Option<std::sync::Arc<shader_cache::CachedRigidShadowSource>>, String> {
    let name = match source.source {
        material::ShadowProgramSource::Disabled => return Ok(None),
        material::ShadowProgramSource::Authored => source.override_program.as_deref(),
        material::ShadowProgramSource::MainProgram => main_program,
    }
    .ok_or("shadow source has no resolved program name")?;
    // HasParm with an authored null source is not a request for the main shader.
    if name.is_empty() {
        return Ok(None);
    }
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let overrides = native_shadow_variant_overrides(library, bytes, source, main_program, name)?;
    library
        .rigid_shadow_source(name.as_bytes(), &overrides)?
        .map(Some)
        .ok_or_else(|| format!("no resident rigid shadow variant for {name}"))
}

/// FUN_1402d5210 bank2 at 1402d66a0..1402d66f0. The direct-instancing
/// exclusion is read BEFORE pass overrides and uses registered defaults,
/// not a MAIN-fallback value. Resident absence is an error, not "no shadow".
pub(super) fn resolve_native_world_shadow_source(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
) -> Result<Option<std::sync::Arc<shader_cache::CachedWorldShadowSource>>, String> {
    let name = match source.source {
        material::ShadowProgramSource::Disabled => return Ok(None),
        material::ShadowProgramSource::Authored => source.override_program.as_deref(),
        material::ShadowProgramSource::MainProgram => main_program,
    }
    .ok_or("world shadow source has no resolved program name")?;
    if name.is_empty() {
        return Ok(None);
    }
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let original = native_material_option_overrides(library, bytes)?;
    let multi = library.registered_option_value(&original, b"useMultiInstancing")? != 0;
    let direct = library.registered_option_value(&original, b"useDirectInstancing")? != 0;
    if material::instancing_bank_override(2, multi, direct)?.is_none() {
        return Ok(None);
    }
    let mut overrides =
        native_shadow_variant_overrides(library, bytes, source, main_program, name)?;
    let value = render_program::SerializedParameterValue::Words([1, 0, 0, 0]);
    if let Some(parameter) = overrides
        .parameters
        .iter_mut()
        .find(|p| p.name.eq_ignore_ascii_case(b"useMultiInstancing"))
    {
        parameter.value = value;
    } else {
        overrides
            .parameters
            .push(render_program::SerializedParameter {
                name: b"useMultiInstancing".to_vec(),
                kind: render_program::RenderParameterKind::Option,
                value,
            });
    }
    library
        .world_shadow_source(name.as_bytes(), &overrides)?
        .map(Some)
        .ok_or_else(|| format!("no resident world shadow variant for {name}"))
}

pub(super) fn resolve_native_shadow_material_parameters(
    library: &mut shader_cache::NativeProgramLibrary,
    catalog: &archive::Catalog,
    bytes: &[u8],
    class: shader_cache::CachedRigidShadowClass,
    model_depth: bool,
) -> Result<NativeShadowMaterialParameters, String> {
    use shader_cache::CachedRigidShadowClass;
    let names: &[&[u8]] = match class {
        CachedRigidShadowClass::Opaque | CachedRigidShadowClass::VistaAlpha => &[],
        CachedRigidShadowClass::AlphaTested => &[b"layer0_scalebias"],
        CachedRigidShadowClass::Vegetation => &[
            b"layer0_scalebias",
            b"vegetanim/mainbendingvar",
            b"vegetanim/mainbendingfreq",
            b"vegetanim/mainbendingamplitude",
            b"vegetanim/leafminwindstrength",
            b"vegetanim/leaffreq",
            b"vegetanim/leafamplitude",
            b"vegetanim/branchamplitude",
        ],
    };
    let mut names = names.to_vec();
    if model_depth {
        names.push(b"modeldepthhack");
    }
    let values =
        entities::material_float_parameters(&entities::native_declaration_text(bytes), |name| {
            names.iter().any(|required| *required == name.as_bytes())
        })?;
    let overrides = render_program::SerializedParameterBlock {
        flags: 0,
        parameters: values
            .into_iter()
            .map(|(name, words)| render_program::SerializedParameter {
                name: name.into_bytes(),
                kind: render_program::RenderParameterKind::Uniform,
                value: render_program::SerializedParameterValue::Words(words),
            })
            .collect(),
    };
    let empty = render_program::SerializedParameterBlock {
        flags: 0,
        parameters: Vec::new(),
    };
    let defaults = names
        .iter()
        .map(|name| {
            library
                .registered_float_vector(catalog, &empty, name, None)
                .map(|words| (name.to_vec(), words))
        })
        .collect::<Result<_, _>>()?;
    Ok(NativeShadowMaterialParameters {
        overrides,
        defaults,
    })
}

pub(super) fn native_material_option_overrides(
    library: &shader_cache::NativeProgramLibrary,
    bytes: &[u8],
) -> Result<render_program::SerializedParameterBlock, String> {
    let values = material::material_option_parameters(bytes, |name| {
        library.parameter_kinds.get(name.as_bytes())
            == Some(&render_program::RenderParameterKind::Option)
    })?;
    Ok(native_option_block(values))
}

pub(super) fn native_option_block(
    values: Vec<(String, i32)>,
) -> render_program::SerializedParameterBlock {
    render_program::SerializedParameterBlock {
        flags: 0,
        parameters: values
            .into_iter()
            .map(|(name, value)| render_program::SerializedParameter {
                name: name.into_bytes(),
                kind: render_program::RenderParameterKind::Option,
                value: render_program::SerializedParameterValue::Words([value as u32, 0, 0, 0]),
            })
            .collect(),
    }
}

pub(super) fn native_shadow_variant_overrides(
    library: &mut shader_cache::NativeProgramLibrary,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
    selected_program: &str,
) -> Result<render_program::SerializedParameterBlock, String> {
    let original = native_material_option_overrides(library, bytes)?;
    // Material +0x4b bit4 uses MAIN, even with an authored shadow source.
    let main = main_program.ok_or("shadow alpha-test owner has no main program")?;
    let alpha_test = library.option_value(main.as_bytes(), &original, b"alphaTest")?;
    // isDecal uses MAIN, but decalType is material/registered-default only.
    let is_decal = library.option_value(main.as_bytes(), &original, b"isDecal")? & 1 != 0;
    let decal_type = library.registered_option_value(&original, b"decalType")? as u32;
    let is_alpha = material::shadow_is_alpha(
        source.main_blend_state,
        is_decal,
        decal_type,
        source.sky_type,
    );
    // 1402d68bb..1402d697d: R15B starts as isAlpha, then ORs three
    // nonzero tests. Each getter uses the SELECTED program +0x2b8 and
    // the UNMODIFIED material block. Sky type is not noScreenShadows.
    // Evaluate every getter even when an earlier operand is true.
    let no_screen_shadows = [
        b"noScreenShadows".as_slice(),
        b"translucency_hq/enable",
        b"translucency/enable",
    ]
    .into_iter()
    .try_fold(is_alpha, |combined, name| {
        library
            .option_value(selected_program.as_bytes(), &original, name)
            .map(|value| combined | (value != 0))
    })?;
    let values = material::rigid_shadow_option_parameters(
        bytes,
        alpha_test,
        is_alpha,
        no_screen_shadows,
        |name| {
            library.parameter_kinds.get(name.as_bytes())
                == Some(&render_program::RenderParameterKind::Option)
        },
    )?;
    Ok(native_option_block(values))
}

pub(super) struct MaterialResolver<'a> {
    pub(super) anisotropic_sampler_image: Handle<Image>,
    pub(super) catalog: Option<&'a archive::Catalog>,
    pub(super) native_programs: Option<&'a mut Result<shader_cache::NativeProgramLibrary, String>>,
    pub(super) probe_materials: &'a mut Assets<ProbeMaterial>,
    pub(super) decal_materials: &'a mut Assets<DecalMaterial>,
    pub(super) probe_sh_buffers: &'a mut Assets<ShaderBuffer>,
    pub(super) probe_sh_buffer: Handle<ShaderBuffer>,
    pub(super) vertex_probe_buffer: Handle<ShaderBuffer>,
    pub(super) native_shadow_buffer: Handle<ShaderBuffer>,
    pub(super) native_local_shadow_buffer: Handle<ShaderBuffer>,
    pub(super) native_shadow_map: Handle<Image>,
    pub(super) probe_sh_records: Vec<[Vec4; 9]>,
    pub(super) probes: Option<&'a probe_lighting::ProbeLightingSet>,
    pub(super) cache: HashMap<String, Option<CachedMaterial>>,
    pub(super) shadow_materials: NativeShadowMaterials,
    pub(super) diffuse_image_slots: HashMap<String, usize>,
    pub(super) diffuse_image_paths: Vec<String>,
    pub(super) preferred_mip: Option<u8>,
    pub(super) misses: usize,
    pub(super) suppressed_transparent: usize,
    pub(super) cutout: usize,
    pub(super) blended: usize,
    pub(super) probe_samples: usize,
    pub(super) probe_sample_misses: usize,
    pub(super) lit: bool,
}

impl MaterialResolver<'_> {
    pub(super) fn native_caster_admission(&mut self, path: &str) -> Result<u8, String> {
        let catalog = self
            .catalog
            .ok_or("native caster material catalog is unavailable")?;
        let bytes = catalog
            .read(&format!("generated/decls/material/{path}.decl"))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("native caster material is not indexed: {path}"))?;
        let main = material_surface::Surface::parse(&bytes).main_program;
        let program = main
            .as_ref()
            .map(|name| {
                catalog
                    .read(&format!(
                        "generated/decls/renderprog/{}.decl",
                        name.to_ascii_lowercase()
                    ))
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("native MAIN program missing: {name}"))
            })
            .transpose()?
            .unwrap_or_default();
        let source = resolve_native_shadow_source_inputs(
            self.native_programs.as_deref_mut(),
            &bytes,
            &program,
            main.as_deref(),
        )?;
        let present = resolve_native_shadow_slot_presence(
            self.native_programs.as_deref_mut(),
            &bytes,
            &source,
            main.as_deref(),
        )?;
        Ok(u8::from(present) | (u8::from(source.needs_wind) << 6))
    }
    pub(super) fn append_probe_record(&mut self, game_position: [f32; 3]) -> u32 {
        let sh = if self.lit {
            match self
                .probes
                .and_then(|probes| probes.interpolate_static_sh(game_position))
            {
                Some(lighting) => {
                    self.probe_samples += 1;
                    lighting.render_lighting_sh
                }
                None => {
                    if self.probes.is_some() {
                        self.probe_sample_misses += 1;
                    }
                    [[0.0; 3]; 9]
                }
            }
        } else {
            [[0.0; 3]; 9]
        };
        let record =
            sh.map(|coefficient| Vec4::new(coefficient[0], coefficient[1], coefficient[2], 0.0));
        let index = self.probe_sh_records.len() as u32;
        self.probe_sh_records.push(record);
        index
    }

    pub(super) fn upload_probe_records(&mut self) {
        if let Some(mut buffer) = self.probe_sh_buffers.get_mut(&self.probe_sh_buffer) {
            buffer.set_data(std::mem::take(&mut self.probe_sh_records));
        }
    }

    pub(super) fn register_image(&mut self, name: &str) -> Option<usize> {
        let catalog = self.catalog?;
        let mut stem = material::compiled_image_stem(catalog, name)?;
        stem.make_ascii_lowercase();
        if material::intrinsic_image_pixels(&stem).is_none()
            && !catalog.contains(&stem)
            && !(0..=self.preferred_mip.unwrap_or(15))
                .any(|mip| catalog.contains(&format!("{stem}_mip{mip}")))
        {
            return None;
        }
        if let Some(&slot) = self.diffuse_image_slots.get(&stem) {
            return Some(slot);
        }
        let slot = self.diffuse_image_paths.len();
        self.diffuse_image_paths.push(stem.clone());
        self.diffuse_image_slots.insert(stem, slot);
        Some(slot)
    }

    pub(super) fn get(
        &mut self,
        path: &str,
        _color_seed: usize,
        game_position: [f32; 3],
    ) -> Option<ResolvedMaterial> {
        if let Some(cached) = self.cache.get(path) {
            let cached = cached.clone()?;
            let probe_index = self.append_probe_record(game_position);
            return Some(ResolvedMaterial {
                handle: cached.handle,
                diffuse_image_slot: cached.diffuse_image_slot,
                probe_index,
                native_vertex_lighting: cached.native_vertex_lighting,
            });
        }
        let key = path.to_owned();
        let bytes = self.catalog.and_then(|c| {
            c.read(&format!("generated/decls/material/{path}.decl"))
                .ok()
                .flatten()
        });
        let bytes = bytes.as_deref().unwrap_or(&[]);
        if material::is_screen_gate_impostor(path, bytes) {
            self.cache.insert(key, None);
            return None;
        }
        let surface = material_surface::Surface::parse(bytes);
        if surface.extra_layers > 2
            || surface
                .layers
                .iter()
                .any(|l| l.mask_channel > 3 || l.mask_mode > 2)
        {
            eprintln!("Unsupported material layer inputs {path}");
            self.cache.insert(key, None);
            return None;
        }
        let mut uniform = surface_gpu::SurfaceUniform::from(&surface);
        let program = surface.main_program.as_ref().and_then(|name| {
            self.catalog.and_then(|c| {
                c.read(&format!(
                    "generated/decls/renderprog/{}.decl",
                    name.to_ascii_lowercase()
                ))
                .ok()
                .flatten()
            })
        });
        let program = program.as_deref().unwrap_or(&[]);
        if matches!(uniform.misc.z, 1 | 2) {
            let receiver = (|| -> Result<UVec4, String> {
                let source = resolve_native_shadow_source_inputs(
                    self.native_programs.as_deref_mut(),
                    bytes,
                    program,
                    surface.main_program.as_deref(),
                )?;
                let library = self
                    .native_programs
                    .as_deref_mut()
                    .ok_or("native program library is unavailable")?
                    .as_mut()
                    .map_err(|error| error.clone())?;
                let main = surface
                    .main_program
                    .as_deref()
                    .ok_or("native receiver MAIN is absent")?;
                let overrides =
                    native_shadow_variant_overrides(library, bytes, &source, Some(main), main)?;
                let mut option =
                    |name: &[u8]| library.option_value(main.as_bytes(), &overrides, name);
                let direct = option(b"noScreenShadows")? != 0;
                let flags = u32::from(option(b"shadow/useNormalOffset")? != 0)
                    | (u32::from(option(b"shadow/useSlopeScale")? != 0) << 1)
                    | (u32::from(option(b"shadow/useSlopeScaleBias")? != 0) << 2)
                    | (u32::from(option(b"shadow/slopeScaleIsPostProjection")? != 0) << 3)
                    | (u32::from(option(b"translucency/enable")? == 0) << 4);
                Ok(UVec4::new(
                    u32::from(direct),
                    flags,
                    option(b"shadowFilteringQuality")? as u32,
                    2,
                ))
            })();
            match receiver {
                Ok(value) => uniform.native_shadow = value,
                Err(error) => {
                    eprintln!("Native forward shadow receiver unavailable for {path}: {error}")
                }
            }
        }
        // FUN_1402d5210 encodes inherited `invisible` as material+0x4b's
        // excluded state; FUN_14033b350 rejects it before creating any rows.
        if material::program_option(bytes, program, "invisible") {
            self.cache.insert(key, None);
            return None;
        }
        let transparency = material::transparency_with_program(bytes, program);
        let program_is_decal = material::program_option(bytes, program, "isDecal");
        let two_sided_lighting = material::program_option(bytes, program, "twoSidedLighting");
        let alpha_test = material::program_option(bytes, program, "alphaTest");
        uniform.misc.w = u32::from(alpha_test)
            | (u32::from(program_is_decal) << 1)
            | (u32::from(two_sided_lighting) << 2);
        if env::var("DH2_TRACE_MATERIAL").ok().as_deref() == Some(path) {
            println!("NATIVE_MATERIAL {path}: {uniform:#?}");
        }
        let decal_bias = material::decal_depth_bias(bytes);
        let mut images = Vec::new();
        for (role, name) in surface.texture_bindings(program_is_decal) {
            if let Some(slot) = self.register_image(name) {
                images.push((slot, surface_gpu::ImageRole(role)));
            } else {
                eprintln!("Material image missing {path} role={role} name={name}");
            }
        }
        let diffuse_image_slot = images
            .iter()
            .find(|(_, role)| role.0 == 0)
            .map(|&(slot, _)| slot);
        if diffuse_image_slot.is_none() {
            self.misses += 1;
        }
        let alpha_mode = match transparency {
            material::Transparency::Opaque => AlphaMode::Opaque,
            material::Transparency::Cutout => {
                self.cutout += 1;
                AlphaMode::Mask(surface.forward_alpha_cutoff())
            }
            material::Transparency::Coverage => {
                if surface
                    .main_program
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case("arkDecal"))
                {
                    self.blended += 1;
                    AlphaMode::Blend
                } else {
                    self.cutout += 1;
                    AlphaMode::AlphaToCoverage
                }
            }
            material::Transparency::Blend => {
                self.blended += 1;
                AlphaMode::Blend
            }
        };
        let is_decal = program_is_decal || decal_bias.is_some();
        let cull_mode = match material::cull_mode_with_program(bytes, program) {
            Ok(material::CullMode::None) => None,
            Ok(material::CullMode::Front) => Some(bevy::render::render_resource::Face::Front),
            Ok(material::CullMode::Back) => Some(bevy::render::render_resource::Face::Back),
            Err(error) => {
                eprintln!("Material {path}: {error}");
                self.misses += 1;
                return None;
            }
        };
        let base = StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: None,
            alpha_mode,
            depth_bias: 0.0,
            unlit: !self.lit || path.contains("/vista/") || path.contains("/skies/"),
            perceptual_roughness: 1.0 - surface.ward[1],
            metallic: surface.metallic_min_max[1],
            double_sided: two_sided_lighting,
            cull_mode,
            ..default()
        };
        let native_vertex_lighting = matches!(uniform.misc.z, 1 | 2);
        let handle = if is_decal {
            MaterialHandle::Decal(self.decal_materials.add(DecalMaterial {
                base,
                extension: DecalExtension {
                    bias: decal_bias.unwrap_or(1678.0).round() as i32,
                    sh: self.probe_sh_buffer.clone(),
                    vertex_probes: self.vertex_probe_buffer.clone(),
                    native_shadow: self.native_shadow_buffer.clone(),
                    native_local_shadow: self.native_local_shadow_buffer.clone(),
                    native_shadow_map: Some(self.native_shadow_map.clone()),
                    native_local_shadow_map: Some(self.native_shadow_map.clone()),
                    native_screen_map: None,
                    native_cloud_map: None,
                    surface: uniform,
                    diffuse0: None,
                    diffuse1: None,
                    diffuse2: None,
                    bump0: None,
                    bump1: None,
                    bump2: None,
                    gloss0: None,
                    gloss1: None,
                    gloss2: None,
                    metallic0: None,
                    metallic1: None,
                    metallic2: None,
                    mask0: None,
                    mask1: None,
                    mask2: None,
                    specular0: None,
                    specular1: None,
                    specular2: None,
                    detail_bump: None,
                    emissive_map: None,
                    emissive_mask: None,
                    anisotropic_sampler_image: Some(self.anisotropic_sampler_image.clone()),
                },
            }))
        } else {
            MaterialHandle::Probe(self.probe_materials.add(ProbeMaterial {
                base,
                extension: ProbeExtension {
                    sh: self.probe_sh_buffer.clone(),
                    vertex_probes: self.vertex_probe_buffer.clone(),
                    native_shadow: self.native_shadow_buffer.clone(),
                    native_local_shadow: self.native_local_shadow_buffer.clone(),
                    native_local_shadow_map: Some(self.native_shadow_map.clone()),
                    native_shadow_map: Some(self.native_shadow_map.clone()),
                    native_screen_map: None,
                    native_cloud_map: None,
                    surface: uniform,
                    diffuse0: None,
                    diffuse1: None,
                    diffuse2: None,
                    bump0: None,
                    bump1: None,
                    bump2: None,
                    gloss0: None,
                    gloss1: None,
                    gloss2: None,
                    metallic0: None,
                    metallic1: None,
                    metallic2: None,
                    mask0: None,
                    mask1: None,
                    mask2: None,
                    specular0: None,
                    specular1: None,
                    specular2: None,
                    detail_bump: None,
                    emissive_map: None,
                    emissive_mask: None,
                    anisotropic_sampler_image: Some(self.anisotropic_sampler_image.clone()),
                },
            }))
        };
        let shadow = resolve_native_shadow_source_inputs(
            self.native_programs.as_deref_mut(),
            bytes,
            program,
            surface.main_program.as_deref(),
        )
        .map(|source| {
            let base_slot10_present = resolve_native_shadow_slot_presence(
                self.native_programs.as_deref_mut(),
                bytes,
                &source,
                surface.main_program.as_deref(),
            );
            let rigid_base_source = resolve_native_rigid_shadow_source(
                self.native_programs.as_deref_mut(),
                bytes,
                &source,
                surface.main_program.as_deref(),
            );
            let world_source = resolve_native_world_shadow_source(
                self.native_programs.as_deref_mut(),
                bytes,
                &source,
                surface.main_program.as_deref(),
            );
            let world_parameters = match &world_source {
                Ok(None) => Ok(None),
                Err(error) => Err(error.clone()),
                Ok(Some(compiled)) => (|| {
                    let catalog = self
                        .catalog
                        .ok_or("native world shadow parameter catalog is unavailable")?;
                    let library = self
                        .native_programs
                        .as_deref_mut()
                        .ok_or("native program library is unavailable")?
                        .as_mut()
                        .map_err(|error| error.clone())?;
                    let class = compiled.class.material_parameter_class();
                    resolve_native_shadow_material_parameters(library, catalog, bytes, class, false)
                        .map(Some)
                })(),
            };
            let parameters = match &rigid_base_source {
                Ok(None) => Ok(None),
                Err(error) => Err(error.clone()),
                Ok(Some(compiled)) => (|| {
                    let catalog = self
                        .catalog
                        .ok_or("native shadow parameter catalog is unavailable")?;
                    let library = self
                        .native_programs
                        .as_deref_mut()
                        .ok_or("native program library is unavailable")?
                        .as_mut()
                        .map_err(|error| error.clone())?;
                    resolve_native_shadow_material_parameters(
                        library,
                        catalog,
                        bytes,
                        compiled.class,
                        true,
                    )
                    .map(Some)
                })(),
            };
            NativeShadowMaterialInputs {
                source,
                rigid_base_source,
                base_slot10_present,
                world_source,
                world_parameters,
                main_program: surface.main_program.clone(),
                alpha_test,
                world_uv_set: surface.layers.first().map(|layer| layer.uv_set),
                diffuse_image_slot,
                parameters,
                cull_mode,
            }
        });
        self.shadow_materials.0.insert(handle.clone(), shadow);
        let cached = CachedMaterial {
            handle,
            diffuse_image_slot,
            images,
            native_vertex_lighting,
        };
        self.cache.insert(key, Some(cached.clone()));
        let probe_index = self.append_probe_record(game_position);
        Some(ResolvedMaterial {
            handle: cached.handle,
            diffuse_image_slot: cached.diffuse_image_slot,
            probe_index,
            native_vertex_lighting: cached.native_vertex_lighting,
        })
    }
}
