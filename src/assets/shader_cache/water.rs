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
    pub fn world_water_colour_class(
        &self,
        variant: &D3dVariantData,
    ) -> Result<crate::rendering::water::program::WorldWaterColourClass> {
        let mut pair = [None; 2];
        for stage in &variant.stages {
            let bytes = match &stage.bytecode {
                StageBytecode::Absent => continue,
                StageBytecode::Inline(bytes) => bytes.clone(),
                StageBytecode::Cached(id) => self.cache.stage(*id as usize)?.bytes,
            };
            if bytes.len() < 32 || bytes.get(..4) != Some(b"DXBC".as_slice())
                || u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize != bytes.len()
            {
                return Err("invalid water colour DXBC container".into());
            }
            let slot = pair.get_mut(stage.kind as usize)
                .ok_or("unsupported water colour stage kind")?;
            if slot.is_some() { return Err("duplicate water colour stage".into()); }
            *slot = Some(<[u8;16]>::try_from(&bytes[4..20]).unwrap());
        }
        // VS14490/14494 have identical WORLD/PNW output for all 512 native
        // stream-output cases, including first-instance 0/2/7. The corresponding
        // PS14491/14495 differ by the source's SSR specialization.
        let supported = [
            ([0x28,0x8c,0x1c,0x55,0xb1,0x3e,0x90,0x53,0x29,0xbb,0x26,0x9d,0xae,0xb8,0x2b,0xb3],
             [0xcb,0x54,0x40,0x7c,0xfa,0x50,0x00,0x37,0xb8,0xe2,0xa6,0x92,0x51,0x50,0x46,0x04]),
            ([0x18,0xde,0xd5,0xaf,0xf3,0x84,0x6f,0x42,0x4c,0x81,0x10,0xc1,0x1e,0x9b,0x25,0xc7],
             [0xc6,0xbe,0x74,0x4d,0x88,0xf3,0x4b,0x51,0xcd,0x9e,0x95,0x0a,0xf3,0xb1,0x05,0x56]),
        ];
        if supported.iter().any(|(v,p)| pair == [Some(*v),Some(*p)]) {
            Ok(crate::rendering::water::program::WorldWaterColourClass::ProbeHorizontal)
        } else {
            Err(format!("unsupported world water colour stages {pair:02x?}"))
        }
    }

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
