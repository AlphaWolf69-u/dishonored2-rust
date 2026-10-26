//! Game shape-tag material/filter decoding (853b80 and 81a5f0).
use super::collision_hit::ShapeHitInfo;

/// The body/resource owner supplies its material-index map and the world's
/// physics-material flags. Neither is a rendering material or an item index.
pub trait ShapeTagSource {
    fn mapped_material(&self, index: usize) -> Result<u16, String>;
    fn material_flags(&self, material: u16) -> Result<u16, String>;
}

/// 81a5f0 edits only the collision-filter bits controlled by material flags.
pub fn material_filter(filter: u32, flags: u16) -> u32 {
    if flags == 0x7f {
        return filter;
    }
    if flags == 0 {
        return filter & 0x0820_001f;
    }
    let low = flags as u8;
    let mut cleared = 0u32;
    for (flag, bits) in [
        (1, (1 << 14) | (1 << 15)),
        (2, 1 << 11),
        (4, (1 << 16) | (1 << 25)),
        (8, 1 << 22),
        (16, 1 << 8),
        (32, (1 << 5) | (1 << 6) | (1 << 7)),
        (64, 1 << 9),
    ] {
        if low & flag == 0 {
            cleared |= bits;
        }
    }
    if low & 0x81 == 0 {
        cleared |= 1 << 19;
    }
    filter & !cleared
}

/// 853b80. A missing body or invalid tag leaves the packet untouched. Direct
/// material tags change only its material; mapped tags also apply material flags.
pub fn decode_shape_tag(
    tag: u16,
    source: Option<&(impl ShapeTagSource + ?Sized)>,
    hit: &mut ShapeHitInfo,
) -> Result<(), String> {
    let Some(source) = source else {
        return Ok(());
    };
    if tag == u16::MAX {
        return Ok(());
    }
    if tag & 0x8000 != 0 {
        hit.material_id = tag & 0x7fff;
        return Ok(());
    }
    let index = usize::from(hit.material_id) + usize::from(tag & 0x7fff);
    let material = source.mapped_material(index)?;
    let filter = material_filter(hit.filter_info, source.material_flags(material)?);
    hit.material_id = material;
    hit.filter_info = filter;
    Ok(())
}

#[cfg(test)]
#[path = "shape_tags/tests.rs"]
mod tests;
