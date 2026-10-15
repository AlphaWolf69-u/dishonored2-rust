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
pub struct NativeShaderCache {
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

impl NativeShaderCache {
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
    pub program: Vec<u8>,
    pub variant_hash: u32,
    pub definition_render_state: [u32; 9],
    pub variant: crate::render_program::D3dVariantData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedRigidShadowClass {
    Opaque,
    AlphaTested,
    Vegetation,
}

fn is_captured_rigid_opaque_stage(bytes: &[u8]) -> bool {
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
pub struct NativeProgramLibrary {
    pub cache: NativeShaderCache,
    pub parameter_kinds: HashMap<Vec<u8>, crate::render_program::RenderParameterKind>,
    option_defaults: HashMap<Vec<u8>, i32>,
    float_vector_defaults: HashMap<Vec<u8>, Result<[u32; 4]>>,
    definitions: HashMap<Vec<u8>, crate::render_program::RenderProgramDefinition>,
    rigid_shadow_sources:
        HashMap<(Vec<u8>, u32), Result<Option<std::sync::Arc<CachedRigidShadowSource>>>>,
}

impl NativeProgramLibrary {
    pub fn open(
        base: &Path,
        backend: ShaderCacheBackend,
        catalog: &crate::archive::Catalog,
    ) -> Result<Self> {
        let cache = NativeShaderCache::open(base, backend)?;
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
            let class = if opaque_vertex && pixel.is_none() {
                CachedRigidShadowClass::Opaque
            } else if vertex == Some(alpha_vertex) && pixel == Some(alpha_pixel) {
                CachedRigidShadowClass::AlphaTested
            } else if vertex == Some(vegetation_vertex) && pixel == Some(vegetation_pixel) {
                CachedRigidShadowClass::Vegetation
            } else {
                return Err(format!(
                    "unsupported shadow bytecode pair for {}",
                    String::from_utf8_lossy(name)
                ));
            };
            let definition = &self.definitions[&name_key(name)];
            if variant.header.source_crc != definition.source_crc {
                return Err(
                    "compiled shadow source CRC differs from its resident definition".into(),
                );
            }
            let state = definition.native_render_state();
            Ok(Some(std::sync::Arc::new(CachedRigidShadowSource {
                class,
                program: name.to_vec(),
                variant_hash: hash,
                definition_render_state: state,
                variant,
            })))
        })();
        self.rigid_shadow_sources.insert(key, result.clone());
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
    fn resident_indices_payloads_and_native_lookup_rules() {
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
        let mut cache = NativeShaderCache::open(&fixture.0, ShaderCacheBackend::Pc).unwrap();
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
        let catalog = crate::archive::Catalog::open(Path::new(
            "D:/Programs/Steam/steamapps/common/Dishonored2/base",
        ))
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
        let base = Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base");
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            NativeProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
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
        let base = Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base");
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            NativeProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
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
    #[ignore = "requires the installed original shader cache and declarations; writes local-only DXBC evidence"]
    fn installed_alpha_shadow_stage_evidence() {
        use crate::render_program::{
            RenderParameterKind, SerializedParameter, SerializedParameterBlock,
            SerializedParameterValue, StageBytecode,
        };
        let base = Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base");
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            NativeProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
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
    #[ignore = "requires the installed original shader cache and declarations"]
    fn installed_opaque_shadow_source_and_option_defaults() {
        use crate::render_program::{
            RenderParameterKind, SerializedParameter, SerializedParameterBlock,
            SerializedParameterValue,
        };
        let base = Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base");
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let mut library =
            NativeProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog).unwrap();
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
            library.definitions[b"arkdefault".as_slice()].native_render_state()
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
        let base = Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base");
        let catalog = crate::archive::Catalog::open(base).unwrap();
        let schema = crate::render_program::render_parameter_schema(&catalog).unwrap();
        let mut cache = NativeShaderCache::open(base, ShaderCacheBackend::Pc).unwrap();
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
        let mut cache = NativeShaderCache::open(
            Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base"),
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
