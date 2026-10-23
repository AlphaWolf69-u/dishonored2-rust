//! Read-only installed shader-cache data, not archived `.vbin` files.
//!
//! Evidence: Dishonored2.exe 1402ca9c0 (v14 indices), 1402cc070
//! (shared names), 1402c9f00 (variant payload), 1402ca440 (stage payload),
//! and 140143650 (ASCII-only case folding, NUL-terminated lookup).
//! This ports synchronous data access, NOT optional preload, GPU-object
//! creation, source/include validation, or render-program bank publication.

use flate2::read::DeflateDecoder;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;
mod water;
mod boat;
pub use water::{CachedWaterDepthClass, CachedWaterDepthSource};
const VERSION: u32 = 14;

/// Cache slots 0/1, NOT the render-program platform enum's PC/PC_D3D12 IDs.
#[derive(Clone, Copy, Debug)]
pub enum ShaderCacheBackend {
    Pc,
    D3d12Pc,
}

impl ShaderCacheBackend {
    fn stem(self) -> &'static str {
        match self {
            Self::Pc => "ShaderCache_PC",
            Self::D3d12Pc => "ShaderCache_D3D12_PC",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheSpan {
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug)]
pub struct CachedProgramIndex {
    pub name: Vec<u8>,
    pub definition: CacheSpan,
    pub variants: HashMap<u32, CacheSpan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CachedStageIndex {
    pub offset: u32,
    pub compressed_size: u32,
    /// Native record +7. Kept opaque; this is NOT a program/stage ID.
    pub flags: u8,
}

#[derive(Debug)]
pub struct CachedStageBytes {
    pub flags: u8,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct ShaderCache {
    pub names: Vec<Vec<u8>>,
    pub programs: HashMap<Vec<u8>, CachedProgramIndex>,
    pub stages: Vec<CachedStageIndex>,
    pub stage_counts: [u32; 2],
    // Keep the large data streams on disk; indices alone are resident.
    definitions: File,
    variants: File,
    bytecode: File,
}

fn name_key(name: &[u8]) -> Vec<u8> {
    name.iter()
        .copied()
        .take_while(|&b| b != 0)
        .map(|b| b.to_ascii_lowercase())
        .collect()
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let mut cursor = Self { bytes, at: 0 };
        let version = cursor.u32()?;
        if version != VERSION {
            return Err(format!("unsupported shader-cache version {version}"));
        }
        Ok(cursor)
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(length)
            .ok_or("shader-cache offset overflow")?;
        let bytes = self
            .bytes
            .get(self.at..end)
            .ok_or("truncated shader-cache index")?;
        self.at = end;
        Ok(bytes)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn name(&mut self) -> Result<Vec<u8>> {
        let length = self.u32()? as usize;
        Ok(self.take(length)?.to_vec())
    }
    fn span(&mut self) -> Result<CacheSpan> {
        Ok(CacheSpan {
            offset: self.u32()?,
            length: self.u32()?,
        })
    }
}

impl ShaderCache {
    /// `base` is the installed game's `base/`, not `base/shaderCache/`.
    pub fn open(base: &Path, backend: ShaderCacheBackend) -> Result<Self> {
        let directory = base.join("shaderCache");
        let path = |extension| directory.join(format!("{}.{}", backend.stem(), extension));
        let read = |extension| {
            let path = path(extension);
            std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))
        };
        let open = |extension| {
            let path = path(extension);
            File::open(&path).map_err(|error| format!("{}: {error}", path.display()))
        };
        Self::from_indices(
            &read("rpi")?,
            &read("rpvi")?,
            &read("uci")?,
            &read("str")?,
            open("rpd")?,
            open("rpvd")?,
            open("ucd")?,
        )
    }

    fn from_indices(
        rpi: &[u8],
        rpvi: &[u8],
        uci: &[u8],
        strings: &[u8],
        definitions: File,
        variants: File,
        bytecode: File,
    ) -> Result<Self> {
        let mut strings = Cursor::new(strings)?;
        let mut names = Vec::new();
        for _ in 0..strings.u16()? {
            names.push(strings.name()?);
        }

        let mut stages = Cursor::new(uci)?;
        // Both counts precede entries. Native row ID is its ARRAY INDEX.
        let stage_counts = [stages.u32()?, stages.u32()?];
        let count = stage_counts[0]
            .checked_add(stage_counts[1])
            .ok_or("shader stage count overflow")?;
        let mut stage_index = Vec::new();
        for _ in 0..count {
            let offset = stages.u32()?;
            let packed = stages.u32()?;
            stage_index.push(CachedStageIndex {
                offset,
                compressed_size: packed >> 8,
                flags: packed as u8,
            });
        }

        let mut programs = HashMap::new();
        let mut program_index = Cursor::new(rpi)?;
        let mut variant_index = Cursor::new(rpvi)?;
        for _ in 0..program_index.u32()? {
            let name = program_index.name()?;
            let definition = program_index.span()?;
            // The original consumes but does not compare this second name.
            let _variant_group_name = variant_index.name()?;
            let mut variants = HashMap::new();
            for _ in 0..variant_index.u32()? {
                variants.insert(variant_index.u32()?, variant_index.span()?);
            }
            // Native insertion reuses an existing case-insensitive name entry.
            let program = programs
                .entry(name_key(&name))
                .or_insert_with(|| CachedProgramIndex {
                    name,
                    definition,
                    variants: HashMap::new(),
                });
            program.definition = definition;
            program.variants.extend(variants);
        }
        Ok(Self {
            names,
            programs,
            stages: stage_index,
            stage_counts,
            definitions,
            variants,
            bytecode,
        })
    }

    pub fn program(&self, name: &[u8]) -> Option<&CachedProgramIndex> {
        self.programs.get(&name_key(name))
    }

    /// RPD program-definition data; its own format is not a D3D variant.
    pub fn raw_definition(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>> {
        let Some(span) = self
            .program(name)
            .map(|p| p.definition)
            .filter(|s| s.length != 0)
        else {
            return Ok(None);
        };
        read_span(
            &mut self.definitions,
            u64::from(span.offset),
            span.length as usize,
        )
        .map(Some)
    }

    /// Exact variant-hash lookup; absent/zero-length entries stay unresolved.
    pub fn variant(&mut self, name: &[u8], hash: u32) -> Result<Option<Vec<u8>>> {
        let Some(span) = self
            .program(name)
            .and_then(|p| p.variants.get(&hash))
            .copied()
            .filter(|s| s.length != 0)
        else {
            return Ok(None);
        };
        let header = read_span(&mut self.variants, u64::from(span.offset), 8)?;
        let compressed = u32::from_le_bytes(header[..4].try_into().unwrap());
        let unpacked = u32::from_le_bytes(header[4..].try_into().unwrap());
        let bytes = read_span(
            &mut self.variants,
            u64::from(span.offset) + 8,
            compressed as usize,
        )?;
        inflate(&bytes, unpacked).map(Some)
    }

    /// 1402ca440: UCI supplies compressed size/flags; UCD supplies unpacked size.
    pub fn stage(&mut self, id: usize) -> Result<CachedStageBytes> {
        let entry = *self
            .stages
            .get(id)
            .ok_or_else(|| format!("shader stage ID {id} out of range"))?;
        let header = read_span(&mut self.bytecode, u64::from(entry.offset), 4)?;
        let unpacked = u32::from_le_bytes(header[..].try_into().unwrap());
        let bytes = read_span(
            &mut self.bytecode,
            u64::from(entry.offset) + 4,
            entry.compressed_size as usize,
        )?;
        Ok(CachedStageBytes {
            flags: entry.flags,
            bytes: inflate(&bytes, unpacked)?,
        })
    }
}

/// Bytecode-identified shadow source (legacy type name), not an engine pointer
/// or fully bound native program. Material draw-state overrides and frame ownership
/// are still required; source/include identity validation is not implied.
#[derive(Clone, Debug)]
pub struct CachedRigidShadowSource {
    /// Identified shader class, never a substitute opaque fallback.
    pub class: CachedRigidShadowClass,
    /// Deformation is independent of the fragment class; never use a rigid
    /// pipeline merely because both fragments are opaque.
    pub skin_influences: u8,
    pub alpha_dither: bool,
    pub program: Vec<u8>,
    pub variant_hash: u32,
    pub definition_render_state: [u32; 9],
    pub variant: crate::render_program::D3dVariantData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedRigidShadowClass {
    Opaque,
    AlphaTested,
    VistaAlpha,
    Vegetation,
}

/// Instanced static-world programs are not the rigid-model CB/input class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedWorldShadowClass {
    Opaque,
    AlphaTested,
    VistaAlpha,
    Vegetation,
    VegetationFixed,
}

impl CachedWorldShadowClass {
    /// Material UV/wind variables are shared by the installed rigid/world
    /// banks; their transform/wind GPU ABIs are deliberately NOT shared.
    pub fn material_parameter_class(self) -> CachedRigidShadowClass {
        match self {
            Self::Opaque => CachedRigidShadowClass::Opaque,
            Self::AlphaTested | Self::VistaAlpha => CachedRigidShadowClass::AlphaTested,
            Self::Vegetation | Self::VegetationFixed => CachedRigidShadowClass::Vegetation,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CachedWorldShadowSource {
    pub class: CachedWorldShadowClass,
    pub program: Vec<u8>,
    pub variant_hash: u32,
    pub definition_render_state: [u32; 9],
    pub variant: crate::render_program::D3dVariantData,
}

fn is_captured_rigid_opaque_stage(bytes: &[u8]) -> bool {
    // The liquid-container VS has extra unused varyings and a different CB
    // layout, but its only rasterized output is exactly the same rigid MVP
    // and depth multiplier.768 native stream-output positions verify the
    // canonical transport shared with Vista (no liquid pixel stage exists).
    if bytes.get(..4) == Some(b"DXBC".as_slice())
        && bytes.get(4..20)
            == Some(
                [
                    0x17, 0xfa, 0x7d, 0x8c, 0x1d, 0x94, 0x16, 0xae, 0xb8, 0x75, 0x94, 0x3a, 0x6c,
                    0x9e, 0xe7, 0x9a,
                ]
                .as_slice(),
            )
    {
        return true;
    }
    // Actual DXBC checksum of captured VS17296, NOT its transient resource ID.
    bytes.get(..4) == Some(b"DXBC".as_slice())
        && bytes.get(4..20)
            == Some(
                [
                    0x78, 0x6c, 0x59, 0xbe, 0xb2, 0xf0, 0x23, 0x7b, 0x2f, 0xf5, 0x1a, 0x92, 0x02,
                    0xe6, 0xb1, 0x29,
                ]
                .as_slice(),
            )
}

/// Resident declaration metadata plus lazy decoded definitions. These are
/// original serialized programs, NOT validated GPU objects or a shadow bank.
/// Lookups do no archive scans after startup; definitions decode once.
#[derive(Debug)]
pub struct ProgramLibrary {
    pub cache: ShaderCache,
    pub parameter_kinds: HashMap<Vec<u8>, crate::render_program::RenderParameterKind>,
    option_defaults: HashMap<Vec<u8>, i32>,
    float_vector_defaults: HashMap<Vec<u8>, Result<[u32; 4]>>,
    definitions: HashMap<Vec<u8>, crate::render_program::RenderProgramDefinition>,
    rigid_shadow_sources:
        HashMap<(Vec<u8>, u32), Result<Option<std::sync::Arc<CachedRigidShadowSource>>>>,
    world_shadow_sources:
        HashMap<(Vec<u8>, u32), Result<Option<std::sync::Arc<CachedWorldShadowSource>>>>,
    water_depth_sources:
        HashMap<(Vec<u8>, u32), Result<Option<std::sync::Arc<CachedWaterDepthSource>>>>,
}

impl ProgramLibrary {
    pub fn open(
        base: &Path,
        backend: ShaderCacheBackend,
        catalog: &crate::archive::Catalog,
    ) -> Result<Self> {
        let cache = ShaderCache::open(base, backend)?;
        let parameter_kinds = crate::render_program::render_parameter_schema(catalog)?;
        let mut option_defaults = HashMap::new();
        for (name, kind) in &parameter_kinds {
            if *kind != crate::render_program::RenderParameterKind::Option {
                continue;
            }
            let path = format!(
                "generated/decls/renderparm/{}.decl",
                std::str::from_utf8(name).map_err(|e| e.to_string())?
            );
            let bytes = catalog
                .read(&path)
                .map_err(|error| format!("{path}: {error}"))?
                .ok_or_else(|| format!("missing option declaration {path}"))?;
            let value =
                crate::entities::declaration_option_default(&String::from_utf8_lossy(&bytes))
                    .map_err(|error| format!("{path}: {error}"))?;
            option_defaults.insert(name.clone(), value);
        }
        Ok(Self {
            cache,
            parameter_kinds,
            option_defaults,
            float_vector_defaults: HashMap::new(),
            definitions: HashMap::new(),
            rigid_shadow_sources: HashMap::new(),
            world_shadow_sources: HashMap::new(),
            water_depth_sources: HashMap::new(),
        })
    }

    pub fn definition(
        &mut self,
        name: &[u8],
    ) -> Result<Option<&crate::render_program::RenderProgramDefinition>> {
        let key = name_key(name);
        if !self.definitions.contains_key(&key) {
            let Some(bytes) = self.cache.raw_definition(name)? else {
                return Ok(None);
            };
            let (definition, tail) =
                crate::render_program::parse_program_definition(&bytes, |name| {
                    self.parameter_kinds
                        .get(&name_key(name))
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "unresolved parameter type {}",
                                String::from_utf8_lossy(name)
                            )
                        })
                })?;
            if !tail.is_empty() {
                return Err("trailing program-definition data".into());
            }
            self.definitions.insert(key.clone(), definition);
        }
        Ok(self.definitions.get(&key))
    }

    fn option_default(&self, name: &[u8]) -> Result<i32> {
        let key = name_key(name);
        if self.parameter_kinds.get(&key)
            != Some(&crate::render_program::RenderParameterKind::Option)
        {
            return Err(format!(
                "not a registered Option: {}",
                String::from_utf8_lossy(name)
            ));
        }
        self.option_defaults
            .get(&key)
            .copied()
            .ok_or("missing registered Option default".into())
    }

    /// Actual floating declaration initializer, decoded once. Integer/bool,
    /// expression, and named-reference initializers remain explicit errors.
    /// No default is guessed from the parameter's spelling or forward material.
    pub fn registered_float_vector(
        &mut self,
        catalog: &crate::archive::Catalog,
        overrides: &crate::render_program::SerializedParameterBlock,
        name: &[u8],
        fallback: Option<&crate::render_program::SerializedParameterBlock>,
    ) -> Result<[u32; 4]> {
        let key = name_key(name);
        if self.parameter_kinds.get(&key)
            != Some(&crate::render_program::RenderParameterKind::Uniform)
        {
            return Err(format!(
                "not a registered floating Uniform: {}",
                String::from_utf8_lossy(name)
            ));
        }
        if !self.float_vector_defaults.contains_key(&key) {
            let path = format!(
                "generated/decls/renderparm/{}.decl",
                std::str::from_utf8(&key).map_err(|error| error.to_string())?
            );
            let result = (|| {
                let bytes = catalog
                    .read(&path)
                    .map_err(|error| format!("{path}: {error}"))?
                    .ok_or_else(|| format!("missing floating Uniform declaration {path}"))?;
                crate::entities::declaration_float_vector_default(&String::from_utf8_lossy(&bytes))
                    .map_err(|error| format!("{path}: {error}"))
            })();
            self.float_vector_defaults.insert(key.clone(), result);
        }
        // The parsed header above proves native kind Uniform/subtype float.
        overrides.vector_words(
            &key,
            0x30,
            self.float_vector_defaults[&key].clone()?,
            fallback,
        )
    }

    /// 2d5210 first merges MAIN-program parms, then authored material parms,
    /// and stores stageSort/Adjust separately at material+2c0/+2c4. Named
    /// stageSort constants resolve through the actual render-parameter registry.
    pub fn material_stage_sort_parameters(
        &mut self,
        catalog: &crate::archive::Catalog,
        main_program: &[u8],
        source: &str,
    ) -> Result<[u32; 2]> {
        let fallback = self
            .definition(main_program)?
            .ok_or("stage sort has no MAIN program definition")?
            .defaults
            .clone();
        let empty = crate::render_program::SerializedParameterBlock {
            flags: 0,
            parameters: Vec::new(),
        };
        let values = crate::entities::material_float_parameters_with_stage_sort(
            source,
            |name| matches!(name, "stagesort" | "stagesortadjust"),
            |name| self.registered_float_vector(catalog, &empty, name.as_bytes(), None),
        )?;
        let overrides = crate::render_program::SerializedParameterBlock {
            flags: 0,
            parameters: values
                .into_iter()
                .map(|(name, words)| crate::render_program::SerializedParameter {
                    name: name.into_bytes(),
                    kind: crate::render_program::RenderParameterKind::Uniform,
                    value: crate::render_program::SerializedParameterValue::Words(words),
                })
                .collect(),
        };
        let value =
            self.registered_float_vector(catalog, &overrides, b"stagesort", Some(&fallback))?[0];
        let adjust =
            self.registered_float_vector(catalog, &overrides, b"stagesortadjust", Some(&fallback))?
                [0];
        Ok([value, adjust])
    }

    pub fn material_stage_sort(
        &mut self,
        catalog: &crate::archive::Catalog,
        main_program: &[u8],
        source: &str,
    ) -> Result<[u32; 2]> {
        let mut result = self.material_stage_sort_parameters(catalog, main_program, source)?;
        // 2d6f39: MAIN==arkWater sets +48 bit5 and forces the final sort
        // to sortWater, after authored overrides. The adjustment is retained.
        if main_program.eq_ignore_ascii_case(b"arkWater") {
            let empty = crate::render_program::SerializedParameterBlock {
                flags: 0,
                parameters: Vec::new(),
            };
            result[0] = self.registered_float_vector(catalog, &empty, b"sortwater", None)?[0];
        }
        Ok(result)
    }

    /// Runtime list boundaries copied by 140351210 to renderer+5fa318.
    /// The final sentinel is the executable's 0x7149f2ca float constant.
    pub fn material_sort_thresholds(
        &mut self,
        catalog: &crate::archive::Catalog,
    ) -> Result<[f32; 10]> {
        let empty = crate::render_program::SerializedParameterBlock {
            flags: 0,
            parameters: Vec::new(),
        };
        let mut values = [0.; 10];
        for (index, name) in [
            "sortboatvolume",
            "sortemit",
            "sortemitonly",
            "sortshadowwalk",
            "sortdecal",
            "sortwater",
            "sortdarkvision",
            "sorthud",
            "sortperturber",
        ]
        .into_iter()
        .enumerate()
        {
            values[index] = f32::from_bits(
                self.registered_float_vector(catalog, &empty, name.as_bytes(), None)?[0],
            );
        }
        values[9] = f32::from_bits(0x7149f2ca);
        Ok(values)
    }

    /// 140342a50 with a null fallback: material overrides then the registered
    /// declaration default. In particular, decalType does NOT use the RPD.
    pub fn registered_option_value(
        &self,
        overrides: &crate::render_program::SerializedParameterBlock,
        name: &[u8],
    ) -> Result<i32> {
        overrides.integer_value(name, 12, self.option_default(name)? as u32, None)
    }

    /// 140342a50's valid Option branch is the same last-wins raw-word lookup
    /// as integer_value(kind=12), including the selected main-program fallback.
    /// The non-Option diagnostic path is not a supported value conversion.
    pub fn option_value(
        &mut self,
        program: &[u8],
        overrides: &crate::render_program::SerializedParameterBlock,
        name: &[u8],
    ) -> Result<i32> {
        let default = self.option_default(name)?;
        if self.definition(program)?.is_none() {
            return Err("missing main-program definition".into());
        }
        overrides.integer_value(
            name,
            12,
            default as u32,
            Some(&self.definitions[&name_key(program)].specialization_defaults),
        )
    }

    /// 1402be900: actual ordered bindings, registry defaults, definition's
    /// fallback block, then the caller's overrides. Unsupported defaults are
    /// reported, not replaced with zero or a nearby cached shader variant.
    pub fn specialization_hash(
        &mut self,
        name: &[u8],
        overrides: &crate::render_program::SerializedParameterBlock,
    ) -> Result<Option<u32>> {
        if self.definition(name)?.is_none() {
            return Ok(None);
        }
        let definition = &self.definitions[&name_key(name)];
        let names = definition.specialization_names(&self.cache.names)?;
        let mut parameters = Vec::with_capacity(names.len());
        for name in names {
            let default = self.option_defaults.get(&name_key(name)).ok_or_else(|| {
                format!(
                    "unresolved registered specialization default {}",
                    String::from_utf8_lossy(name)
                )
            })?;
            let value = overrides.integer_value(
                name,
                12,
                *default as u32,
                Some(&definition.specialization_defaults),
            )?;
            parameters.push((name, value));
        }
        crate::render_program::shader_variant_hash(&parameters).map(Some)
    }

    /// Resolve actual caller overrides and require exact already-ported bytecode
    /// pairs: rigid opaque/cutout or verified vegetation VS14200/PS14201. Class
    /// membership never comes from a program name. Other wind, skin, vista or
    /// paraboloid programs are not replaced with a superficially similar pair.
    /// Both successful and unsupported results are memoized by native signature.
    pub fn rigid_shadow_source(
        &mut self,
        name: &[u8],
        overrides: &crate::render_program::SerializedParameterBlock,
    ) -> Result<Option<std::sync::Arc<CachedRigidShadowSource>>> {
        let Some(hash) = self.specialization_hash(name, overrides)? else {
            return Ok(None);
        };
        let key = (name_key(name), hash);
        if let Some(result) = self.rigid_shadow_sources.get(&key) {
            return result.clone();
        }
        let result = (|| {
            let Some(variant) = self.serialized_variant(name, overrides)? else {
                return Ok(None);
            };
            let mut vertex = None;
            let mut pixel = None;
            let mut opaque_vertex = false;
            for stage in &variant.stages {
                let bytes = match &stage.bytecode {
                    crate::render_program::StageBytecode::Absent => continue,
                    crate::render_program::StageBytecode::Inline(bytes) => bytes.clone(),
                    crate::render_program::StageBytecode::Cached(id) => {
                        self.cache.stage(*id as usize)?.bytes
                    }
                };
                if bytes.get(..4) != Some(b"DXBC".as_slice()) || bytes.len() < 32 {
                    return Err("invalid rigid shadow DXBC container".into());
                }
                if u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize != bytes.len() {
                    return Err("rigid shadow DXBC extent differs from resident payload".into());
                }
                let checksum: [u8; 16] = bytes[4..20].try_into().unwrap();
                match stage.kind {
                    0 if vertex.is_none() => {
                        opaque_vertex = is_captured_rigid_opaque_stage(&bytes);
                        vertex = Some(checksum);
                    }
                    1 if pixel.is_none() => pixel = Some(checksum),
                    _ => {
                        return Err(format!(
                            "unsupported/duplicate rigid shadow stage {}",
                            stage.kind
                        ))
                    }
                }
            }
            // Resident alpha-test sources were resolved with alphaTest=1,
            // forcePixelShader=1 and disassembled offline. This exact pair
            // passes raw UV in VS, applies layer0 scale/bias in PS, samples
            // bilinearly and rejects alpha strictly below 0.5.
            let alpha_vertex = [
                0x31, 0x18, 0xdb, 0x42, 0xc4, 0x80, 0x60, 0x43, 0x41, 0x8e, 0x19, 0x98, 0x13, 0x5f,
                0x22, 0xaf,
            ];
            let alpha_pixel = [
                0x8b, 0xeb, 0x07, 0x0d, 0x4c, 0x4f, 0xbf, 0x16, 0x4d, 0x48, 0x86, 0x38, 0x16, 0x4f,
                0xd4, 0x6a,
            ];
            // The actual vegetation pair has a different 208-byte model CB,
            // raw color/normal inputs and native wind deformation. Its port is
            // checked against installed VS stream output, then indexed D16 draws.
            let vegetation_vertex = [
                0xf7, 0xdc, 0x89, 0xe4, 0xf8, 0xfb, 0xd6, 0xcd, 0x18, 0xa2, 0x2d, 0xdf, 0xb5, 0x2b,
                0x5d, 0xd0,
            ];
            let vegetation_pixel = [
                0xee, 0xd2, 0xfa, 0x0b, 0x2d, 0xfe, 0xbc, 0x1a, 0xf1, 0xc1, 0x87, 0xe5, 0xdd, 0x8b,
                0x45, 0xb6,
            ];
            let skin4 = [
                0x00, 0x05, 0x02, 0x61, 0xd3, 0xfb, 0x45, 0x65, 0xe6, 0xbc, 0x40, 0xbe, 0xa9, 0x40,
                0xec, 0xe7,
            ];
            let skin8 = [
                0x65, 0xc7, 0xb3, 0xa3, 0xd4, 0x2d, 0x7f, 0x48, 0x7f, 0x11, 0x05, 0xf2, 0x25, 0x03,
                0xf3, 0x09,
            ];
            let skin4_uv = [
                0x3a, 0xd6, 0x27, 0x69, 0xe0, 0x1a, 0xf2, 0xa2, 0x22, 0x71, 0x77, 0x74, 0x2d, 0xaa,
                0xbf, 0xb1,
            ];
            let skin8_uv = [
                0x4b, 0x77, 0x87, 0xe3, 0x75, 0x83, 0xb6, 0x2a, 0xcd, 0x71, 0x1d, 0x88, 0x2c, 0xfd,
                0x01, 0x37,
            ];
            let skin4_extended = [
                0xf8, 0x96, 0x70, 0xf6, 0xf1, 0xa0, 0x54, 0xe6, 0xb7, 0x41, 0x0d, 0xb8, 0x3a, 0x7f,
                0x04, 0x2e,
            ];
            let skin8_extended = [
                0xe6, 0xf3, 0xc8, 0x28, 0x02, 0x41, 0xeb, 0xe5, 0x8e, 0xd7, 0xc1, 0xe8, 0x17, 0x71,
                0xf0, 0x87,
            ];
            let skin8_dual_uv = [
                0xfb, 0x88, 0x0d, 0x5d, 0x5d, 0x1c, 0x1e, 0xd6, 0x1e, 0xf9, 0xda, 0xf3, 0xeb, 0x4a,
                0x4c, 0x43,
            ];
            let skin4_world_outputs = [
                0xbc, 0x2c, 0x56, 0x91, 0x7a, 0x73, 0xd5, 0xe0, 0x97, 0xd6, 0x9a, 0x90, 0x49, 0x6a,
                0xe5, 0xbe,
            ];
            let skin_alpha = [
                0x07, 0x9f, 0x7d, 0x69, 0x5d, 0x3a, 0x52, 0x8d, 0x2e, 0x22, 0xd1, 0x33, 0xdb, 0x3f,
                0x29, 0x92,
            ];
            let skin_dither = [
                0x67, 0x63, 0xe0, 0x76, 0xad, 0x91, 0x20, 0xa3, 0x1a, 0xc3, 0xe4, 0x30, 0x4c, 0x99,
                0x18, 0x3b,
            ];
            let skin_influences = if [skin4, skin4_uv, skin4_extended, skin4_world_outputs]
                .into_iter()
                .any(|hash| vertex == Some(hash))
            {
                4
            } else if [skin8, skin8_uv, skin8_extended, skin8_dual_uv]
                .into_iter()
                .any(|hash| vertex == Some(hash))
            {
                8
            } else {
                0
            };
            let alpha_dither = pixel == Some(skin_dither);
            let class = if skin_influences != 0 && pixel.is_none() {
                CachedRigidShadowClass::Opaque
            } else if skin_influences != 0
                && (pixel == Some(skin_alpha) || alpha_dither)
                && [
                    skin4_uv,
                    skin8_uv,
                    skin4_extended,
                    skin8_extended,
                    skin8_dual_uv,
                ]
                .into_iter()
                .any(|hash| vertex == Some(hash))
            {
                CachedRigidShadowClass::AlphaTested
            } else if opaque_vertex && pixel.is_none() {
                CachedRigidShadowClass::Opaque
            } else if vertex == Some(alpha_vertex) && pixel == Some(alpha_pixel) {
                CachedRigidShadowClass::AlphaTested
            } else if vertex
                == Some([
                    0x8f, 0xb0, 0xe7, 0xab, 0xaf, 0x5b, 0x64, 0xb6, 0xcb, 0x28, 0xd3, 0xbd, 0x14,
                    0x4e, 0x5a, 0x99,
                ])
                && pixel
                    == Some([
                        0x1a, 0x8e, 0xaf, 0x20, 0x45, 0xc7, 0x0c, 0x56, 0x8c, 0x1e, 0xdb, 0x69,
                        0x7b, 0x3d, 0xd2, 0x1b,
                    ])
            {
                CachedRigidShadowClass::VistaAlpha
            } else if vertex == Some(vegetation_vertex) && pixel == Some(vegetation_pixel) {
                CachedRigidShadowClass::Vegetation
            } else {
                if let Some(directory) = std::env::var_os("DH2_EXPORT_UNSUPPORTED_SHADOWS") {
                    let directory = Path::new(&directory);
                    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
                    for stage in &variant.stages {
                        let bytes = match &stage.bytecode {
                            crate::render_program::StageBytecode::Absent => continue,
                            crate::render_program::StageBytecode::Inline(bytes) => bytes.clone(),
                            crate::render_program::StageBytecode::Cached(id) => {
                                self.cache.stage(*id as usize)?.bytes
                            }
                        };
                        let stem = String::from_utf8_lossy(name).replace(['/', '\\'], "_");
                        std::fs::write(
                            directory.join(format!("{stem}-{hash:08x}-{}.dxbc", stage.kind)),
                            bytes,
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
                return Err(format!(
                    "unsupported shadow bytecode pair for {} signature={hash:08x} VS={vertex:02x?} PS={pixel:02x?}",
                    String::from_utf8_lossy(name),
                ));
            };
            let definition = &self.definitions[&name_key(name)];
            if variant.header.source_crc != definition.source_crc {
                return Err(
                    "compiled shadow source CRC differs from its resident definition".into(),
                );
            }
            let state = definition.render_state();
            Ok(Some(std::sync::Arc::new(CachedRigidShadowSource {
                class,
                skin_influences,
                alpha_dither,
                program: name.to_vec(),
                variant_hash: hash,
                definition_render_state: state,
                variant,
            })))
        })();
        self.rigid_shadow_sources.insert(key, result.clone());
        result
    }

    /// Resolve actual world caller overrides. Only the installed, disassembled
    /// opaque and cutout pairs with their original instance-buffer ABI qualify.
    /// None remains a resident-cache MISS, never a no-shadow decision.
    pub fn world_shadow_source(
        &mut self,
        name: &[u8],
        overrides: &crate::render_program::SerializedParameterBlock,
    ) -> Result<Option<std::sync::Arc<CachedWorldShadowSource>>> {
        let Some(hash) = self.specialization_hash(name, overrides)? else {
            return Ok(None);
        };
        let key = (name_key(name), hash);
        if let Some(result) = self.world_shadow_sources.get(&key) {
            return result.clone();
        }
        let result = (|| {
            let Some(variant) = self.serialized_variant(name, overrides)? else {
                return Ok(None);
            };
            let mut vertex = None;
            let mut pixel = None;
            for stage in &variant.stages {
                let bytes = match &stage.bytecode {
                    crate::render_program::StageBytecode::Absent => continue,
                    crate::render_program::StageBytecode::Inline(bytes) => bytes.clone(),
                    crate::render_program::StageBytecode::Cached(id) => {
                        self.cache.stage(*id as usize)?.bytes
                    }
                };
                if bytes.len() < 32
                    || bytes.get(..4) != Some(b"DXBC".as_slice())
                    || u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize != bytes.len()
                {
                    return Err("invalid world shadow DXBC extent/container".into());
                }
                // Explicit offline test capture only; never writes game data
                // from production. Capture unsupported pairs before rejection
                // so the next class is ported from its real installed shader.
                #[cfg(test)]
                if let Some(directory) = std::env::var_os("DH2_WORLD_SHADER_EVIDENCE") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                    let file = directory.join(format!("{hash:08x}-{}.dxbc", stage.kind));
                    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
                    eprintln!(
                        "WORLD EVIDENCE {} {}",
                        String::from_utf8_lossy(name),
                        file.display()
                    );
                }
                let checksum: [u8; 16] = bytes[4..20].try_into().unwrap();
                match stage.kind {
                    0 if vertex.is_none() => vertex = Some(checksum),
                    1 if pixel.is_none() => pixel = Some(checksum),
                    _ => {
                        return Err(format!(
                            "unsupported/duplicate world shadow stage {}",
                            stage.kind
                        ))
                    }
                }
            }
            let class = match (vertex, pixel) {
                // The UV-exporting VS has the identical position instructions
                // (CB1[2..5], instance remap and 48-byte model rows). With no PS
                // its extra UV varying is dead: transport the same opaque depth
                // function through our position-only backend ABI. Its original
                // CB2 instance offset is at word4 rather than word0; neither
                // original CB is rebound directly to this rewrite-authored ABI.
                (Some([0x63,0xab,0xa4,0xea,0xd9,0xc2,0xc3,0xaa,0xb2,0x63,0xa3,0x6a,0xe8,0xed,0x36,0x2c]), None) |
                (Some([0x65,0x82,0x5b,0x7d,0xdd,0x6c,0x6e,0xc0,0xba,0x7a,0x54,0x34,0x47,0x18,0xc7,0xce]), None) =>
                    CachedWorldShadowClass::Opaque,
                (Some([0x63,0xab,0xa4,0xea,0xd9,0xc2,0xc3,0xaa,0xb2,0x63,0xa3,0x6a,0xe8,0xed,0x36,0x2c]),
                 Some([0x2c,0xdc,0xb1,0x21,0x51,0x4d,0xf8,0x98,0x80,0x79,0x97,0xea,0x08,0xc1,0xd6,0x94])) =>
                    CachedWorldShadowClass::AlphaTested,
                // Native arkvista's raw-UV cutout and explicitly declared
                // anisotropicsampler; not the Layer0/bilinear alpha program.
                (Some([0xdf,0xf3,0xe1,0xd3,0xec,0xb4,0xd9,0x6f,0x7c,0xe6,0x76,0xa3,0x52,0x63,0x1c,0x44]),
                 Some([0x1a,0x8e,0xaf,0x20,0x45,0xc7,0x0c,0x56,0x8c,0x1e,0xdb,0x69,0x7b,0x3d,0xd2,0x1b])) =>
                    CachedWorldShadowClass::VistaAlpha,
                // Actual installed world-wind VS/PS, not rigid VS14200.
                // Stream-output verified in world_shadow_wind_cases.
                (Some([0xe5,0xee,0xdb,0x97,0x8c,0xbe,0x4c,0xee,0x3d,0x57,0xcf,0x15,0xf8,0x61,0x9f,0x90]),
                 Some([0x75,0x42,0x36,0x0b,0x4a,0x74,0xed,0x4a,0x15,0xbf,0x8d,0xf5,0x2e,0xb3,0xd4,0x4b])) =>
                    CachedWorldShadowClass::Vegetation,
                // Separately captured 51-slot variant: real fixed local wind,
                // no wind-buffer reads. Never substituted for the pair above.
                (Some([0x11,0xd8,0xd0,0xa6,0x07,0x10,0x97,0x61,0xd3,0x2b,0x13,0x47,0x53,0x5c,0xf8,0x3f]),
                 Some([0x76,0xbf,0x50,0x91,0x0b,0x56,0xd0,0xe0,0xe7,0xd3,0xa9,0xd4,0x35,0x9e,0x6e,0x1c])) |
                // Same fixed-wind family with hasBumpMap enabled. BOTH stages
                // have byte-identical SHEX, ISGN, OSGN and STAT to the pair
                // above; only non-executed RDEF reflection differs. This is
                // shader-family coverage, never a material-name exception.
                (Some([0x81,0x04,0x6a,0xaa,0xd8,0x79,0xf0,0x17,0xd1,0x6d,0x2e,0xef,0x4a,0xfc,0xb1,0x67]),
                 Some([0x75,0x42,0x36,0x0b,0x4a,0x74,0xed,0x4a,0x15,0xbf,0x8d,0xf5,0x2e,0xb3,0xd4,0x4b])) =>
                    CachedWorldShadowClass::VegetationFixed,
                _ => return Err(format!("unsupported world shadow bytecode pair for {}: VS={vertex:02x?} PS={pixel:02x?}",String::from_utf8_lossy(name))),
            };
            let definition = &self.definitions[&name_key(name)];
            if variant.header.source_crc != definition.source_crc {
                return Err("compiled world shadow CRC differs from resident definition".into());
            }
            Ok(Some(std::sync::Arc::new(CachedWorldShadowSource {
                class,
                program: name.to_vec(),
                variant_hash: hash,
                definition_render_state: definition.render_state(),
                variant,
            })))
        })();
        self.world_shadow_sources.insert(key, result.clone());
        result
    }

    pub fn serialized_variant(
        &mut self,
        name: &[u8],
        overrides: &crate::render_program::SerializedParameterBlock,
    ) -> Result<Option<crate::render_program::D3dVariantData>> {
        let Some(hash) = self.specialization_hash(name, overrides)? else {
            return Ok(None);
        };
        let Some(bytes) = self.cache.variant(name, hash)? else {
            return Ok(None);
        };
        let (variant, tail) = crate::render_program::parse_d3d_variant(&bytes)?;
        if !tail.is_empty() {
            return Err("trailing D3D program data".into());
        }
        Ok(Some(variant))
    }
}

fn read_span(file: &mut File, offset: u64, length: usize) -> Result<Vec<u8>> {
    let end = offset
        .checked_add(length as u64)
        .ok_or("shader-cache data offset overflow")?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    if end > size {
        return Err(format!(
            "shader-cache data span {offset}..{end} exceeds file size {size}"
        ));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn inflate(bytes: &[u8], unpacked: u32) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    DeflateDecoder::new(bytes)
        .take(u64::from(unpacked) + 1)
        .read_to_end(&mut result)
        .map_err(|error| format!("shader-cache raw DEFLATE: {error}"))?;
    if result.len() != unpacked as usize {
        return Err(format!(
            "shader-cache unpacked size {} != {unpacked}",
            result.len()
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires installed RPD/registered stage-sort constants"]
    fn installed_water_stage_sort_uses_main_defaults_and_authored_overrides() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(&base).unwrap();
        let mut library = super::ProgramLibrary::open(&base, super::ShaderCacheBackend::Pc, &catalog).unwrap();
        for (program, expected) in [(b"arkWater".as_slice(), 300f32), (b"arkBoatVolume".as_slice(), 80f32)] {
            assert_eq!(library.material_stage_sort(&catalog, program, "{}").unwrap(), [expected.to_bits(), 0]);
            let authored_sort = if program.eq_ignore_ascii_case(b"arkWater") {300f32} else {-1.5f32};
            assert_eq!(library.material_stage_sort(&catalog, program,
                "{stageSort -1.5 stageSortAdjust .25}").unwrap(), [authored_sort.to_bits(), 0.25f32.to_bits()]);
            assert_eq!(library.material_stage_sort(&catalog, program,
                "{stageSort sortWater stageSort sortBoatVolume}").unwrap(), [expected.to_bits(), 0]);
        }
        let thresholds = library.material_sort_thresholds(&catalog).unwrap();
        assert_eq!(thresholds[0], 80.);
        assert_eq!(thresholds[5], 300.);
        assert_eq!(crate::material::draw_bucket(0, 0, 0, false, [80., 0.], thresholds), 0x26);
        assert_eq!(crate::material::draw_bucket(0, 0, 0, false, [300., 0.], thresholds), 0x2d);
        println!("Installed material-list boundaries: {thresholds:?}");
    }
    use super::*;
    use flate2::{write::DeflateEncoder, Compression};
    use std::io::Write;

    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend(value.to_le_bytes());
    }
    fn name(out: &mut Vec<u8>, value: &[u8]) {
        word(out, value.len() as u32);
        out.extend(value);
    }
    fn packed(data: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rigid_opaque_identity_requires_actual_dxbc_checksum() {
        let mut bytes = b"DXBC".to_vec();
        for word in [0xbe596c78u32, 0x7b23f0b2, 0x921af52f, 0x29b1e602] {
            bytes.extend(word.to_le_bytes());
        }
        assert!(is_captured_rigid_opaque_stage(&bytes));
        bytes[19] ^= 1;
        assert!(!is_captured_rigid_opaque_stage(&bytes));
        assert!(!is_captured_rigid_opaque_stage(b"17296"));
        assert!(!is_captured_rigid_opaque_stage(b"DXBC"));
    }

    #[test]
    fn resident_indices_payloads_and_lookup_rules() {
        let dir =
            std::env::temp_dir().join(format!("dh2-owned-shader-cache-{}", std::process::id()));
        let fixture = Fixture(dir);
        std::fs::create_dir_all(fixture.0.join("shaderCache")).unwrap();
        let variant = b"owned variant bytes";
        let stage = b"owned shader bytes";
        let vc = packed(variant);
        let sc = packed(stage);
        let mut rpi = Vec::new();
        word(&mut rpi, VERSION);
        word(&mut rpi, 1);
        name(&mut rpi, b"ArkDefault");
        word(&mut rpi, 1);
        word(&mut rpi, 3);
        let mut rpvi = Vec::new();
        word(&mut rpvi, VERSION);
        name(&mut rpvi, b"not compared by native");
        word(&mut rpvi, 2);
        word(&mut rpvi, 0xfeed);
        word(&mut rpvi, 0);
        word(&mut rpvi, 1); // marker != decoded length
        word(&mut rpvi, 0xdead);
        word(&mut rpvi, u32::MAX);
        word(&mut rpvi, 0);
        let mut uci = Vec::new();
        word(&mut uci, VERSION);
        word(&mut uci, 0);
        word(&mut uci, 1);
        word(&mut uci, 0);
        word(&mut uci, ((sc.len() as u32) << 8) | 0xa5);
        let mut strings = Vec::new();
        word(&mut strings, VERSION);
        strings.extend(2u16.to_le_bytes());
        name(&mut strings, b"first");
        name(&mut strings, b"");
        let mut rpvd = Vec::new();
        word(&mut rpvd, vc.len() as u32);
        word(&mut rpvd, variant.len() as u32);
        rpvd.extend(vc);
        let mut ucd = Vec::new();
        word(&mut ucd, stage.len() as u32);
        ucd.extend(sc);
        ucd.extend([0x91; 64]);
        for (extension, bytes) in [
            ("rpi", rpi),
            ("rpvi", rpvi),
            ("uci", uci),
            ("str", strings),
            ("rpd", b"xdef".to_vec()),
            ("rpvd", rpvd),
            ("ucd", ucd),
        ] {
            std::fs::write(
                fixture
                    .0
                    .join("shaderCache")
                    .join(format!("ShaderCache_PC.{extension}")),
                bytes,
            )
            .unwrap();
        }
        let mut cache = ShaderCache::open(&fixture.0, ShaderCacheBackend::Pc).unwrap();
        assert_eq!(cache.stage_counts, [0, 1]);
        assert_eq!(cache.names, [b"first".to_vec(), Vec::new()]);
        assert_eq!(
            cache.raw_definition(b"arkDEFAULT\0ignored").unwrap(),
            Some(b"def".to_vec())
        );
        assert_eq!(
            cache.variant(b"arkDefault", 0xfeed).unwrap(),
            Some(variant.to_vec())
        );
        assert_eq!(cache.variant(b"arkDefault", 0xdead).unwrap(), None);
        assert_eq!(cache.variant(b"arkDefault", 0).unwrap(), None);
        assert_eq!(cache.variant(b"missing", 0xfeed).unwrap(), None);
        let decoded = cache.stage(0).unwrap();
        assert_eq!(decoded.flags, 0xa5);
        assert_eq!(decoded.bytes, stage);
        assert!(cache.stage(1).is_err());
        assert!(inflate(&packed(b"longer"), 2).is_err());
        assert!(inflate(&packed(b"short"), 20).is_err());
        assert_eq!(name_key(b"A\xC0Z"), b"a\xC0z");
        for length in 0..4 {
            assert!(Cursor::new(&VERSION.to_le_bytes()[..length]).is_err());
        }
        assert!(Cursor::new(&13u32.to_le_bytes()).is_err());
    }

    #[test]
    #[ignore = "requires installed original render-parameter declarations"]
    fn installed_parameter_declaration_source() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let paths = catalog.paths_containing("renderparm");
        eprintln!(
            "render-parameter declaration paths: {} {:?}",
            paths.len(),
            &paths[..paths.len().min(8)]
        );
        for path in paths
            .iter()
            .filter(|p| {
                p.ends_with("/isshadow.decl")
                    || p.ends_with("/diffusemap.decl")
                    || p.ends_with("/shadowprogram.decl")
            })
            .take(6)
        {
            eprintln!(
                "{path}: {}",
                String::from_utf8_lossy(&catalog.read(path).unwrap().unwrap())
            );
        }
        let source = catalog
            .read("generated/decls/renderprog/arkdefault.decl")
            .unwrap()
            .unwrap();
        eprintln!(
            "arkdefault prefix: {}",
            String::from_utf8_lossy(&source[..source.len().min(2500)])
        );
        assert!(!paths.is_empty());
    }

    #[test]
    #[ignore = "requires the installed original shader cache and declarations"]
    fn installed_ordered_specializations_use_registered_defaults() {
        let base = &crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
        let programs: Vec<_> = library
            .cache
            .programs
            .values()
            .map(|p| p.name.clone())
            .collect();
        let empty = crate::render_program::SerializedParameterBlock {
            flags: 0,
            parameters: vec![],
        };
        let mut cached = 0;
        for program in &programs {
            let hash = library
                .specialization_hash(program, &empty)
                .unwrap_or_else(|error| panic!("{}: {error}", String::from_utf8_lossy(program)))
                .unwrap();
            if library
                .cache
                .program(program)
                .unwrap()
                .variants
                .contains_key(&hash)
            {
                cached += 1;
            }
        }
        assert_eq!(library.definitions.len(), programs.len());
        eprintln!("{} actual option defaults; {} ordered program signatures; {cached} resident default variants", library.option_defaults.len(), programs.len());
    }

    #[test]
    #[ignore = "requires the installed original shader cache and declarations"]
    fn installed_shadow_pass_serialized_variants() {
        use crate::render_program::{
            RenderParameterKind, SerializedParameter, SerializedParameterBlock,
            SerializedParameterValue, StageBytecode,
        };
        let base = &crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
        // 1402d5210 slot10, ordinary nonblended/non-sky fixture. These are
        // explicit fixture inputs, NOT a runtime fallback for other classes.
        let overrides = SerializedParameterBlock {
            flags: 0,
            parameters: [("isShadow", 1), ("isAlpha", 0), ("noScreenShadows", 0)]
                .into_iter()
                .map(|(name, value)| SerializedParameter {
                    name: name.as_bytes().to_vec(),
                    kind: RenderParameterKind::Option,
                    value: SerializedParameterValue::Words([value, 0, 0, 0]),
                })
                .collect(),
        };
        let programs: Vec<_> = library
            .cache
            .programs
            .values()
            .map(|p| p.name.clone())
            .collect();
        let mut resolved = 0;
        for program in programs {
            library.definition(&program).unwrap().unwrap();
            let definition = &library.definitions[&name_key(&program)];
            if !definition
                .specialization_names(&library.cache.names)
                .unwrap()
                .iter()
                .any(|name| name.eq_ignore_ascii_case(b"isShadow"))
            {
                continue;
            }
            let Some(variant) = library.serialized_variant(&program, &overrides).unwrap() else {
                continue;
            };
            resolved += 1;
            for stage in variant.stages {
                let bytes = match stage.bytecode {
                    StageBytecode::Cached(id) => library.cache.stage(id as usize).unwrap().bytes,
                    StageBytecode::Inline(bytes) => bytes,
                    StageBytecode::Absent => continue,
                };
                assert_eq!(bytes.get(..4), Some(b"DXBC".as_slice()));
                let checksum: [u32; 4] = std::array::from_fn(|i| {
                    u32::from_le_bytes(bytes[4 + i * 4..8 + i * 4].try_into().unwrap())
                });
                eprintln!(
                    "shadow variant {} stage={} checksum={checksum:08x?}",
                    String::from_utf8_lossy(&program),
                    stage.kind
                );
            }
        }
        assert!(resolved > 0);
        eprintln!("{resolved} exact resident serialized shadow variants (fixture inputs)");
    }

    #[test]
    #[ignore = "requires installed original cache; writes local-only exact world-instanced DXBC evidence"]
    fn installed_world_shadow_stage_evidence() {
        use crate::render_program::{
            RenderParameterKind, SerializedParameter, SerializedParameterBlock,
            SerializedParameterValue, StageBytecode,
        };
        let base = &crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
        let directory = Path::new("artifacts/native-world-shadow-source");
        std::fs::create_dir_all(directory).unwrap();
        let mut found = 0;
        for (name, alpha) in [
            ("arkdefault", 0),
            ("arkdefault", 1),
            ("arkvegetation", 0),
            ("arkvegetation", 1),
            ("arkvista", 0),
            ("arkvista", 1),
        ] {
            let overrides = SerializedParameterBlock {
                flags: 0,
                parameters: [
                    ("isShadow", 1),
                    ("isAlpha", 0),
                    ("noScreenShadows", 0),
                    ("useMultiInstancing", 1),
                    ("alphaTest", alpha),
                    ("forcePixelShader", alpha),
                ]
                .into_iter()
                .map(|(name, value)| SerializedParameter {
                    name: name.as_bytes().to_vec(),
                    kind: RenderParameterKind::Option,
                    value: SerializedParameterValue::Words([value, 0, 0, 0]),
                })
                .collect(),
            };
            let Some(hash) = library
                .specialization_hash(name.as_bytes(), &overrides)
                .unwrap()
            else {
                eprintln!("world shadow {name}: no installed definition");
                continue;
            };
            let Some(variant) = library
                .serialized_variant(name.as_bytes(), &overrides)
                .unwrap()
            else {
                eprintln!(
                    "world shadow {name} alpha={alpha} signature={hash:08x}: no resident variant"
                );
                continue;
            };
            if name == "arkdefault" {
                let source = library
                    .world_shadow_source(name.as_bytes(), &overrides)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    source.class,
                    if alpha == 0 {
                        CachedWorldShadowClass::Opaque
                    } else {
                        CachedWorldShadowClass::AlphaTested
                    }
                );
                assert!(std::sync::Arc::ptr_eq(
                    &source,
                    &library
                        .world_shadow_source(name.as_bytes(), &overrides)
                        .unwrap()
                        .unwrap()
                ));
                // An instanced source must never be accepted as a rigid model.
                assert!(library
                    .rigid_shadow_source(name.as_bytes(), &overrides)
                    .is_err());
            }
            for stage in variant.stages {
                let bytes = match stage.bytecode {
                    StageBytecode::Absent => continue,
                    StageBytecode::Inline(bytes) => bytes,
                    StageBytecode::Cached(id) => library.cache.stage(id as usize).unwrap().bytes,
                };
                let path = directory.join(format!("{name}-{hash:08x}-{}.dxbc", stage.kind));
                std::fs::write(&path, &bytes).unwrap();
                eprintln!(
                    "world shadow {} stage={} checksum={:02x?}",
                    path.display(),
                    stage.kind,
                    &bytes[4..20]
                );
                found += 1;
            }
        }
        assert!(
            found > 0,
            "no resident world-shadow variants for explicit input options"
        );
    }

    #[test]
    #[ignore = "requires the installed original shader cache and declarations; writes local-only DXBC evidence"]
    fn installed_alpha_shadow_stage_evidence() {
        use crate::render_program::{
            RenderParameterKind, SerializedParameter, SerializedParameterBlock,
            SerializedParameterValue, StageBytecode,
        };
        let base = &crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
        let overrides = SerializedParameterBlock {
            flags: 0,
            parameters: [
                ("isShadow", 1),
                ("isAlpha", 0),
                ("noScreenShadows", 0),
                ("alphaTest", 1),
                ("forcePixelShader", 1),
            ]
            .into_iter()
            .map(|(name, value)| SerializedParameter {
                name: name.as_bytes().to_vec(),
                kind: RenderParameterKind::Option,
                value: SerializedParameterValue::Words([value, 0, 0, 0]),
            })
            .collect(),
        };
        let directory = Path::new("artifacts/native-alpha-shadow-source");
        std::fs::create_dir_all(directory).unwrap();
        let mut found = 0;
        for name in [b"arkdefault".as_slice(), b"arkvista"] {
            let hash = library
                .specialization_hash(name, &overrides)
                .unwrap()
                .unwrap();
            let Some(variant) = library.serialized_variant(name, &overrides).unwrap() else {
                continue;
            };
            if name == b"arkdefault" {
                let source = library
                    .rigid_shadow_source(name, &overrides)
                    .unwrap()
                    .unwrap();
                assert_eq!(source.class, CachedRigidShadowClass::AlphaTested);
                assert_eq!(source.variant_hash, hash);
            } else {
                // Arkvista's different VS/anisotropic PS are not this class.
                assert!(library.rigid_shadow_source(name, &overrides).is_err());
            }
            for stage in variant.stages {
                let bytes = match stage.bytecode {
                    StageBytecode::Absent => continue,
                    StageBytecode::Inline(bytes) => bytes,
                    StageBytecode::Cached(id) => library.cache.stage(id as usize).unwrap().bytes,
                };
                let file = directory.join(format!(
                    "{}-{hash:08x}-{}.dxbc",
                    String::from_utf8_lossy(name),
                    stage.kind
                ));
                std::fs::write(&file, &bytes).unwrap();
                eprintln!(
                    "alpha shadow source {} stage={} checksum={:02x?}",
                    file.display(),
                    stage.kind,
                    &bytes[4..20]
                );
                found += 1;
            }
        }
        assert!(
            found > 0,
            "no actual resident alpha shadow variants for these explicit fixture inputs"
        );
    }

    #[test]
    #[ignore = "requires installed original shader cache; writes local-only skin DXBC evidence"]
    fn installed_skin_shadow_stage_evidence() {
        use crate::render_program::{
            RenderParameterKind, SerializedParameter, SerializedParameterBlock,
            SerializedParameterValue, StageBytecode,
        };
        let base = &crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
        let directory = Path::new("artifacts/native-skin-shadow-source");
        std::fs::create_dir_all(directory).unwrap();
        let mut found = 0;
        for influences in [4, 8] {
            for alpha in [0, 1] {
                for screen in [0, 1] {
                    let overrides = SerializedParameterBlock {
                        flags: 0,
                        parameters: [
                            ("isShadow", 1),
                            ("isAlpha", 0),
                            ("noScreenShadows", screen),
                            ("alphaTest", alpha),
                            ("forcePixelShader", alpha),
                            ("useSkinning", u32::from(influences == 4)),
                            ("useSkinning8", u32::from(influences == 8)),
                        ]
                        .into_iter()
                        .map(|(name, value)| SerializedParameter {
                            name: name.as_bytes().to_vec(),
                            kind: RenderParameterKind::Option,
                            value: SerializedParameterValue::Words([value, 0, 0, 0]),
                        })
                        .collect(),
                    };
                    let Some(variant) = library
                        .serialized_variant(b"arkdefault", &overrides)
                        .unwrap()
                    else {
                        eprintln!("SKIN resident miss: influences={influences} alpha={alpha} noScreen={screen}");
                        continue;
                    };
                    for stage in variant.stages {
                        let bytes = match stage.bytecode {
                            StageBytecode::Absent => continue,
                            StageBytecode::Inline(bytes) => bytes,
                            StageBytecode::Cached(id) => {
                                library.cache.stage(id as usize).unwrap().bytes
                            }
                        };
                        let file = directory.join(format!(
                            "skin{influences}-alpha{alpha}-screen{screen}-{}.dxbc",
                            stage.kind
                        ));
                        std::fs::write(&file, &bytes).unwrap();
                        eprintln!("SKIN {} {:02x?}", file.display(), &bytes[4..20]);
                        found += 1;
                    }
                }
            }
        }
        assert!(found > 0);
    }

    #[test]
    #[ignore = "requires the installed original shader cache and declarations"]
    fn installed_opaque_shadow_source_and_option_defaults() {
        use crate::render_program::{
            RenderParameterKind, SerializedParameter, SerializedParameterBlock,
            SerializedParameterValue,
        };
        let base = &crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
        let mut overrides = SerializedParameterBlock {
            flags: 0,
            parameters: [("isShadow", 1), ("isAlpha", 0), ("noScreenShadows", 0)]
                .into_iter()
                .map(|(name, value)| SerializedParameter {
                    name: name.as_bytes().to_vec(),
                    kind: RenderParameterKind::Option,
                    value: SerializedParameterValue::Words([value, 0, 0, 0]),
                })
                .collect(),
        };
        assert_eq!(
            library
                .option_value(b"arkdefault", &overrides, b"alphaTest")
                .unwrap(),
            0
        );
        let source = library
            .rigid_shadow_source(b"arkdefault", &overrides)
            .unwrap()
            .unwrap();
        let same = library
            .rigid_shadow_source(b"ARKDEFAULT", &overrides)
            .unwrap()
            .unwrap();
        assert_eq!(source.class, CachedRigidShadowClass::Opaque);
        assert!(std::sync::Arc::ptr_eq(&source, &same));
        assert_eq!(library.rigid_shadow_sources.len(), 1);
        assert_eq!(
            source.variant.header.source_crc,
            library.definitions[b"arkdefault".as_slice()].source_crc
        );
        assert_eq!(
            source.definition_render_state,
            library.definitions[b"arkdefault".as_slice()].render_state()
        );
        overrides.parameters.push(SerializedParameter {
            name: b"alphaTest".to_vec(),
            kind: RenderParameterKind::Option,
            value: SerializedParameterValue::Words([7, 0, 0, 0]),
        });
        assert_eq!(
            library
                .option_value(b"arkdefault", &overrides, b"ALPHATEST")
                .unwrap(),
            7
        );
        overrides.parameters.push(SerializedParameter {
            name: b"alphaTest".to_vec(),
            kind: RenderParameterKind::Option,
            value: SerializedParameterValue::Words([0, 0, 0, 0]),
        });
        assert_eq!(
            library
                .option_value(b"arkdefault", &overrides, b"alphaTest")
                .unwrap(),
            0
        );
        eprintln!(
            "actual arkdefault rigid source {:08x}, repeated lookup shares decoded data",
            source.variant_hash
        );

        // A deliberate DIFFERENT RPD fallback demonstrates the native null-
        // fallback path used for decalType, rather than coincident zeroes.
        let material_only = library
            .registered_option_value(&overrides, b"decalType")
            .unwrap();
        library
            .definitions
            .get_mut(b"arkdefault".as_slice())
            .unwrap()
            .specialization_defaults
            .parameters
            .push(SerializedParameter {
                name: b"decalType".to_vec(),
                kind: RenderParameterKind::Option,
                value: SerializedParameterValue::Words([(material_only ^ 1) as u32, 0, 0, 0]),
            });
        assert_eq!(
            library
                .option_value(b"arkdefault", &overrides, b"decalType")
                .unwrap(),
            material_only ^ 1
        );
        assert_eq!(
            library
                .registered_option_value(&overrides, b"DECALTYPE")
                .unwrap(),
            material_only
        );
        overrides.parameters.push(SerializedParameter {
            name: b"decalType".to_vec(),
            kind: RenderParameterKind::Option,
            value: SerializedParameterValue::Words([(material_only ^ 2) as u32, 0, 0, 0]),
        });
        assert_eq!(
            library
                .registered_option_value(&overrides, b"decalType")
                .unwrap(),
            material_only ^ 2
        );
        assert_eq!(
            library
                .option_value(b"arkdefault", &overrides, b"decalType")
                .unwrap(),
            material_only ^ 2
        );
    }

    #[test]
    #[ignore = "requires the installed original shader cache and declarations"]
    fn installed_program_definitions_and_parameter_types() {
        let base = &crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let schema = crate::render_program::render_parameter_schema(&catalog).unwrap();
        let mut cache = ShaderCache::open(base, ShaderCacheBackend::Pc).unwrap();
        let programs: Vec<_> = cache.programs.values().map(|p| p.name.clone()).collect();
        let mut parameters = 0;
        let mut options = 0;
        for name in &programs {
            let bytes = cache.raw_definition(name).unwrap().unwrap();
            let (definition, tail) =
                crate::render_program::parse_program_definition(&bytes, |name| {
                    schema.get(&name_key(name)).copied().ok_or_else(|| {
                        format!("unresolved type: {}", String::from_utf8_lossy(name))
                    })
                })
                .unwrap_or_else(|error| panic!("{}: {error}", String::from_utf8_lossy(name)));
            assert!(
                tail.is_empty(),
                "{}: {} unparsed bytes",
                String::from_utf8_lossy(name),
                tail.len()
            );
            let names = definition.specialization_names(&cache.names).unwrap();
            for include in &definition.includes {
                include.name.bytes(&cache.names).unwrap();
            }
            for group in [&definition.bindings, &definition.stage_bindings] {
                for parameter in &group.parameters {
                    parameter.name.bytes(&cache.names).unwrap();
                }
            }
            parameters += definition.defaults.parameters.len()
                + definition.specialization_defaults.parameters.len();
            options += names.len();
            if name.eq_ignore_ascii_case(b"arkdefault") {
                eprintln!(
                    "arkdefault specialization order: {:?}",
                    names
                        .iter()
                        .map(|n| String::from_utf8_lossy(n))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "arkdefault specialization defaults: {:?}",
                    definition.specialization_defaults
                );
            }
        }
        eprintln!("installed definitions: {} declarations, {} complete definitions, {parameters} default parameters, {options} ordered specialization parameters", schema.len(), programs.len());
        assert!(!programs.is_empty());
    }

    #[test]
    #[ignore = "requires the installed original shader cache"]
    fn installed_cache_all_indexed_stages_and_variants() {
        let mut cache = ShaderCache::open(
            &crate::assets::installation::game_base().unwrap(),
            ShaderCacheBackend::Pc,
        )
        .unwrap();
        let programs = cache.programs.len();
        let names = cache.names.len();
        let stage_counts = cache.stage_counts;
        let mut byte_count = 0u64;
        for id in 0..cache.stages.len() {
            let stage = cache
                .stage(id)
                .unwrap_or_else(|error| panic!("stage {id}: {error}"));
            assert_eq!(&stage.bytes[..4], b"DXBC", "stage {id}");
            assert_eq!(
                u32::from_le_bytes(stage.bytes[24..28].try_into().unwrap()) as usize,
                stage.bytes.len()
            );
            byte_count += stage.bytes.len() as u64;
        }
        let entries: Vec<_> = cache
            .programs
            .values()
            .map(|p| {
                (
                    p.name.clone(),
                    p.variants.keys().copied().collect::<Vec<_>>(),
                )
            })
            .collect();
        let mut variants = 0;
        let mut stage_references = 0;
        let mut parameter_checksums = HashMap::new();
        let mut include_checksums = HashMap::new();
        for (name, hashes) in entries {
            let definition = cache.raw_definition(&name).unwrap().unwrap();
            assert_eq!(&definition[..4], &0x8000_003fu32.to_be_bytes());
            let mut source_crc = None;
            for hash in hashes {
                if let Some(bytes) = cache.variant(&name, hash).unwrap() {
                    let (data, tail) = crate::render_program::parse_d3d_variant(&bytes)
                        .unwrap_or_else(|error| {
                            panic!("{} {hash:08x}: {error}", String::from_utf8_lossy(&name))
                        });
                    assert!(
                        tail.is_empty(),
                        "unparsed variant bytes: {} {hash:08x}",
                        String::from_utf8_lossy(&name)
                    );
                    assert_eq!(
                        *source_crc.get_or_insert(data.header.source_crc),
                        data.header.source_crc
                    );
                    for include in &data.includes {
                        let name = include.name.bytes(&cache.names).unwrap().to_vec();
                        if let Some(old) = include_checksums.insert(name, include.checksum) {
                            assert_eq!(old, include.checksum);
                        }
                    }
                    for parameter in &data.parameters {
                        parameter.bytes(&cache.names).unwrap();
                    }
                    for group in std::iter::once(&data.bindings)
                        .chain(data.stages.iter().map(|stage| &stage.bindings))
                    {
                        for parameter in &group.parameters {
                            let name = parameter.name.bytes(&cache.names).unwrap().to_vec();
                            if let Some(old) = parameter_checksums.insert(name, parameter.checksum)
                            {
                                assert_eq!(old, parameter.checksum);
                            }
                        }
                    }
                    for stage in &data.stages {
                        if let crate::render_program::StageBytecode::Cached(id) = stage.bytecode {
                            assert!((id as usize) < cache.stages.len());
                            stage_references += 1;
                        }
                    }
                    variants += 1;
                }
            }
        }
        eprintln!("installed native shader cache: {programs} programs, {names} shared names, stage counts {stage_counts:?}, {byte_count} decoded stage bytes, {variants} complete D3D variants, {stage_references} cached-stage references, {} include CRCs, {} parameter checksums", include_checksums.len(), parameter_checksums.len());
    }
}
