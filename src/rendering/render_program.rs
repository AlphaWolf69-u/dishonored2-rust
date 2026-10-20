//! Original render-program variant naming and compiled D3D data.
//! Installed resident-cache access is in `shader_cache.rs`; archived `.vbin`
//! lookup is a distinct optional source, not the installed shader bank.
//!
//! FUN_1402dbb10 preserves its RDX parameter dictionary when calling
//! 1402be900. That function walks the program's ordered parameter handles
//! (+0x2b6..+0x2b8, table +0x2c0), resolves values via 140342830 and hashes
//! `+name[value]` with 1401431a0. Callers must supply that actual order/names
//! and resolved integer values; a sorted material dictionary is NOT equivalent.
//! This is not the compiler/lifecycle in 1402dbb90/1402db460. Those also try
//! an in-memory program, use the installed binary, then validate/bind compiled
//! stages. Parsing that program and publishing the shadow banks remain separate.
//! Verified loader boundary: 1402db460 tries view vtable +0x10 (resident), then
//! +0x08 (generated .vbin name from 1402bf9c0), resets via +0x20 on failure,
//! and calls 1402db390 only after success. 1402dbb90 returns/caches the owned
//! program even when loading fails in shipping mode; only editor/development
//! flags throw "Unknow error while compiling render program for PC".
//! Consequently a missing resident permutation is NOT evidence for noShadow
//! or permission to pick another permutation. Still blocked: view +8/+0x10
//! loader/bind lifecycle and native program-bank publication/selection.

/// FUN_1402c0ab0 (arkProgViewDataD3d virtual +0x38).
pub const D3D_VARIANT_MAGIC: u32 = 0x8000_004a;

/// The prefix consumed by FUN_1402bfc10, before include identity validation.
/// All words are big-endian; flag bits are retained, not reinterpreted as a
/// shadow program class. A valid prefix alone does NOT make a usable program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct D3dVariantHeader {
    pub source_crc: u32,
    pub flags: u32,
    pub shared_names: bool,
    pub include_count: u32,
}

pub fn parse_d3d_variant_header(bytes: &[u8]) -> Result<(D3dVariantHeader, &[u8]), String> {
    let prefix = bytes.get(..17).ok_or("truncated D3D program header")?;
    let word = |at| u32::from_be_bytes(prefix[at..at + 4].try_into().unwrap());
    if word(0) != D3D_VARIANT_MAGIC {
        return Err(format!("unsupported D3D program magic {:08x}", word(0)));
    }
    Ok((
        D3dVariantHeader {
            source_crc: word(4),
            flags: word(8),
            shared_names: prefix[12] != 0,
            include_count: word(13),
        },
        &bytes[17..],
    ))
}

/// Serialized name, before interning/identity/checksum validation. Shared IDs
/// are big-endian u16; inline string lengths are LITTLE-endian u32.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantName {
    Shared(u16),
    Inline(Vec<u8>),
}
impl VariantName {
    pub fn bytes<'a>(&'a self, names: &'a [Vec<u8>]) -> Result<&'a [u8], String> {
        match self {
            Self::Shared(id) => names
                .get(usize::from(*id))
                .map(Vec::as_slice)
                .ok_or_else(|| format!("D3D shared name ID {id} out of range")),
            Self::Inline(bytes) => Ok(bytes),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantIdentity {
    pub name: VariantName,
    pub checksum: u32,
}

/// 1402be020: its sharing flag is present only when count > 0, and is
/// independent of the variant header's include/parameter sharing flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantBindings {
    pub shared_names: Option<bool>,
    pub parameters: Vec<VariantIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageBytecode {
    Absent,
    Cached(u32),
    Inline(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct D3dStage {
    pub kind: u32,
    pub bindings: VariantBindings,
    /// 1402bddc0: nine big-endian u16s. No inferred vertex format/class.
    pub layout: [u16; 9],
    pub bytecode: StageBytecode,
}

/// Complete serialized data consumed by 1402bfc10, NOT a validated/bound
/// executable program. That function also checks current source CRC, ordered
/// include identities/CRCs, and every current render-parameter checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct D3dVariantData {
    pub header: D3dVariantHeader,
    pub includes: Vec<VariantIdentity>,
    pub parameters: Vec<VariantName>,
    pub stages: Vec<D3dStage>,
    pub bindings: VariantBindings,
    pub layout: [u16; 9],
}

struct VariantReader<'a> {
    bytes: &'a [u8],
}
impl<'a> VariantReader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let bytes = self
            .bytes
            .get(..count)
            .ok_or("truncated render-program data")?;
        self.bytes = &self.bytes[count..];
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn name(&mut self, shared: bool) -> Result<VariantName, String> {
        if shared {
            return Ok(VariantName::Shared(self.u16()?));
        }
        Ok(VariantName::Inline(self.string()?))
    }
    // 140003580 and the inline-name loops clear strings for signed lengths <= 0.
    fn string(&mut self) -> Result<Vec<u8>, String> {
        let length = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
        Ok(if length <= 0 {
            Vec::new()
        } else {
            self.take(length as usize)?.to_vec()
        })
    }
    fn bindings(&mut self) -> Result<VariantBindings, String> {
        let count = self.u32()?;
        let shared_names = if count == 0 {
            None
        } else {
            Some(self.byte()? != 0)
        };
        let mut parameters = Vec::new();
        for _ in 0..count {
            let checksum = self.u32()?;
            let name = self.name(shared_names.unwrap())?;
            parameters.push(VariantIdentity { name, checksum });
        }
        Ok(VariantBindings {
            shared_names,
            parameters,
        })
    }
    fn layout(&mut self) -> Result<[u16; 9], String> {
        let mut layout = [0; 9];
        for value in &mut layout {
            *value = self.u16()?;
        }
        Ok(layout)
    }
}

pub fn parse_d3d_variant(bytes: &[u8]) -> Result<(D3dVariantData, &[u8]), String> {
    let (header, bytes) = parse_d3d_variant_header(bytes)?;
    let mut reader = VariantReader { bytes };
    let mut includes = Vec::new();
    for _ in 0..header.include_count {
        let name = reader.name(header.shared_names)?;
        includes.push(VariantIdentity {
            name,
            checksum: reader.u32()?,
        });
    }
    let mut parameters = Vec::new();
    for _ in 0..reader.u32()? {
        parameters.push(reader.name(header.shared_names)?);
    }
    let mut stages = Vec::new();
    for _ in 0..reader.u32()? {
        let kind = reader.u32()?;
        // Native local arrays have exactly six stage slots.
        if kind >= 6 {
            return Err(format!("D3D shader stage kind {kind} out of range"));
        }
        let bindings = reader.bindings()?;
        let layout = reader.layout()?;
        let word = reader.u32()?;
        let bytecode = if word == 0x8000_0000 {
            StageBytecode::Absent
        } else if word & 0x8000_0000 != 0 {
            StageBytecode::Cached(!word)
        } else {
            StageBytecode::Inline(reader.take(word as usize)?.to_vec())
        };
        stages.push(D3dStage {
            kind,
            bindings,
            layout,
            bytecode,
        });
    }
    let bindings = reader.bindings()?;
    let layout = reader.layout()?;
    Ok((
        D3dVariantData {
            header,
            includes,
            parameters,
            stages,
            bindings,
            layout,
        },
        reader.bytes,
    ))
}

/// Low four bits of idDeclRenderParm +0x4c. Literal type dispatch in
/// 1402d8820; 1401435f0 compares ASCII case-insensitively. This identifies the
/// serialized value shape, NOT the full declaration/default/annotation parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RenderParameterKind {
    Uniform = 0,
    Texture = 1,
    MultisampleTexture = 2,
    ReadOnlyBuffer = 3,
    ReadWriteBuffer = 4,
    ConstantBuffer = 5,
    Struct = 6,
    StructuredBuffer = 7,
    StructuredConstantBuffer = 8,
    Sampler = 9,
    Program = 10,
    String = 11,
    Option = 12,
    Semantic = 13,
    Material = 14,
    ParameterBlock = 15,
}

pub fn render_parameter_kind(source: &str) -> Result<RenderParameterKind, String> {
    use RenderParameterKind::*;
    let token = crate::entities::declaration_type_name(source)?;
    Ok(match token.to_ascii_lowercase().as_str() {
        "vec" | "uniform" => Uniform,
        "tex" | "tex2d" | "texture2d" | "texture2darray" | "tex3d" | "texture3d" | "texcube"
        | "texturecube" | "texturecubearray" | "texshadow2d" | "texshadow3d" | "texshadowcube" => {
            Texture
        }
        "texture2dms" | "texmultisample2d" => MultisampleTexture,
        "robuffer" => ReadOnlyBuffer,
        "rwbuffer" => ReadWriteBuffer,
        "constantbuffer" => ConstantBuffer,
        "struct" => Struct,
        "structuredbuffer" => StructuredBuffer,
        "structuredbufferascbuffer" => StructuredConstantBuffer,
        "sampler" | "samplercmp" => Sampler,
        "program" => Program,
        "string" => String,
        "option" => Option,
        "semantic" | "semanticarray" => Semantic,
        "material" => Material,
        "pblock" => ParameterBlock,
        _ => {
            return Err(format!(
                "unresolved render-parameter declaration type {token:?}"
            ))
        }
    })
}

/// Actual declaration types, loaded once from the installed assets. Names are
/// declaration paths, never guessed from a parameter's spelling/checksum.
pub fn render_parameter_schema(
    catalog: &crate::archive::Catalog,
) -> Result<std::collections::HashMap<Vec<u8>, RenderParameterKind>, String> {
    let mut schema = std::collections::HashMap::new();
    for path in catalog.paths_containing("generated/decls/renderparm/") {
        let Some(name) = path
            .strip_prefix("generated/decls/renderparm/")
            .and_then(|name| name.strip_suffix(".decl"))
        else {
            continue;
        };
        let bytes = catalog
            .read(&path)
            .map_err(|error| format!("{path}: {error}"))?
            .ok_or_else(|| format!("missing render-parameter declaration {path}"))?;
        // Native type names are ASCII. Legacy byte comments/body text must not
        // reject an otherwise valid header; no body/value is decoded here.
        let source = String::from_utf8_lossy(&bytes);
        let kind = render_parameter_kind(&source).map_err(|error| format!("{path}: {error}"))?;
        schema.insert(name.as_bytes().to_vec(), kind);
    }
    Ok(schema)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SerializedParameterValue {
    /// 140678110 swaps each of four words; preserve integer/float/NaN bits.
    Words([u32; 4]),
    Name(Vec<u8>),
    /// Native sampler case really falls through to the program read at
    /// 140343377 (JMP from 140343321); retain BOTH serialized references.
    SamplerThenProgram {
        sampler: Vec<u8>,
        program: Vec<u8>,
    },
    /// This kind has no value payload in 140343090.
    NoPayload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializedParameter {
    pub name: Vec<u8>,
    pub kind: RenderParameterKind,
    pub value: SerializedParameterValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializedParameterBlock {
    /// Original first byte, retained even though the loader does not use it.
    pub flags: u8,
    pub parameters: Vec<SerializedParameter>,
}

impl SerializedParameterBlock {
    /// FUN_140342c30: seed from the registered declaration, then fallback,
    /// then this block. Entries are visited in storage order, so the last
    /// matching interned name wins. Preserve all four words, including NaNs.
    /// Numeric callers cannot reinterpret a serialized resource reference.
    fn numeric_words(
        &self,
        name: &[u8],
        declared_default: [u32; 4],
        fallback: Option<&Self>,
    ) -> Result<[u32; 4], String> {
        let mut value = declared_default;
        for block in fallback.into_iter().chain(std::iter::once(self)) {
            for parameter in &block.parameters {
                if parameter
                    .name
                    .split(|&b| b == 0)
                    .next()
                    .unwrap()
                    .eq_ignore_ascii_case(name.split(|&b| b == 0).next().unwrap())
                {
                    let SerializedParameterValue::Words(words) = &parameter.value else {
                        return Err("numeric render parameter has a nonnumeric payload".into());
                    };
                    value = *words;
                }
            }
        }
        Ok(value)
    }

    /// FUN_140342e70: actual +0x4c declaration flags, not an inferred type.
    /// GetVector copies four raw words; it does not splat, clamp or convert.
    pub fn vector_words(
        &self,
        name: &[u8],
        declaration_flags: u32,
        declared_default: [u32; 4],
        fallback: Option<&Self>,
    ) -> Result<[u32; 4], String> {
        if declaration_flags & 15 != 0 && declaration_flags & 0x30 != 0x30 {
            return Err("not a vector render parameter".into());
        }
        self.numeric_words(name, declared_default, fallback)
    }

    /// FUN_140342830: EAX/MOV or CVTTSS2SI, not Ghidra's float return.
    /// `declaration_flags` is the actual native +0x4c.
    pub fn integer_value(
        &self,
        name: &[u8],
        declaration_flags: u32,
        declared_default: u32,
        fallback: Option<&Self>,
    ) -> Result<i32, String> {
        let kind = declaration_flags & 15;
        if kind != 0 && kind != 12 && declaration_flags & 0x30 != 0x10 {
            return Err("not an integer render parameter".into());
        }
        let word = self.numeric_words(name, [declared_default, 0, 0, 0], fallback)?[0];
        Ok(integer_word(word, declaration_flags) as i32)
    }

    /// FUN_140342ce0: the unsigned getter validates subtype 0x20 but still
    /// uses SIGNED CVTTSS2SI EAX (140342ddc), then returns those 32 bits.
    /// In particular, float 3e9 is not converted with Rust's `as u32`.
    pub fn unsigned_integer_value(
        &self,
        name: &[u8],
        declaration_flags: u32,
        declared_default: u32,
        fallback: Option<&Self>,
    ) -> Result<u32, String> {
        let kind = declaration_flags & 15;
        if kind != 0 && kind != 12 && declaration_flags & 0x30 != 0x20 {
            return Err("not an unsigned integer render parameter".into());
        }
        let word = self.numeric_words(name, [declared_default, 0, 0, 0], fallback)?[0];
        Ok(integer_word(word, declaration_flags))
    }
}

// Reuse the exact token/parameter conversion without making the standalone
// entities/archive tools depend on the renderer's wire-format module.
pub(crate) use crate::entities::integer_word;

/// Full wire data in 1402beb30. Parsing is NOT registry/checksum validation or
/// GPU bank publication. Render/annotation state and category bytes stay raw
/// until their own consumers interpret them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderProgramDefinition {
    pub source_crc: u32,
    pub shared_names: bool,
    pub includes: Vec<VariantIdentity>,
    pub render_state: [u8; 36],
    pub defaults: SerializedParameterBlock,
    pub specialization_defaults: SerializedParameterBlock,
    pub feature_flags: u8,
    pub bindings: VariantBindings,
    pub layout: [u16; 9],
    pub stage_bindings: VariantBindings,
    pub stage_layout: [u8; 45],
    /// Native record order; wire stores fields [2], [0], [1].
    pub records: Vec<[u32; 3]>,
    pub source_name: Vec<u8>,
}

impl RenderProgramDefinition {
    /// 1402be900 uses [layout[7], layout[8]), NOT a count or a sorted map.
    pub fn specialization_names<'a>(
        &'a self,
        names: &'a [Vec<u8>],
    ) -> Result<Vec<&'a [u8]>, String> {
        let range = usize::from(self.layout[7])..usize::from(self.layout[8]);
        self.bindings
            .parameters
            .get(range)
            .ok_or("invalid specialization binding range")?
            .iter()
            .map(|binding| binding.name.bytes(names))
            .collect()
    }
}

impl VariantReader<'_> {
    fn parameter_block(
        &mut self,
        kind: &mut impl FnMut(&[u8]) -> Result<RenderParameterKind, String>,
    ) -> Result<SerializedParameterBlock, String> {
        use RenderParameterKind::*;
        let flags = self.byte()?;
        let count = self.u32()?;
        let mut parameters = Vec::new();
        for _ in 0..count {
            let name = self.string()?;
            let kind = kind(&name)?;
            let value = match kind {
                Uniform | Option => SerializedParameterValue::Words([
                    self.u32()?,
                    self.u32()?,
                    self.u32()?,
                    self.u32()?,
                ]),
                Texture | MultisampleTexture | Program | String | Material | ParameterBlock => {
                    SerializedParameterValue::Name(self.string()?)
                }
                Sampler => SerializedParameterValue::SamplerThenProgram {
                    sampler: self.string()?,
                    program: self.string()?,
                },
                ReadOnlyBuffer
                | ReadWriteBuffer
                | ConstantBuffer
                | Struct
                | StructuredBuffer
                | StructuredConstantBuffer
                | Semantic => SerializedParameterValue::NoPayload,
            };
            parameters.push(SerializedParameter { name, kind, value });
        }
        Ok(SerializedParameterBlock { flags, parameters })
    }
}

impl RenderProgramDefinition {
    /// 1402beb30 reads these 0x24 bytes straight into the native state at +8;
    /// unlike surrounding serialized words, there is NO byte swap. The x64
    /// state factories therefore consume nine little-endian memory words.
    pub fn render_state(&self) -> [u32; 9] {
        std::array::from_fn(|i| {
            u32::from_le_bytes(self.render_state[i * 4..i * 4 + 4].try_into().unwrap())
        })
    }
}

pub fn parse_program_definition(
    bytes: &[u8],
    mut kind: impl FnMut(&[u8]) -> Result<RenderParameterKind, String>,
) -> Result<(RenderProgramDefinition, &[u8]), String> {
    let mut reader = VariantReader { bytes };
    let magic = reader.u32()?;
    if magic != 0x8000_003f {
        return Err(format!("unsupported program-definition magic {magic:08x}"));
    }
    let source_crc = reader.u32()?;
    let shared_names = reader.byte()? != 0;
    let mut includes = Vec::new();
    for _ in 0..reader.u32()? {
        let name = reader.name(shared_names)?;
        includes.push(VariantIdentity {
            name,
            checksum: reader.u32()?,
        });
    }
    let render_state = reader.take(36)?.try_into().unwrap();
    let defaults = reader.parameter_block(&mut kind)?;
    let specialization_defaults = reader.parameter_block(&mut kind)?;
    let mut feature_flags = 0;
    for bit in 1..8 {
        feature_flags |= (reader.byte()? & 1) << bit;
    }
    let bindings = reader.bindings()?;
    let layout = reader.layout()?;
    let stage_bindings = reader.bindings()?;
    let stage_layout = reader.take(45)?.try_into().unwrap();
    let mut records = Vec::new();
    for _ in 0..reader.u32()? {
        let third = reader.u32()?;
        records.push([reader.u32()?, reader.u32()?, third]);
    }
    let source_name = reader.string()?;
    Ok((
        RenderProgramDefinition {
            source_crc,
            shared_names,
            includes,
            render_state,
            defaults,
            specialization_defaults,
            feature_flags,
            bindings,
            layout,
            stage_bindings,
            stage_layout,
            records,
            source_name,
        },
        reader.bytes,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RenderProgramPlatform {
    Pc = 0,
    Durango = 1,
    Orbis = 2,
    Mantle = 3,
    PcD3d12 = 4,
}

impl RenderProgramPlatform {
    /// FUN_1402bf210, pointer table 14229bbd0. No unsupported index fallback.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pc => "PC",
            Self::Durango => "DURANGO",
            Self::Orbis => "ORBIS",
            Self::Mantle => "MANTLE",
            Self::PcD3d12 => "PC_D3D12",
        }
    }
}

/// The native formatter negates signed values then uses IDIV and an alphabet
/// lookup. Overflow indexes before that alphabet; do not invent a cache key
/// for that unresolved/invalid formatter input.
pub fn shader_variant_key(parameters: &[(&[u8], i32)]) -> Result<Vec<u8>, String> {
    let mut key = Vec::new();
    for &(name, value) in parameters {
        key.push(b'+');
        key.extend_from_slice(name.split(|&byte| byte == 0).next().unwrap());
        key.push(b'[');
        value
            .checked_abs()
            .ok_or("shader parameter decimal formatting overflows")?;
        key.extend_from_slice(value.to_string().as_bytes());
        key.push(b']');
    }
    Ok(key)
}

pub fn shader_variant_hash(parameters: &[(&[u8], i32)]) -> Result<u32, String> {
    Ok(crate::entities::declaration_name_hash(&shader_variant_key(
        parameters,
    )?))
}

/// FUN_1402bf9c0: machine code masks AL to four bits per hex digit; the
/// decompiler's full-word 0xffffff0f is not the emitted character value.
pub fn shader_binary_path(platform: RenderProgramPlatform, program: &str, variant: u32) -> String {
    let program = program.split('\0').next().unwrap();
    format!(
        "generated/renderprogs2/{}/{program}.{variant:08x}.release.vbin",
        platform.name()
    )
}

/// Optional archive entry only: NOT the resident cache, virtual filesystem or
/// D3D program-data parser used by 1402db460. The installed archives can contain
/// no `.vbin` entries; missing data remains unresolved, never an opaque program.
pub fn read_archived_shader_variant(
    catalog: &crate::archive::Catalog,
    platform: RenderProgramPlatform,
    program: &str,
    parameters: &[(&[u8], i32)],
) -> Result<Option<Vec<u8>>, String> {
    let path = shader_binary_path(platform, program, shader_variant_hash(parameters)?);
    catalog
        .read(&path)
        .map_err(|error| format!("{path}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_parameter_uses_declared_fallback_and_last_override_in_order() {
        let entry = |name: &[u8], value: i32| SerializedParameter {
            name: name.to_vec(),
            kind: RenderParameterKind::Option,
            value: SerializedParameterValue::Words([value as u32, 0, 0, 0]),
        };
        let fallback = SerializedParameterBlock {
            flags: 0,
            parameters: vec![
                entry(b"other", 7),
                entry(b"isShadow", 2),
                entry(b"ISSHADOW", 3),
            ],
        };
        let mut overrides = SerializedParameterBlock {
            flags: 0,
            parameters: vec![],
        };
        assert_eq!(
            overrides.integer_value(b"isShadow", 12, 1, None).unwrap(),
            1
        );
        assert_eq!(
            overrides
                .integer_value(b"isShadow", 12, 1, Some(&fallback))
                .unwrap(),
            3
        );
        overrides
            .parameters
            .extend([entry(b"isshadow", 4), entry(b"isSHADOW\0tail", -5)]);
        assert_eq!(
            overrides
                .integer_value(b"isShadow", 12, 1, Some(&fallback))
                .unwrap(),
            -5
        );
        assert_eq!(
            overrides
                .integer_value(b"isShadow", 0x10, 1, Some(&fallback))
                .unwrap(),
            -5
        );
        assert!(overrides.integer_value(b"isShadow", 1, 0, None).is_err());
        let empty = SerializedParameterBlock {
            flags: 0,
            parameters: vec![],
        };
        for (value, expected) in [
            (7.9f32, 7),
            (-7.9, -7),
            (f32::NAN, i32::MIN),
            (f32::INFINITY, i32::MIN),
            (2147483648.0, i32::MIN),
            (2147483520.0, 2147483520),
        ] {
            assert_eq!(
                empty
                    .integer_value(b"float", 0x30, value.to_bits(), None)
                    .unwrap(),
                expected
            );
        }
        assert_eq!(
            empty.integer_value(b"option", 0x3c, 123, None).unwrap(),
            123
        );
    }

    #[test]
    fn integer_parameter_matches_owned_memory_cases() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/parm_integer_cases.txt"
        ))
        .lines()
        {
            let mut fields = line.split_whitespace();
            let flags = fields.next().unwrap().parse::<u32>().unwrap();
            let default = fields.next().unwrap().parse::<u32>().unwrap();
            let mut block = || {
                let count = fields.next().unwrap().parse::<usize>().unwrap();
                let parameters = (0..count)
                    .map(|_| {
                        let id = fields.next().unwrap();
                        let name = if id == "0" {
                            b"target".to_vec()
                        } else {
                            id.as_bytes().to_vec()
                        };
                        let word = fields.next().unwrap().parse::<u32>().unwrap();
                        SerializedParameter {
                            name,
                            kind: RenderParameterKind::Option,
                            value: SerializedParameterValue::Words([word, 0, 0, 0]),
                        }
                    })
                    .collect();
                SerializedParameterBlock {
                    flags: 0,
                    parameters,
                }
            };
            let fallback = block();
            let overrides = block();
            let expected = fields.next().unwrap().parse::<i32>().unwrap();
            assert_eq!(
                overrides
                    .integer_value(b"target", flags, default, Some(&fallback))
                    .unwrap(),
                expected,
                "{line}"
            );
            assert!(fields.next().is_none());
            count += 1;
        }
        assert_eq!(count, 1824);
    }

    #[test]
    fn vector_words_preserve_bits_and_reject_nonnumeric_payloads() {
        let mut block = SerializedParameterBlock {
            flags: 0,
            parameters: vec![],
        };
        let words = [0x7fa0_1234, 0x8000_0000, 0x3f80_0000, 0xffff_ffff];
        assert_eq!(block.vector_words(b"wind", 0, words, None).unwrap(), words);
        assert!(block.vector_words(b"wind", 12, words, None).is_err());
        assert!(block
            .unsigned_integer_value(b"wind", 0x11, 0, None)
            .is_err());
        assert_eq!(
            block
                .unsigned_integer_value(b"wind", 0x30, 3e9f32.to_bits(), None)
                .unwrap(),
            0x8000_0000
        );
        block.parameters.push(SerializedParameter {
            name: b"WIND\0suffix".to_vec(),
            kind: RenderParameterKind::Uniform,
            value: SerializedParameterValue::Words(words),
        });
        assert_eq!(
            block.vector_words(b"wind", 0x30, [0; 4], None).unwrap(),
            words
        );
        block.parameters[0].value = SerializedParameterValue::Name(b"not numeric".to_vec());
        assert!(block.vector_words(b"wind", 0, [0; 4], None).is_err());
    }

    #[test]
    fn numeric_parameter_getters_match_executable() {
        fn block(fields: &mut impl Iterator<Item = u32>, count: usize) -> SerializedParameterBlock {
            SerializedParameterBlock {
                flags: 0,
                parameters: (0..count)
                    .map(|_| {
                        let name = match fields.next().unwrap() {
                            0 => b"wind".as_slice(),
                            1 => b"other",
                            _ => panic!("invalid fixture ID"),
                        };
                        SerializedParameter {
                            name: name.to_vec(),
                            kind: RenderParameterKind::Uniform,
                            value: SerializedParameterValue::Words(std::array::from_fn(|_| {
                                fields.next().unwrap()
                            })),
                        }
                    })
                    .collect(),
            }
        }
        let mut counts = [0; 3];
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/render_parameter_cases.txt"
        ))
        .lines()
        .filter(|line| !line.starts_with('#'))
        {
            let mut fields = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap());
            let kind = fields.next().unwrap() as usize;
            let flags = fields.next().unwrap();
            let default = std::array::from_fn(|_| fields.next().unwrap());
            let has_fallback = fields.next().unwrap() != 0;
            let fallback_count = fields.next().unwrap() as usize;
            let own_count = fields.next().unwrap() as usize;
            let fallback = block(&mut fields, fallback_count);
            let own = block(&mut fields, own_count);
            let expected: [u32; 4] = std::array::from_fn(|_| fields.next().unwrap());
            assert!(fields.next().is_none());
            let fallback = has_fallback.then_some(&fallback);
            let actual = match kind {
                0 => [
                    own.integer_value(b"wind", flags, default[0], fallback)
                        .unwrap() as u32,
                    0,
                    0,
                    0,
                ],
                1 => [
                    own.unsigned_integer_value(b"wind", flags, default[0], fallback)
                        .unwrap(),
                    0,
                    0,
                    0,
                ],
                2 => own.vector_words(b"wind", flags, default, fallback).unwrap(),
                _ => panic!("invalid fixture getter"),
            };
            assert_eq!(actual, expected, "{line}");
            counts[kind] += 1;
        }
        assert_eq!(counts, [256; 3]);
    }

    #[test]
    fn declaration_types_use_ascii_comparison_and_actual_type_not_name() {
        let types = [
            ("Vec", 0),
            ("Uniform", 0),
            ("Tex", 1),
            ("Tex2D", 1),
            ("Texture2D", 1),
            ("Texture2DArray", 1),
            ("Tex3D", 1),
            ("Texture3D", 1),
            ("TexCube", 1),
            ("TextureCube", 1),
            ("TextureCubeArray", 1),
            ("TexShadow2D", 1),
            ("TexShadow3D", 1),
            ("TexShadowCube", 1),
            ("Texture2DMS", 2),
            ("TexMultisample2D", 2),
            ("ROBuffer", 3),
            ("RWBuffer", 4),
            ("ConstantBuffer", 5),
            ("struct", 6),
            ("StructuredBuffer", 7),
            ("StructuredBufferAsCBuffer", 8),
            ("Sampler", 9),
            ("SamplerCmp", 9),
            ("Program", 10),
            ("String", 11),
            ("option", 12),
            ("Semantic", 13),
            ("SemanticArray", 13),
            ("Material", 14),
            ("Pblock", 15),
        ];
        for (name, kind) in types {
            for name in [
                name.to_owned(),
                name.to_ascii_uppercase(),
                name.to_ascii_lowercase(),
            ] {
                assert_eq!(
                    render_parameter_kind(&format!("// comment\n{{ {name} ignored }}")).unwrap()
                        as u8,
                    kind
                );
            }
        }
        assert!(render_parameter_kind("{ shadowMap value }").is_err());
        assert!(render_parameter_kind("{}").is_err());
        assert!(render_parameter_kind("Texture2D").is_err());
    }

    #[test]
    fn program_definition_all_value_kinds_wire_order_and_specialization_range() {
        use RenderParameterKind::*;
        let kinds = [
            Uniform,
            Texture,
            MultisampleTexture,
            ReadOnlyBuffer,
            ReadWriteBuffer,
            ConstantBuffer,
            Struct,
            StructuredBuffer,
            StructuredConstantBuffer,
            Sampler,
            Program,
            String,
            Option,
            Semantic,
            Material,
            ParameterBlock,
        ];
        let word = |out: &mut Vec<u8>, v: u32| out.extend(v.to_be_bytes());
        let string = |out: &mut Vec<u8>, v: &[u8]| {
            out.extend((v.len() as u32).to_le_bytes());
            out.extend(v);
        };
        let mut bytes = Vec::new();
        word(&mut bytes, 0x8000_003f);
        word(&mut bytes, 0x1234);
        bytes.push(1);
        word(&mut bytes, 1);
        bytes.extend(257u16.to_be_bytes());
        word(&mut bytes, 0xbeef);
        bytes.extend([0x96; 36]);
        bytes.push(0xa5);
        word(&mut bytes, kinds.len() as u32);
        for (i, kind) in kinds.iter().enumerate() {
            string(&mut bytes, &[i as u8]);
            match kind {
                Uniform | Option => {
                    for v in [0x7fc0_1234, 0x8000_0000, 0xffff_ffff, i as u32] {
                        word(&mut bytes, v);
                    }
                }
                Sampler => {
                    string(&mut bytes, b"sampler");
                    string(&mut bytes, b"program");
                }
                Texture | MultisampleTexture | Program | String | Material | ParameterBlock => {
                    string(&mut bytes, b"value")
                }
                _ => {}
            }
        }
        bytes.push(0x5a);
        word(&mut bytes, 0); // independent empty defaults
        bytes.extend([2, 3, 4, 5, 6, 7, 8]); // only low bit contributes
        word(&mut bytes, 3);
        bytes.push(0);
        for name in [b"unused".as_slice(), b"isShadow", b"useSkinning"] {
            word(&mut bytes, 0x1234);
            string(&mut bytes, name);
        }
        for value in [0, 0, 0, 0, 0, 0, 0, 1, 3u16] {
            bytes.extend(value.to_be_bytes());
        }
        word(&mut bytes, 0); // empty stage bindings have no flag
        bytes.extend(0..45u8);
        word(&mut bytes, 1);
        word(&mut bytes, 300);
        word(&mut bytes, 100);
        word(&mut bytes, 200);
        string(&mut bytes, b"source");
        let end = bytes.len();
        bytes.push(0xdd);
        let parse_kind = |name: &[u8]| {
            kinds
                .get(usize::from(*name.first().ok_or("empty kind name")?))
                .copied()
                .ok_or("unknown kind".into())
        };
        let (data, tail) = parse_program_definition(&bytes, parse_kind).unwrap();
        assert_eq!(tail, [0xdd]);
        assert_eq!(data.source_crc, 0x1234);
        assert_eq!(
            data.includes,
            [VariantIdentity {
                name: VariantName::Shared(257),
                checksum: 0xbeef
            }]
        );
        assert_eq!(data.render_state, [0x96; 36]);
        let mut state_fixture = data.clone();
        state_fixture.render_state = std::array::from_fn(|i| i as u8);
        assert_eq!(
            state_fixture.render_state(),
            [
                0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c, 0x13121110, 0x17161514, 0x1b1a1918,
                0x1f1e1d1c, 0x23222120
            ]
        );
        assert_eq!(data.defaults.flags, 0xa5);
        assert_eq!(data.defaults.parameters.len(), 16);
        assert_eq!(
            data.defaults.parameters[12].value,
            SerializedParameterValue::Words([0x7fc0_1234, 0x8000_0000, 0xffff_ffff, 12])
        );
        assert_eq!(
            data.defaults.parameters[9].value,
            SerializedParameterValue::SamplerThenProgram {
                sampler: b"sampler".to_vec(),
                program: b"program".to_vec()
            }
        );
        assert_eq!(
            data.defaults.parameters[7].value,
            SerializedParameterValue::NoPayload
        );
        assert_eq!(data.feature_flags, 0b0101_0100);
        assert_eq!(
            data.specialization_names(&[]).unwrap(),
            [b"isShadow".as_slice(), b"useSkinning"]
        );
        assert_eq!(data.stage_layout, std::array::from_fn(|i| i as u8));
        assert_eq!(data.records, [[100, 200, 300]]);
        assert_eq!(data.source_name, b"source");
        for length in 0..end {
            assert!(
                parse_program_definition(&bytes[..length], parse_kind).is_err(),
                "prefix {length}"
            );
        }
        let mut invalid = data.clone();
        invalid.layout[8] = 4;
        assert!(invalid.specialization_names(&[]).is_err());
        invalid.layout[7] = 4;
        invalid.layout[8] = 3;
        assert!(invalid.specialization_names(&[]).is_err());
        let mut empty = VariantReader {
            bytes: &(-1i32).to_le_bytes(),
        };
        assert!(empty.string().unwrap().is_empty());
    }

    #[test]
    fn d3d_body_mixed_endianness_binding_flags_and_all_bytecode_modes() {
        let mut bytes = Vec::new();
        let word = |bytes: &mut Vec<u8>, value: u32| bytes.extend(value.to_be_bytes());
        let inline = |bytes: &mut Vec<u8>, name: &[u8]| {
            bytes.extend((name.len() as u32).to_le_bytes());
            bytes.extend(name);
        };
        word(&mut bytes, D3D_VARIANT_MAGIC);
        word(&mut bytes, 0x1234);
        word(&mut bytes, 0xfeed);
        bytes.push(0);
        word(&mut bytes, 1); // include/parameter names inline
        inline(&mut bytes, b"include/name");
        word(&mut bytes, 0x9876);
        word(&mut bytes, 1);
        inline(&mut bytes, b"parameter");
        word(&mut bytes, 3);
        for (kind, code) in [(0, 0x8000_0000), (1, !14710u32), (5, 3)] {
            word(&mut bytes, kind);
            word(&mut bytes, 1);
            bytes.push(1); // independently shared binding names
            word(&mut bytes, 0xbeef);
            bytes.extend(257u16.to_be_bytes());
            for i in 0..9u16 {
                bytes.extend((256 + i).to_be_bytes());
            }
            word(&mut bytes, code);
            if code == 3 {
                bytes.extend(b"abc");
            }
        }
        word(&mut bytes, 0); // no sharing flag for an empty binding list
        for i in 0..9u16 {
            bytes.extend(i.to_be_bytes());
        }
        let end = bytes.len();
        bytes.extend([0xde, 0xad]);
        let (data, tail) = parse_d3d_variant(&bytes).unwrap();
        assert_eq!(tail, [0xde, 0xad]);
        assert_eq!(
            data.includes[0],
            VariantIdentity {
                name: VariantName::Inline(b"include/name".to_vec()),
                checksum: 0x9876
            }
        );
        assert_eq!(
            data.parameters,
            [VariantName::Inline(b"parameter".to_vec())]
        );
        assert_eq!(data.stages[0].bytecode, StageBytecode::Absent);
        assert_eq!(data.stages[1].bytecode, StageBytecode::Cached(14710));
        assert_eq!(
            data.stages[2].bytecode,
            StageBytecode::Inline(b"abc".to_vec())
        );
        assert_eq!(
            data.stages[0].bindings.parameters[0],
            VariantIdentity {
                name: VariantName::Shared(257),
                checksum: 0xbeef
            }
        );
        assert_eq!(
            data.stages[1].layout,
            [256, 257, 258, 259, 260, 261, 262, 263, 264]
        );
        assert_eq!(data.bindings.shared_names, None);
        assert_eq!(data.layout, [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        for length in 0..end {
            assert!(
                parse_d3d_variant(&bytes[..length]).is_err(),
                "prefix {length}"
            );
        }
        assert_eq!(
            VariantName::Shared(0).bytes(&[b"owned".to_vec()]).unwrap(),
            b"owned"
        );
        assert!(VariantName::Shared(1).bytes(&[]).is_err());
    }

    #[test]
    fn d3d_header_preserves_wire_endianness_flags_and_remaining_payload() {
        let bytes = [
            0x80, 0, 0, 0x4a, 0x12, 0x34, 0x56, 0x78, 0xfe, 0xdc, 0xba, 0x98, 2, 0, 0, 1, 2, 0xaa,
            0xbb,
        ];
        let (header, tail) = parse_d3d_variant_header(&bytes).unwrap();
        assert_eq!(
            header,
            D3dVariantHeader {
                source_crc: 0x12345678,
                flags: 0xfedcba98,
                shared_names: true,
                include_count: 258
            }
        );
        assert_eq!(tail, &[0xaa, 0xbb]);
        for length in 0..17 {
            assert!(parse_d3d_variant_header(&bytes[..length]).is_err());
        }
        let mut invalid = bytes;
        invalid[0] = 0;
        assert!(parse_d3d_variant_header(&invalid).is_err());
        invalid = bytes;
        invalid[12] = 0;
        assert!(!parse_d3d_variant_header(&invalid).unwrap().0.shared_names);
    }
    #[test]
    #[ignore = "requires the installed original archive indices"]
    fn archived_cache_roundtrip_and_missing_remains_unresolved() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let mut checked = 0;
        for path in catalog.paths_with_suffix(".release.vbin") {
            let Some(relative) = path.strip_prefix("generated/renderprogs2/pc/") else {
                continue;
            };
            let (program, hash) = relative
                .strip_suffix(".release.vbin")
                .unwrap()
                .rsplit_once('.')
                .unwrap();
            assert_eq!(hash.len(), 8);
            let hash = u32::from_str_radix(hash, 16).unwrap();
            let path = shader_binary_path(RenderProgramPlatform::Pc, program, hash);
            assert_eq!(path.to_ascii_lowercase(), path);
            let binary = catalog.read(&path).unwrap().unwrap();
            assert_eq!(Some(binary.len()), catalog.resource_size(&path));
            assert!(!binary.is_empty());
            checked += 1;
            if checked == 16 {
                break;
            }
        }
        assert!(catalog.contains("generated/decls/renderprog/arkdefault.decl"));
        assert!(read_archived_shader_variant(
            &catalog,
            RenderProgramPlatform::Pc,
            "__owned_missing_variant_probe__",
            &[]
        )
        .unwrap()
        .is_none());
        eprintln!("archived PC binaries checked={checked}; missing stays unresolved (resident cache/parser not tested)");
    }

    #[test]
    fn variant_names_preserve_order_case_integer_and_c_string_rules() {
        let params: &[(&[u8], i32)] = &[
            (b"isShadow", 1),
            (b"useSkinning\0ignored", 0),
            (b"signed", -2147483647),
        ];
        assert_eq!(
            shader_variant_key(params).unwrap(),
            b"+isShadow[1]+useSkinning[0]+signed[-2147483647]"
        );
        assert_ne!(
            shader_variant_hash(params).unwrap(),
            shader_variant_hash(&[params[1], params[0], params[2]]).unwrap()
        );
        assert_eq!(shader_variant_hash(&[]).unwrap(), 0);
        assert!(shader_variant_hash(&[(b"signed", i32::MIN)]).is_err());
        assert_eq!(
            shader_binary_path(RenderProgramPlatform::Pc, "arkDefault\0ignored", 0x12ab),
            "generated/renderprogs2/PC/arkDefault.000012ab.release.vbin"
        );
        for platform in [
            RenderProgramPlatform::Pc,
            RenderProgramPlatform::Durango,
            RenderProgramPlatform::Orbis,
            RenderProgramPlatform::Mantle,
            RenderProgramPlatform::PcD3d12,
        ] {
            assert!(shader_binary_path(platform, "test", u32::MAX)
                .ends_with("/test.ffffffff.release.vbin"));
        }
    }
}
