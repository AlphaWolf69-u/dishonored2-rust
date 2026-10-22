//! Shared downsample allocation from 36f8a0/372db0 and mip counts from 2e9860.
pub fn downsample_extent(full: [u32; 2], quality: i32) -> Result<[u32; 2], String> {
    if full.contains(&0) || full.iter().any(|&v| v > i32::MAX as u32 - 15) {
        return Err(format!("invalid downsample view extent {full:?}"));
    }
    if !(1..=2).contains(&quality) {
        return Err(format!("unsupported r_downsampleQuality {quality}"));
    }
    let minimum = 4 << (quality - 1);
    Ok(full.map(|v| (v >> quality).max(minimum)))
}

pub fn pyramid_extents(full: [u32; 2], quality: i32) -> Result<Vec<[u32; 2]>, String> {
    let [width, height] = downsample_extent(full, quality)?;
    let mut counts = [0; 4];
    crate::texture::initialize_image_mip_counts(11, [width as i32, height as i32, 1], &mut counts);
    Ok((0..counts[0]).map(|i| [width >> i, height >> i]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_preserves_quality_minimum_and_short_axis_mip_stop() {
        assert_eq!(downsample_extent([1921, 1081], 1).unwrap(), [960, 540]);
        assert_eq!(downsample_extent([1921, 1081], 2).unwrap(), [480, 270]);
        assert_eq!(downsample_extent([3, 3], 1).unwrap(), [4, 4]);
        assert_eq!(downsample_extent([3, 3], 2).unwrap(), [8, 8]);
        assert_eq!(
            pyramid_extents([38, 18], 1).unwrap(),
            [[19, 9], [9, 4], [4, 2], [2, 1]]
        );
        assert!(downsample_extent([1920, 1080], 0).is_err());
    }
}
