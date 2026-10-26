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
    /// Slot6 is the alpha prepass used for volume boundary marking. Its
    /// empty PS640 and position-only vertex stages need no colour attachment.
    pub fn water_stencil_source(
        &mut self, name: &[u8], overrides: &SerializedParameterBlock,
    ) -> Result<Option<Arc<CachedWaterDepthSource>>> {
        let Some(hash)=self.specialization_hash(name,overrides)? else {return Ok(None)};
        let Some(variant)=self.serialized_variant(name,overrides)? else {return Ok(None)};
        let pair=self.stage_checksums(&variant)?;
        let hex=|value:Option<[u8;16]>|value.map(|v|v.iter().map(|b|format!("{b:02x}")).collect::<String>());
        let actual=pair.map(hex);
        if actual[1].as_deref()!=Some("c1a10697307ed5a825c419158737cc9f") {
            return Err(format!("unsupported water stencil pixel stage {:?}",actual[1]));
        }
        let class=match actual[0].as_deref() {
            Some("d29749d093a59a239c0fb51d87ac4c30"|"fc98e66346f349a97e837db36ca6a102"|"ba16e8625b5dcd329749e170980574cc")=>CachedWaterDepthClass::Rigid,
            Some("b5201c0a2e3df3004a154aa9c7a4c70b"|"47c79de56ba06d13e08ba8fd8066e90d"|"644bb6426ebbae813a49c82378850504")=>CachedWaterDepthClass::World,
            _=>return Err(format!("unsupported water stencil vertex stage {:?}",actual[0])),
        };
        Ok(Some(Arc::new(CachedWaterDepthSource{class,program:name.to_vec(),variant_hash:hash,variant})))
    }
    pub fn model_water_colour_class(
        &mut self,
        variant: &D3dVariantData,
    ) -> Result<crate::rendering::water::program::WorldWaterColourClass> {
        let pair = self.stage_checksums(variant)?;
        // Installed rigid horizontal VS14451/14463/14474: position-only
        // world/clip transforms. Their fragments do not consume probe indices.
        let supported = [
            (
                "d29749d093a59a239c0fb51d87ac4c30",
                "0a1cf914f92ae63fb64778aa01507ccf",
            ),
            (
                "fc98e66346f349a97e837db36ca6a102",
                "32c29801be45bdc244de3e9d534d64f8",
            ),
            (
                "ba16e8625b5dcd329749e170980574cc",
                "ac053e35907bbb32b9356ee08262a955",
            ),
        ];
        let hex = |bytes: Option<[u8; 16]>| {
            bytes.map(|b| b.iter().map(|v| format!("{v:02x}")).collect::<String>())
        };
        let actual = pair.map(hex);
        let probes = [
            ("4d9546d195afdcd738681477a2e1045e", "5da304690551bd71dc0f9c20ca37614c"),
            ("5555deb4610383c2391d907c7ae8d42d", "d117987c5ad5933ee06133336cfda1ca"),
        ];
        if probes.iter().any(|(v,p)|actual[0].as_deref()==Some(v)&&actual[1].as_deref()==Some(p)) {
            return Ok(crate::rendering::water::program::WorldWaterColourClass::ProbeHorizontal);
        }
        if supported
            .iter()
            .any(|(v, p)| actual[0].as_deref() == Some(v) && actual[1].as_deref() == Some(p))
        {
            Ok(crate::rendering::water::program::WorldWaterColourClass::Horizontal)
        } else {
            Err(format!("unsupported model water colour stages {pair:02x?}"))
        }
    }
    pub(super) fn stage_checksums(
        &mut self,
        variant: &D3dVariantData,
    ) -> Result<[Option<[u8; 16]>; 2]> {
        let mut pair = [None; 2];
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
                return Err("invalid water colour DXBC container".into());
            }
            let slot = pair
                .get_mut(stage.kind as usize)
                .ok_or("unsupported water colour stage kind")?;
            if slot.is_some() {
                return Err("duplicate water colour stage".into());
            }
            *slot = Some(<[u8; 16]>::try_from(&bytes[4..20]).unwrap());
        }
        Ok(pair)
    }
    pub fn world_water_colour_class(
        &mut self,
        variant: &D3dVariantData,
    ) -> Result<crate::rendering::water::program::WorldWaterColourClass> {
        let pair = self.stage_checksums(variant)?;
        // VS14490/14494 have identical WORLD/PNW output for all 512 native
        // stream-output cases, including first-instance 0/2/7. The corresponding
        // PS14491/14495 differ by the source's SSR specialization.
        let supported = [
            (
                [
                    0x28, 0x8c, 0x1c, 0x55, 0xb1, 0x3e, 0x90, 0x53, 0x29, 0xbb, 0x26, 0x9d, 0xae,
                    0xb8, 0x2b, 0xb3,
                ],
                [
                    0xcb, 0x54, 0x40, 0x7c, 0xfa, 0x50, 0x00, 0x37, 0xb8, 0xe2, 0xa6, 0x92, 0x51,
                    0x50, 0x46, 0x04,
                ],
            ),
            (
                [
                    0x18, 0xde, 0xd5, 0xaf, 0xf3, 0x84, 0x6f, 0x42, 0x4c, 0x81, 0x10, 0xc1, 0x1e,
                    0x9b, 0x25, 0xc7,
                ],
                [
                    0xc6, 0xbe, 0x74, 0x4d, 0x88, 0xf3, 0x4b, 0x51, 0xcd, 0x9e, 0x95, 0x0a, 0xf3,
                    0xb1, 0x05, 0x56,
                ],
            ),
        ];
        // These horizontal variants have no IBL probes; their world vertex
        // stages only transform position (different PerInstanceCB offsets).
        let horizontal = [
            (
                [
                    0xb5, 0x20, 0x1c, 0x0a, 0x2e, 0x3d, 0xf3, 0x00, 0x4a, 0x15, 0x4a, 0xa9, 0xc7,
                    0xa4, 0xc7, 0x0b,
                ],
                [
                    0xf6, 0xec, 0x7b, 0x02, 0xd6, 0x04, 0xcc, 0x79, 0xc5, 0xe5, 0x6f, 0x3d, 0x1f,
                    0xb8, 0xff, 0x07,
                ],
            ),
            (
                [
                    0x47, 0xc7, 0x9d, 0xe5, 0x6b, 0xa0, 0x6d, 0x13, 0xe0, 0x8b, 0xa8, 0xfd, 0x80,
                    0x66, 0xe9, 0x0d,
                ],
                [
                    0x3e, 0xfc, 0x49, 0xc9, 0xce, 0x21, 0xfb, 0x22, 0x6a, 0x37, 0x41, 0xec, 0xc7,
                    0x6d, 0xd1, 0x81,
                ],
            ),
            (
                [
                    0x64, 0x4b, 0xb6, 0x42, 0x6e, 0xbb, 0xae, 0x81, 0x3a, 0x49, 0xc8, 0x23, 0x78,
                    0x85, 0x05, 0x04,
                ],
                [
                    0x16, 0xe2, 0xab, 0x0d, 0xa5, 0xa5, 0xe9, 0x95, 0x91, 0x1f, 0xc3, 0xc1, 0x3c,
                    0x4d, 0x45, 0x3a,
                ],
            ),
        ];
        if horizontal.iter().any(|(v, p)| pair == [Some(*v), Some(*p)]) {
            Ok(crate::rendering::water::program::WorldWaterColourClass::Horizontal)
        } else if supported.iter().any(|(v, p)| pair == [Some(*v), Some(*p)]) {
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
