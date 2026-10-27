//! BIM volume records: mip-major, then Z slices, with a two-dimensional
//! payload header for each slice. Installed colour cubes use this layout.
use super::{compressed_mip, word, BimageInfo};

pub struct InlineVolumeChain {
    pub info: BimageInfo,
    pub depth: u32,
    pub payload: Vec<u8>,
}

pub fn compressed_inline_volume(bytes: &[u8]) -> Result<InlineVolumeChain, String> {
    let info = BimageInfo::parse(bytes)?;
    // +32/+36 retain width/height fields; +40 is slice depth. Cubic colour
    // LUTs mask that distinction, while installed256x256x64 FX volumes do not.
    let depth = word(bytes, 40)?;
    if word(bytes, 8)? != 1
        || word(bytes, 52)? != 1
        || info.width == 0
        || info.height == 0
        || depth == 0
        || !(1..=16).contains(&info.mip_count)
    {
        return Err("invalid BIM volume descriptor".into());
    }
    let records: u64 = (0..info.mip_count)
        .map(|m| u64::from((depth >> m).max(1)))
        .sum();
    if records > bytes.len().saturating_sub(62) as u64 / 20 {
        return Err("truncated BIM volume slice records".into());
    }
    let mut at = 62usize;
    let mut payload = Vec::new();
    for level in 0..info.mip_count {
        let width = (info.width >> level).max(1);
        let height = (info.height >> level).max(1);
        for slice in 0..(depth >> level).max(1) {
            let header = bytes
                .get(at..at + 20)
                .ok_or("truncated volume slice header")?;
            let size = word(header, 16)? as usize;
            let end = at
                .checked_add(20)
                .and_then(|n| n.checked_add(size))
                .ok_or("volume slice record overflow")?;
            let record = bytes.get(at..end).ok_or("truncated volume slice payload")?;
            let mip = compressed_mip(record, level, info.format)?;
            if word(record, 4)? != slice || (mip.width, mip.height) != (width, height) {
                return Err("volume slice/mip descriptor mismatch".into());
            }
            payload.extend_from_slice(mip.payload);
            at = end;
        }
    }
    if at != bytes.len() {
        return Err("volume mip chain size mismatch".into());
    }
    Ok(InlineVolumeChain {
        info,
        depth,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed archive colour cubes"]
    fn installed_colour_cubes_preserve_srgb_and_all_32_slices() {
        let c = crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
            .unwrap();
        let mut count = 0;
        for path in c
            .paths_containing("colorcube")
            .into_iter()
            .filter(|p| p.ends_with(".bimage7"))
        {
            let bytes = c.read(&path).unwrap().unwrap();
            let volume = compressed_inline_volume(&bytes).unwrap();
            assert_eq!(
                (
                    volume.info.width,
                    volume.info.height,
                    volume.depth,
                    volume.info.mip_count
                ),
                (32, 32, 32, 1),
                "{path}"
            );
            assert_eq!(
                volume.info.format,
                super::super::ImageFormat::Rgba8Srgb,
                "{path}"
            );
            assert_eq!(volume.payload.len(), 32 * 32 * 32 * 4);
            count += 1;
        }
        assert!(count > 20);
        println!("{count} installed colour cubes validated");
    }
}
