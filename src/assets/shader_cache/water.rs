//! Exact cached stage admission for the already-verified placed-water depth ABI.
use super::{name_key, ProgramLibrary, Result};
use crate::render_program::{D3dVariantData, SerializedParameterBlock, StageBytecode};
use std::sync::Arc;

#[cfg(test)]
#[path = "water_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedWaterDepthClass {
    Rigid,
    World,
}

#[derive(Clone, Debug)]
pub struct CachedWaterDepthSource {
    pub class: CachedWaterDepthClass,
    pub program: Vec<u8>,
    pub variant_hash: u32,
    pub variant: D3dVariantData,
}

impl ProgramLibrary {
    /// Pass/instancing overrides are supplied by the material constructor.
    /// A resident miss is distinct from an unsupported bytecode pair. Memoize
    /// both so materials sharing a specialization never reread its stages.
    pub fn water_depth_source(
        &mut self,
        name: &[u8],
        overrides: &SerializedParameterBlock,
    ) -> Result<Option<Arc<CachedWaterDepthSource>>> {
        let Some(hash) = self.specialization_hash(name, overrides)? else {
            return Ok(None);
        };
        let key = (name_key(name), hash);
        if let Some(result) = self.water_depth_sources.get(&key) {
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
                    StageBytecode::Absent => continue,
                    StageBytecode::Inline(bytes) => bytes.clone(),
                    StageBytecode::Cached(id) => self.cache.stage(*id as usize)?.bytes,
                };
                if bytes.len() < 32
                    || bytes.get(..4) != Some(b"DXBC".as_slice())
                    || u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize != bytes.len()
                {
                    return Err("invalid water depth DXBC extent/container".into());
                }
                let checksum: [u8; 16] = bytes[4..20].try_into().unwrap();
                match stage.kind {
                    0 if vertex.is_none() => vertex = Some(checksum),
                    1 if pixel.is_none() => pixel = Some(checksum),
                    _ => {
                        return Err(format!(
                            "unsupported/duplicate water depth stage {}",
                            stage.kind
                        ))
                    }
                }
            }
            // Actual cached PS31: only motion channels. VS14476 is the rigid
            // MVP/depth-hack ABI; VS14487 indexes raw world instance matrices.
            // Stage IDs are provenance; selection uses the actual bytecode.
            if pixel
                != Some([
                    0xc8, 0x86, 0x3d, 0xd4, 0x7f, 0x03, 0xf2, 0x48, 0x50, 0x6b, 0x7c, 0x61, 0xad,
                    0x74, 0xc5, 0xf0,
                ])
            {
                return Err(format!("unsupported water depth pixel stage {pixel:02x?}"));
            }
            let class = match vertex {
                Some(
                    [0x0d, 0x23, 0x97, 0xfb, 0xd5, 0xee, 0xc0, 0x20, 0xee, 0xfc, 0x2a, 0xf2, 0x78, 0x7e, 0x61, 0xf2],
                ) => CachedWaterDepthClass::Rigid,
                Some(
                    [0x89, 0xed, 0xfc, 0xb5, 0xe9, 0xbf, 0x6f, 0x05, 0xcf, 0xe2, 0x15, 0xef, 0xff, 0x6f, 0x35, 0xab],
                ) => CachedWaterDepthClass::World,
                _ => {
                    return Err(format!(
                        "unsupported water depth vertex stage {vertex:02x?}"
                    ))
                }
            };
            Ok(Some(Arc::new(CachedWaterDepthSource {
                class,
                program: name.to_vec(),
                variant_hash: hash,
                variant,
            })))
        })();
        self.water_depth_sources.insert(key, result.clone());
        result
    }
}
