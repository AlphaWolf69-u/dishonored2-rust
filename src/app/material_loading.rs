//! Material loading.
use super::{
    archive, entities, env, material, material_surface, probe_lighting, render_program,
    render_state, shader_cache, surface_gpu, CachedMaterial, DecalExtension, DecalMaterial,
    HashMap, MaterialHandle, ProbeExtension, ProbeMaterial, ResolvedMaterial, ShaderBuffer,
    ShadowMaterialInputs, ShadowMaterialParameters, ShadowMaterials,
};
use bevy::prelude::*;

mod cache;
mod water;
pub(in crate::app) mod alpha;
use cache::{MaterialCache, MaterialGeometry};

#[cfg(test)]
mod pass_tests;

/// Only boat-list membership is resolved here. World and model predicates
/// differ; ordinary source admission and the other draw lists retain their owners.
fn boat_volume_routes(
    library: &mut shader_cache::ProgramLibrary,
    catalog: &archive::Catalog,
    bytes: &[u8],
    main: &str,
    stage_sort: [u32; 2],
) -> Result<[bool; 2], String> {
    let sort = stage_sort.map(f32::from_bits);
    let thresholds = library.material_sort_thresholds(catalog)?;
    // Other flag bits only change branches outside the boat interval.
    if material::draw_bucket(0, 0, 0, false, sort, thresholds) != 0x26 {
        return Ok([false; 2]);
    }
    let overrides = material_option_overrides(library, bytes)?;
    let mut flags48 =
        (u8::from(library.option_value(main.as_bytes(), &overrides, b"isDecal")? & 1 != 0) << 4)
            | (u8::from(main.eq_ignore_ascii_case("atm/worldDistanceFog/arkInteriorVolume")) << 7);
    let mut flags4a =
        (u8::from(library.option_value(main.as_bytes(), &overrides, b"hairType")? & 1 != 0) << 4)
            | ((library.registered_option_value(&overrides, b"decalType")? as u8 & 3) << 2);
    let flags4b = library.option_value(main.as_bytes(), &overrides, b"skyType")? as u8 & 3;
    // Slot12 is SSS, not refraction. Its registered Program default
    // is null; an authored non-null program precedes sort admission.
    let sss = material::authored_program_override(bytes, "sssProgram")
        .is_some_and(|p| !p.is_empty() && p != "0");
    let model = material::draw_bucket(flags48, flags4a, flags4b, sss, sort, thresholds) == 0x26;
    flags48 |=
        u8::from(library.option_value(main.as_bytes(), &overrides, b"needsWind")? & 1 != 0) << 6;
    flags4a |=
        (library.registered_option_value(&overrides, b"rendering/darkVisionLayer")? as u8) << 5;
    let darkvision = material::authored_program_override(bytes, "darkVisionProgram")
        .is_some_and(|p| !p.is_empty() && p != "0");
    let state = library
        .definition(main.as_bytes())?
        .ok_or("material MAIN definition absent")?
        .render_state();
    let state = render_state::material_render_state(&entities::declaration_text(bytes), state)?;
    // Boat admission is unchanged by the view mask: an implicit
    // dark-vision layer routes elsewhere or is rejected in either case.
    let world = crate::rendering::material_lists::world_draw_list(
        crate::rendering::material_lists::WorldMaterial {
            flags_48: flags48,
            flags_4a: flags4a,
            flags_4b: flags4b,
            main_blend_state: state[0],
            stage_sort: sort,
            darkvision_program_present: darkvision,
        },
        thresholds,
        0,
    ) == Some(crate::rendering::material_lists::WorldDrawList::BoatVolume);
    Ok([world, model])
}

/// Resolve the slot-admission fields from the same dictionaries as 1402d5210.
/// In particular +0x48 bit4 is MAIN-fallback isDecal, not invisible. This is
/// source selection only, not a claim that all other material flags are loaded.
pub(super) fn resolve_shadow_source_inputs(
    library: Option<&mut Result<shader_cache::ProgramLibrary, String>>,
    bytes: &[u8],
    program: &[u8],
    main_program: Option<&str>,
) -> Result<material::ShadowSourceInputs, String> {
    let mut source = material::shadow_source_inputs(bytes, program)?;
    let library = library
        .ok_or("program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("program library: {error}"))?;
    let main = main_program.ok_or("shadow source owner has no main program")?;
    let state = library
        .definition(main.as_bytes())?
        .ok_or_else(|| format!("missing native main program {main}"))?
        .render_state();
    // 1402d662d passes all nine MAIN words to 1402c3bc0. A line scan
    // misses compact state blocks and can mistake nested option names for
    // drawing commands, changing native shadow-bank admission.
    let material_state =
        render_state::material_render_state(&String::from_utf8_lossy(bytes), state)?;
    source.main_blend_state = material_state[0];
    source.main_render_state = Some(material_state);
    // These constructor fields precede per-pass specialization. Read the
    // material dictionary, not a block already overwritten for shadow slot 10.
    let overrides = material_option_overrides(library, bytes)?;
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

/// Native material +0x48 bit0 starts with the finalized base-bank slot10
/// pointer test, then is cleared for sortBoatVolume by material finalization.
/// It is independent of the world-bank pointer and supported shader ABIs.
/// All current level sources are resident; a missing cache row still requires
/// the original runtime compile path and must not become `false` here.
pub(super) fn resolve_shadow_slot_presence(
    library: Option<&mut Result<shader_cache::ProgramLibrary, String>>,
    catalog: &archive::Catalog,
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
        .ok_or("program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("program library: {error}"))?;
    // 2d6f39 clears +48 bit0 for the merged stageSort==sortBoatVolume,
    // before the later arkWater sort override. It does not clear the actual
    // program pointer, so retain its source independently of this flag.
    let main = main_program.ok_or("shadow material MAIN absent")?;
    let sort = library.material_stage_sort_parameters(catalog, main.as_bytes(), &entities::declaration_text(bytes))?;
    let empty = render_program::SerializedParameterBlock {flags: 0, parameters: Vec::new()};
    let boat = library.registered_float_vector(catalog, &empty, b"sortboatvolume", None)?[0];
    if f32::from_bits(sort[0]) == f32::from_bits(boat) {
        return Ok(false);
    }
    let overrides = shadow_variant_overrides(library, bytes, source, main_program, name)?;
    if library
        .serialized_variant(name.as_bytes(), &overrides)?
        .is_none()
    {
        return Err(format!(
            "base shadow slot needs runtime compilation: {name}"
        ));
    }
    Ok(true)
}

pub(super) fn resolve_rigid_shadow_source(
    library: Option<&mut Result<shader_cache::ProgramLibrary, String>>,
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
        .ok_or("program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("program library: {error}"))?;
    let overrides = shadow_variant_overrides(library, bytes, source, main_program, name)?;
    library
        .rigid_shadow_source(name.as_bytes(), &overrides)?
        .map(Some)
        .ok_or_else(|| format!("no resident rigid shadow variant for {name}"))
}

/// FUN_1402d5210 bank2 at 1402d66a0..1402d66f0. The direct-instancing
/// exclusion is read BEFORE pass overrides and uses registered defaults,
/// not a MAIN-fallback value. Resident absence is an error, not "no shadow".
pub(super) fn resolve_world_shadow_source(
    library: Option<&mut Result<shader_cache::ProgramLibrary, String>>,
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
        .ok_or("program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("program library: {error}"))?;
    let reference = material_option_overrides(library, bytes)?;
    let multi = library.registered_option_value(&reference, b"useMultiInstancing")? != 0;
    let direct = library.registered_option_value(&reference, b"useDirectInstancing")? != 0;
    if material::instancing_bank_override(2, multi, direct)?.is_none() {
        return Ok(None);
    }
    let mut overrides = shadow_variant_overrides(library, bytes, source, main_program, name)?;
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

pub(super) fn resolve_shadow_material_parameters(
    library: &mut shader_cache::ProgramLibrary,
    catalog: &archive::Catalog,
    bytes: &[u8],
    class: shader_cache::CachedRigidShadowClass,
    model_depth: bool,
) -> Result<ShadowMaterialParameters, String> {
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
    let values = entities::material_float_parameters(&entities::declaration_text(bytes), |name| {
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
    Ok(ShadowMaterialParameters::new(overrides, defaults))
}

pub(super) fn material_option_overrides(
    library: &shader_cache::ProgramLibrary,
    bytes: &[u8],
) -> Result<render_program::SerializedParameterBlock, String> {
    let values = material::material_option_parameters(bytes, |name| {
        library.parameter_kinds.get(name.as_bytes())
            == Some(&render_program::RenderParameterKind::Option)
    })?;
    Ok(option_block(values))
}

pub(super) fn option_block(values: Vec<(String, i32)>) -> render_program::SerializedParameterBlock {
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

pub(super) fn shadow_variant_overrides(
    library: &mut shader_cache::ProgramLibrary,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
    selected_program: &str,
) -> Result<render_program::SerializedParameterBlock, String> {
    let reference = material_option_overrides(library, bytes)?;
    // Material +0x4b bit4 uses MAIN, even with an authored shadow source.
    let main = main_program.ok_or("shadow alpha-test owner has no main program")?;
    let alpha_test = library.option_value(main.as_bytes(), &reference, b"alphaTest")?;
    // isDecal uses MAIN, but decalType is material/registered-default only.
    let is_decal = library.option_value(main.as_bytes(), &reference, b"isDecal")? & 1 != 0;
    let decal_type = library.registered_option_value(&reference, b"decalType")? as u32;
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
            .option_value(selected_program.as_bytes(), &reference, name)
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
    Ok(option_block(values))
}

fn forward_fog_flags(
    library: &mut shader_cache::ProgramLibrary,
    bytes: &[u8],
    main: &str,
    source: &material::ShadowSourceInputs,
) -> Result<u32, String> {
    let reference = option_block(material::material_option_parameters(bytes, |name| {
        library.parameter_kinds.get(name.as_bytes())
            == Some(&render_program::RenderParameterKind::Option)
    })?);
    let is_decal = library.option_value(main.as_bytes(), &reference, b"isDecal")? & 1 != 0;
    let decal_type = library.registered_option_value(&reference, b"decalType")? as u32;
    let is_alpha = material::shadow_is_alpha(
        source.main_blend_state,
        is_decal,
        decal_type,
        source.sky_type,
    );
    // 3386a0 routes ordinary decal types 0/3 into the opaque list (0x23),
    // despite their framebuffer blend. Keep that route for the host adapter.
    let before_fog = u32::from(is_decal && matches!(decal_type & 3, 0 | 3)) << 9;
    if !is_alpha
        || library.option_value(main.as_bytes(), &reference, b"tool/unlit")? != 0
        || library.option_value(main.as_bytes(), &reference, b"isMainPassAlpha")? != 0
    {
        return Ok(before_fog);
    }
    // Existing misc.w bits0..2 retain alpha-test, decal and sided-lighting
    // controls. Fog bits participate in the same material pipeline key.
    let mut flags = 8;
    if main.eq_ignore_ascii_case("particle/filing_container") {
        flags |= 16;
        if library.option_value(main.as_bytes(), &reference, b"fx/opt/use_mesh")? != 0 {
            flags |= 32;
        }
    }
    Ok(flags)
}

pub(super) struct MaterialResolver<'a> {
    pub(super) fog_material: crate::rendering::bevy_backend::fog_material::FogMaterialBindings,
    pub(super) reflections: Option<Handle<Image>>,
    pub(super) reflection_brdf_lookup: Option<Handle<Image>>,
    pub(super) reflection_planes: Handle<ShaderBuffer>,
    pub(super) material_exposure: Handle<ShaderBuffer>,
    pub(super) anisotropic_sampler_image: Handle<Image>,
    pub(super) linear_clamp_image: Handle<Image>,
    pub(super) vista_sun: [Vec4; 2],
    pub(super) environment: Option<&'a entities::EnvironmentRenderParameters>,
    pub(super) catalog: Option<&'a archive::Catalog>,
    pub(super) programs: Option<&'a mut Result<shader_cache::ProgramLibrary, String>>,
    pub(super) probe_materials: &'a mut Assets<ProbeMaterial>,
    pub(super) sky_materials: &'a mut Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
    pub(super) decal_materials: &'a mut Assets<DecalMaterial>,
    pub(super) probe_sh_buffers: &'a mut Assets<ShaderBuffer>,
    pub(super) probe_sh_buffer: Handle<ShaderBuffer>,
    pub(super) vertex_probe_buffer: Handle<ShaderBuffer>,
    pub(super) shadow_buffer: Handle<ShaderBuffer>,
    pub(super) local_shadow_buffer: Handle<ShaderBuffer>,
    pub(super) shadow_map: Handle<Image>,
    pub(super) probe_sh_records: Vec<[Vec4; 9]>,
    pub(super) probes: Option<&'a probe_lighting::ProbeLightingSet>,
    pub(super) cache: MaterialCache,
    pub(super) shadow_materials: ShadowMaterials,
    pub(super) vegetation: super::vegetation_wind::VegetationMaterials,
    pub(super) water_materials: crate::rendering::bevy_backend::water_scene::WaterMaterials,
    pub(super) water_model_colours: crate::rendering::bevy_backend::water_scene::WaterModelColours,
    pub(super) water_stencil_materials: crate::rendering::bevy_backend::water_scene::WaterStencilMaterials,
    pub(super) boat_materials: crate::rendering::bevy_backend::boat_scene::BoatMaterials,
    pub(super) diffuse_image_slots: HashMap<String, usize>,
    pub(super) diffuse_image_paths: Vec<String>,
    pub(super) misses: usize,
    pub(super) suppressed_transparent: usize,
    pub(super) cutout: usize,
    pub(super) blended: usize,
    pub(super) probe_samples: usize,
    pub(super) probe_sample_misses: usize,
    pub(super) lit: bool,
}

pub(super) fn material_float_vectors<const N: usize>(
    library: &mut shader_cache::ProgramLibrary,
    catalog: &archive::Catalog,
    bytes: &[u8],
    names: [&str; N],
) -> Result<[[f32; 4]; N], String> {
    let values = entities::material_float_parameters(&entities::declaration_text(bytes), |name| {
        names.contains(&name)
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
    let mut result = [[0.; 4]; N];
    for (i, name) in names.iter().enumerate() {
        result[i] = library
            .registered_float_vector(catalog, &overrides, name.as_bytes(), None)?
            .map(f32::from_bits);
    }
    Ok(result)
}

impl MaterialResolver<'_> {
    fn float_vectors<const N: usize>(
        &mut self,
        bytes: &[u8],
        names: [&str; N],
    ) -> Result<[[f32; 4]; N], String> {
        let library = self
            .programs
            .as_deref_mut()
            .ok_or("program library unavailable")?
            .as_mut()
            .map_err(|e| e.clone())?;
        let catalog = self.catalog.ok_or("material catalog unavailable")?;
        material_float_vectors(library, catalog, bytes, names)
    }

    pub(super) fn caster_admission(&mut self, path: &str) -> Result<u8, String> {
        let catalog = self
            .catalog
            .ok_or("caster material catalog is unavailable")?;
        let bytes = catalog
            .read(&format!("generated/decls/material/{path}.decl"))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("caster material is not indexed: {path}"))?;
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
                    .ok_or_else(|| format!("MAIN program missing: {name}"))
            })
            .transpose()?
            .unwrap_or_default();
        let source = resolve_shadow_source_inputs(
            self.programs.as_deref_mut(),
            &bytes,
            &program,
            main.as_deref(),
        )?;
        let present = resolve_shadow_slot_presence(
            self.programs.as_deref_mut(),
            catalog,
            &bytes,
            &source,
            main.as_deref(),
        )?;
        Ok(u8::from(present) | (u8::from(source.needs_wind) << 6))
    }
    pub(super) fn append_probe_record(&mut self, game_position: [f32; 3]) -> u32 {
        let record = if self.lit {
            match self
                .probes
                .and_then(|probes| probes.interpolate_static_sh(game_position))
            {
                Some(lighting) => {
                    self.probe_samples += 1;
                    lighting.object_shader_record()
                }
                None => {
                    if self.probes.is_some() {
                        self.probe_sample_misses += 1;
                    }
                    [[0.0; 4]; 9]
                }
            }
        } else {
            [[0.0; 4]; 9]
        };
        let record = record.map(Vec4::from_array);
        let index = self.probe_sh_records.len() as u32;
        self.probe_sh_records.push(record);
        index
    }

    /// VS14200 shadow casters and every forward arkVegetation material read
    /// the original local position, NORMAL.xy and COLOR channels.
    pub(super) fn needs_vegetation_channels(&self, material: &MaterialHandle) -> bool {
        self.shadow_materials.uses_vegetation(material)
            || self.vegetation.parameters.contains_key(material)
    }

    pub(super) fn upload_probe_records(&mut self) {
        if let Some(mut buffer) = self.probe_sh_buffers.get_mut(&self.probe_sh_buffer) {
            buffer.set_data(std::mem::take(&mut self.probe_sh_records));
        }
    }

    pub(super) fn register_image(&mut self, name: &str) -> Option<usize> {
        let catalog = self.catalog?;
        let mut stem = material::image_load_resource(catalog, name)?;
        if stem == "_default" && !name.trim().eq_ignore_ascii_case("_default") {
            eprintln!("Image {name}: resource missing; using the image loader's default texture");
        }
        stem.make_ascii_lowercase();
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
        self.get_for(path, MaterialGeometry::Model, game_position)
    }

    pub(super) fn get_world(
        &mut self,
        path: &str,
        _color_seed: usize,
        game_position: [f32; 3],
    ) -> Option<ResolvedMaterial> {
        self.get_for(path, MaterialGeometry::World, game_position)
    }

    fn get_for(
        &mut self,
        path: &str,
        geometry: MaterialGeometry,
        game_position: [f32; 3],
    ) -> Option<ResolvedMaterial> {
        if let Some(cached) = self.cache.get_for(path, geometry) {
            let cached = cached.clone()?;
            let probe_index = self.append_probe_record(game_position);
            return Some(ResolvedMaterial {
                handle: cached.handle,
                diffuse_image_slot: cached.diffuse_image_slot,
                probe_index,
                vertex_lighting: cached.vertex_lighting,
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
            self.cache.insert(key, geometry, None);
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
            self.cache.insert(key, geometry, None);
            return None;
        }
        let mut uniform = surface_gpu::SurfaceUniform::from(&surface);
        if matches!(uniform.misc.z, 1 | 2 | 6) {
            match self.float_vectors(bytes,["sheen","sheenImportance"]) {
                Ok(v) => uniform.direct = Vec4::new(v[0][0],v[1][0],0.0,0.0),
                Err(error) => eprintln!("Surface direct-light parameters unavailable {path}: {error}"),
            }
        }
        let mut sky_depth_parameters = None;
        if matches!(
            surface.program(),
            material_surface::Program::Sky | material_surface::Program::Clouds
        ) {
            let result = (|| -> Result<_, String> {
                let values = self.float_vectors(bytes, crate::rendering::sky::PARAMETER_NAMES)?;
                let mut p = [Vec4::ZERO; 7];
                p[..5].copy_from_slice(
                    &crate::rendering::sky::parameter_vectors(values).map(Vec4::from_array),
                );
                // Registered Uniform int cloudProjOnFar defaults to zero.
                let project =
                    entities::material_float_parameters(&entities::declaration_text(bytes), |n| {
                        n == "cloudprojonfar"
                    })?;
                p[4].x = project
                    .last()
                    .map(|(_, v)| f32::from_bits(v[0]))
                    .unwrap_or(0.0);
                let environment = self.environment.ok_or("sky environment unavailable")?;
                let read = crate::assets::environment::ParameterReader {
                    catalog: self.catalog.ok_or("sky parameter catalog unavailable")?,
                    parameters: environment,
                };
                let direction = if read.boolean("atm/worldfog/useactualsundirection")? {
                    self.vista_sun[1].truncate()
                } else {
                    super::game_to_view(crate::render_shadow::environment_sun_direction(
                        read.scalar("atm/worldfog/scatteringangletheta")?,
                        read.scalar("atm/worldfog/scatteringanglephi")?,
                    ))
                };
                p[5] = direction.extend(read.scalar("atm/worldfog/scatteringphase")?);
                let color = if read.boolean("atm/worldfog/useactualsuncolor")? {
                    self.vista_sun[0].truncate()
                } else {
                    Vec4::from_array(read.vector("atm/worldfog/artistscatteringcolor")?).truncate()
                };
                p[6] = color.extend(0.0);
                Ok(p)
            })();
            match result {
                Ok(p) => {
                    uniform.sky = p;
                    let depth_name = if surface.program() == material_surface::Program::Clouds {
                        "clouddepthoffset"
                    } else {
                        "skydepthoffset"
                    };
                    match self.float_vectors(bytes, [depth_name, "skydepthuvscale"]) {
                        Ok([offset, uv]) => {
                            sky_depth_parameters = Some((offset[0], Vec4::from_array(uv)))
                        }
                        Err(error) => {
                            eprintln!("Sky fog depth unavailable {path}: {error}");
                            return None;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Sky material unavailable {path}: {e}");
                    return None;
                }
            }
        }
        if surface.program() == material_surface::Program::Vista {
            let parameters = (|| -> Result<_, String> {
                use crate::rendering::vista;
                let library = self
                    .programs
                    .as_deref_mut()
                    .ok_or("program library unavailable")?
                    .as_mut()
                    .map_err(|e| e.clone())?;
                let catalog = self.catalog.ok_or("material catalog unavailable")?;
                let values = entities::material_float_parameters(
                    &entities::declaration_text(bytes),
                    |name| vista::PARAMETER_NAMES.contains(&name),
                )?;
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
                vista::parameter_vectors(|name| {
                    library
                        .registered_float_vector(catalog, &overrides, name.as_bytes(), None)
                        .map(|v| v.map(f32::from_bits))
                })
            })();
            match parameters {
                Ok(values) => {
                    uniform.vista = values.map(Vec4::from_array);
                    uniform.vista_sun = self.vista_sun;
                }
                Err(error) => {
                    eprintln!("Vista parameters unavailable for {path}: {error}");
                    return None;
                }
            }
        }
        if surface.program() == material_surface::Program::Filling {
            let result = (|| -> Result<_, String> {
                use crate::rendering::filling;
                let v = self.float_vectors(bytes, filling::PARAMETER_NAMES)?;
                let clip = self.float_vectors(bytes, filling::CLIP_PARAMETER_NAMES)?;
                let exposure = self.float_vectors(bytes, ["fx/auto_exposure_amount"])?[0][0];
                let library = self
                    .programs
                    .as_deref_mut()
                    .unwrap()
                    .as_mut()
                    .map_err(|e| e.clone())?;
                let overrides = material_option_overrides(library, bytes)?;
                let mut option = |name: &[u8]| {
                    library.option_value(b"particle/filing_container", &overrides, name)
                };
                let mut flags = 0u32;
                for (i, name) in [
                    b"fx/opt/use_mesh".as_slice(),
                    b"fx/opt/fresnel",
                    b"fx/opt/fresnel/useNormalMap",
                    b"fx/opt/fresnel/useLUT",
                ]
                .iter()
                .enumerate()
                {
                    flags |= u32::from(option(name)? != 0) << i;
                }
                let lighting = option(b"fx/opt/use_lighting")? as u32;
                let view_fade = option(b"fx/opt/view_dependent_fade")? != 0;
                // Particle instance geometry and vertex-lit families require
                // their own original input producer, not invented mesh inputs.
                if flags & 1 == 0
                    || lighting > 1
                    || option(b"fx/opt/use_softcontact")? != 0
                    || option(b"fx/opt/use_sprite_lighting")? != 0
                {
                    return Err("filling material requires unported particle/vertex-light/depth-fade input producer".into());
                }
                let mut parameters = filling::parameter_vectors(v, clip);
                parameters[10][1] = exposure;
                if view_fade {
                    flags |= 16;
                    let fade = self.float_vectors(bytes, filling::FADE_PARAMETER_NAMES)?;
                    parameters[10][2] = fade[0][0];
                    parameters[10][3] = fade[1][0];
                }
                Ok((parameters, UVec4::new(flags, lighting, 0, 0)))
            })();
            match result {
                Ok((p, flags)) => {
                    uniform.filling = p.map(Vec4::from_array);
                    uniform.filling_flags = flags;
                }
                Err(error) => {
                    eprintln!("Filling material unavailable {path}: {error}");
                    return None;
                }
            }
        }
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
        let stage_sort = (|| -> Result<[u32; 2], String> {
            let catalog = self.catalog.ok_or("material catalog unavailable")?;
            let main = surface
                .main_program
                .as_deref()
                .ok_or("material MAIN absent")?;
            let library = self
                .programs
                .as_deref_mut()
                .ok_or("program library unavailable")?
                .as_mut()
                .map_err(|e| e.clone())?;
            library.material_stage_sort(catalog, main.as_bytes(), &entities::declaration_text(bytes))
        })();
        let boat_routes = (|| -> Result<[bool; 2], String> {
            let sort = *stage_sort.as_ref().map_err(Clone::clone)?;
            let catalog = self.catalog.ok_or("material catalog unavailable")?;
            let main = surface
                .main_program
                .as_deref()
                .ok_or("material MAIN absent")?;
            let library = self
                .programs
                .as_deref_mut()
                .ok_or("program library unavailable")?
                .as_mut()
                .map_err(|e| e.clone())?;
            boat_volume_routes(library, catalog, bytes, main, sort)
        })()
        .unwrap_or([false; 2]);
        let empty_fragment = surface.program() == material_surface::Program::BoatVolume;
        let boat_routes = boat_routes.map(|route| route || empty_fragment);
        let boat_volume = boat_routes[match geometry {
            MaterialGeometry::World => 0,
            MaterialGeometry::Model => 1,
        }];
        let mut fog_flags = 0;
        if !boat_volume && matches!(uniform.misc.z, 1 | 2 | 6) {
            let receiver = (|| -> Result<UVec4, String> {
                let source = resolve_shadow_source_inputs(
                    self.programs.as_deref_mut(),
                    bytes,
                    program,
                    surface.main_program.as_deref(),
                )?;
                let library = self
                    .programs
                    .as_deref_mut()
                    .ok_or("program library is unavailable")?
                    .as_mut()
                    .map_err(|error| error.clone())?;
                let main = surface
                    .main_program
                    .as_deref()
                    .ok_or("receiver MAIN is absent")?;
                fog_flags = forward_fog_flags(library, bytes, main, &source)?;
                let overrides =
                    shadow_variant_overrides(library, bytes, &source, Some(main), main)?;
                let mut option =
                    |name: &[u8]| library.option_value(main.as_bytes(), &overrides, name);
                // lightContext::ComputeAmbientOcclusion bypasses both map AO
                // and screen AO for the actual isAlpha specialization.
                fog_flags |= u32::from(option(b"isAlpha")? != 0) << 6;
                let standard_brdf = matches!(uniform.misc.z, 1 | 6)
                    && option(b"hasSheenMap")? == 0
                    && option(b"hasDiffuseLookup")? == 0
                    && option(b"fuzz/enable")? == 0
                    && option(b"translucency/enable")? == 0;
                fog_flags |= u32::from(standard_brdf) << 7;
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
                Ok(value) => uniform.shadow = value,
                Err(error) => {
                    eprintln!("Forward shadow receiver unavailable for {path}: {error}")
                }
            }
        }
        // FUN_1402d5210 encodes inherited `invisible` as material+0x4b's
        // excluded state; FUN_14033b350 rejects it before creating any rows.
        if material::program_option(bytes, program, "invisible") {
            self.cache.insert(key, geometry, None);
            return None;
        }
        let transparency = material::transparency_with_program(bytes, program);
        let program_is_decal = material::program_option(bytes, program, "isDecal");
        let two_sided_lighting = material::program_option(bytes, program, "twoSidedLighting");
        let alpha_test = material::program_option(bytes, program, "alphaTest");
        uniform.misc.w = fog_flags
            | u32::from(alpha_test)
            | (u32::from(program_is_decal) << 1)
            | (u32::from(two_sided_lighting) << 2);
        if env::var("DH2_TRACE_MATERIAL").ok().as_deref() == Some(path) {
            println!("MATERIAL {path}: {uniform:#?}");
        }
        // Resolve this Uniform through the same token/registry path as every
        // other material parameter. Program/parameter names are case-insensitive;
        // real arkdecal declarations need the same offset as arkDecal.
        let is_decal = program_is_decal
            || surface
                .main_program
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("arkDecal"));
        let decal_bias = if is_decal {
            match self.float_vectors(bytes, ["zbias"]) {
                Ok([value]) => value[0],
                Err(error) => {
                    eprintln!("Decal depth parameter unavailable {path}: {error}");
                    return None;
                }
            }
        } else {
            0.0
        };
        let mut images = Vec::new();
        for (role, name) in surface.texture_bindings(program_is_decal).into_iter().filter(|_| !boat_volume && surface.program() != material_surface::Program::Water) {
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
        if diffuse_image_slot.is_none()
            && !boat_volume && !matches!(surface.program(), material_surface::Program::InteriorVolume | material_surface::Program::BoatVolume | material_surface::Program::Water)
        {
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
            unlit: !self.lit
                || (surface.program() == material_surface::Program::Filling
                    && uniform.filling_flags.y == 0)
                || surface.main_program.as_deref().is_some_and(|p| {
                    p.eq_ignore_ascii_case("arkSky")
                        || p.eq_ignore_ascii_case("arkClouds")
                        || p.eq_ignore_ascii_case("arkSimpleSprite")
                }),
            perceptual_roughness: 1.0 - surface.ward[1],
            metallic: surface.metallic_min_max[1],
            double_sided: two_sided_lighting,
            cull_mode,
            ..default()
        };
        let vertex_lighting = !boat_volume && matches!(uniform.misc.z, 1 | 2 | 6);
        let handle = if surface.program() == material_surface::Program::InteriorVolume {
            MaterialHandle::Interior(path.into())
        } else if boat_volume || surface.program() == material_surface::Program::BoatVolume {
            let result=(||->Result<_,String>{
                let library=self.programs.as_deref_mut().ok_or("boat program library unavailable")?.as_mut().map_err(|e|e.clone())?;
                if self.boat_materials.model.is_none() {
                    self.boat_materials.model=Some(library.boat_volume_program_state(b"arkBoatVolume",&render_program::SerializedParameterBlock{flags:0,parameters:Vec::new()},false));
                }
                let main=surface.main_program.as_deref().unwrap_or("arkDefault");
                let options=material_option_overrides(library,bytes)?;
                let base=library.boat_volume_program_state(main.as_bytes(),&options,true)?;
                render_state::material_render_state(&entities::declaration_text(bytes),base)
            })();
            std::sync::Arc::make_mut(&mut self.boat_materials.world).insert(path.into(),result);
            MaterialHandle::BoatVolume(path.into())
        } else if surface.program() == material_surface::Program::Water {
            let inputs = (|| -> Result<_, String> {
                let library = self.programs.as_deref_mut()
                    .ok_or("water program library unavailable")?.as_mut().map_err(|e|e.clone())?;
                let main = surface.main_program.as_deref().ok_or("water MAIN absent")?;
                let base = library.definition(main.as_bytes())?
                    .ok_or("water MAIN definition absent")?.render_state();
                let state = render_state::material_render_state(&entities::declaration_text(bytes), base)?;
                let options = material_option_overrides(library, bytes)?;
                let depth = water::depth_sources(library, bytes, main, &options, state);
                let stencil=water::stencil_sources(library,bytes,main,&options,state);
                std::sync::Arc::make_mut(&mut self.water_stencil_materials.0).insert(path.into(),stencil);
                let parameters = water::colour_parameters(library, bytes, main)?;
                let world_colour = water::world_colour_source(library, bytes, main, &options, state);
                let model_colour = water::model_colour_source(library,bytes,main,&options,state);
                std::sync::Arc::make_mut(&mut self.water_model_colours.0).insert(path.into(),model_colour);
                Ok(std::sync::Arc::new(crate::rendering::bevy_backend::water_scene::WaterMaterialSource {
                    main_program: main.to_owned(),
                    state,
                    options,
                    depth,
                    declaration: bytes.into(),
                    parameters,
                    world_colour,
                }))
            })();
            if let Err(error) = &inputs { eprintln!("Water material unavailable {path}: {error}"); }
            std::sync::Arc::make_mut(&mut self.water_materials.0).insert(path.into(), inputs);
            MaterialHandle::Water(path.into())
        } else if let Some((depth_offset, depth_uv)) = sky_depth_parameters {
            let result = (|| -> Result<_, String> {
                let library = self
                    .programs
                    .as_deref_mut()
                    .ok_or("sky program library unavailable")?
                    .as_mut()
                    .map_err(|error| error.clone())?;
                let main = surface
                    .main_program
                    .as_deref()
                    .ok_or("sky program missing")?;
                let state = library
                    .definition(main.as_bytes())?
                    .ok_or("sky program definition missing")?
                    .render_state();
                let state =
                    render_state::material_render_state(&entities::declaration_text(bytes), state)?;
                let clouds = surface.program() == material_surface::Program::Clouds;
                Ok(crate::rendering::bevy_backend::sky_mask::SkyMaterial {
                    sky: uniform.sky,
                    depth_offset,
                    depth_uv,
                    clouds,
                    project_far: !clouds || uniform.sky[4].x != 0.,
                    state,
                    images: std::array::from_fn(|_| None),
                })
            })();
            match result {
                Ok(material) => MaterialHandle::Sky(self.sky_materials.add(material)),
                Err(error) => {
                    eprintln!("Sky render state unavailable {path}: {error}");
                    return None;
                }
            }
        } else if is_decal {
            MaterialHandle::Decal(self.decal_materials.add(DecalMaterial {
                base,
                extension: DecalExtension {
                    fog_parameters: self.fog_material.parameters.clone(),
                    fog_objects: self.fog_material.objects.clone(),
                    fog_world: self.fog_material.world.clone(),
                    fog_canvas: self.fog_material.canvas.clone(),
                    fog_curve: self.fog_material.curve.clone(),
                    fog_depth: self.fog_material.opaque_depth.clone(),
                    reflections: self.reflections.clone(),
                    reflection_brdf_lookup: self.reflection_brdf_lookup.clone(),
                    reflection_planes: self.reflection_planes.clone(),
                    material_exposure: self.material_exposure.clone(),
                    bias: decal_bias,
                    sh: self.probe_sh_buffer.clone(),
                    vertex_probes: self.vertex_probe_buffer.clone(),
                    shadow: self.shadow_buffer.clone(),
                    local_shadow: self.local_shadow_buffer.clone(),
                    shadow_map: Some(self.shadow_map.clone()),
                    local_shadow_map: Some(self.shadow_map.clone()),
                    screen_map: None,
                    cloud_map: None,
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
                    lightmap: None,
                    filling_diffuse: None,
                    filling_distortion1: None,
                    filling_distortion2: None,
                    filling_lookup: None,
                    filling_normal: None,
                    filling_fresnel: None,
                    cloud_opacity: None,
                    cloud_density: None,
                    cloud_back_density: None,
                    linear_clamp_image: Some(self.linear_clamp_image.clone()),
                },
            }))
        } else {
            MaterialHandle::Probe(self.probe_materials.add(ProbeMaterial {
                base,
                extension: ProbeExtension {
                    fog_parameters: self.fog_material.parameters.clone(),
                    fog_objects: self.fog_material.objects.clone(),
                    fog_world: self.fog_material.world.clone(),
                    fog_canvas: self.fog_material.canvas.clone(),
                    fog_curve: self.fog_material.curve.clone(),
                    fog_depth: self.fog_material.opaque_depth.clone(),
                    reflections: self.reflections.clone(),
                    reflection_brdf_lookup: self.reflection_brdf_lookup.clone(),
                    reflection_planes: self.reflection_planes.clone(),
                    material_exposure: self.material_exposure.clone(),
                    sh: self.probe_sh_buffer.clone(),
                    vertex_probes: self.vertex_probe_buffer.clone(),
                    shadow: self.shadow_buffer.clone(),
                    local_shadow: self.local_shadow_buffer.clone(),
                    local_shadow_map: Some(self.shadow_map.clone()),
                    shadow_map: Some(self.shadow_map.clone()),
                    screen_map: None,
                    cloud_map: None,
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
                    lightmap: None,
                    filling_diffuse: None,
                    filling_distortion1: None,
                    filling_distortion2: None,
                    filling_lookup: None,
                    filling_normal: None,
                    filling_fresnel: None,
                    cloud_opacity: None,
                    cloud_density: None,
                    cloud_back_density: None,
                    linear_clamp_image: Some(self.linear_clamp_image.clone()),
                    vegetation_slots: self.vegetation.slots.clone(),
                    vegetation_instances: self.vegetation.instances.clone(),
                },
            }))
        };
        if surface.program() == material_surface::Program::Vegetation {
            // The forward VS reads the same vegetAnim vectors as VS14200,
            // whichever shadow class (or none) this material resolves to.
            let parameters = (|| {
                let catalog = self
                    .catalog
                    .ok_or("vegetation parameter catalog is unavailable")?;
                let library = self
                    .programs
                    .as_deref_mut()
                    .ok_or("program library is unavailable")?
                    .as_mut()
                    .map_err(|error| error.clone())?;
                let p = resolve_shadow_material_parameters(
                    library,
                    catalog,
                    bytes,
                    shader_cache::CachedRigidShadowClass::Vegetation,
                    false,
                )?
                .vegetation(None)?;
                Ok([
                    p.main_variation[0],
                    p.main_frequency[0],
                    p.main_amplitude[0],
                    p.minimum_wind[0],
                    p.leaf_frequency[0],
                    p.leaf_amplitude[0],
                    p.branch_amplitude[0],
                ])
            })();
            self.vegetation.parameters.insert(handle.clone(), parameters);
        }
        let shadow = resolve_shadow_source_inputs(
            self.programs.as_deref_mut(),
            bytes,
            program,
            surface.main_program.as_deref(),
        )
        .map(|source| {
            let base_slot10_present = match self.catalog {
                Some(catalog) => resolve_shadow_slot_presence(
                    self.programs.as_deref_mut(), catalog, bytes, &source,
                    surface.main_program.as_deref(),
                ),
                None => Err("shadow material catalog unavailable".into()),
            };
            let rigid_base_source = resolve_rigid_shadow_source(
                self.programs.as_deref_mut(),
                bytes,
                &source,
                surface.main_program.as_deref(),
            );
            let world_source = resolve_world_shadow_source(
                self.programs.as_deref_mut(),
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
                        .ok_or("world shadow parameter catalog is unavailable")?;
                    let library = self
                        .programs
                        .as_deref_mut()
                        .ok_or("program library is unavailable")?
                        .as_mut()
                        .map_err(|error| error.clone())?;
                    let class = compiled.class.material_parameter_class();
                    resolve_shadow_material_parameters(library, catalog, bytes, class, false)
                        .map(Some)
                })(),
            };
            let parameters = match &rigid_base_source {
                Ok(None) => Ok(None),
                Err(error) => Err(error.clone()),
                Ok(Some(compiled)) => (|| {
                    let catalog = self
                        .catalog
                        .ok_or("shadow parameter catalog is unavailable")?;
                    let library = self
                        .programs
                        .as_deref_mut()
                        .ok_or("program library is unavailable")?
                        .as_mut()
                        .map_err(|error| error.clone())?;
                    resolve_shadow_material_parameters(
                        library,
                        catalog,
                        bytes,
                        compiled.class,
                        true,
                    )
                    .map(Some)
                })(),
            };
            ShadowMaterialInputs {
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
            vertex_lighting,
            stage_sort,
            compatible_geometry_routes: boat_routes[0] == boat_routes[1],
        };
        self.cache.insert(key, geometry, Some(cached.clone()));
        let probe_index = self.append_probe_record(game_position);
        Some(ResolvedMaterial {
            handle: cached.handle,
            diffuse_image_slot: cached.diffuse_image_slot,
            probe_index,
            vertex_lighting: cached.vertex_lighting,
        })
    }
}
