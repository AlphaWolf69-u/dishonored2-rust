//! Environment-cubemap data transport: 291350 normal packing and 2922c0 planes.
use crate::bprobe;

pub fn pack_plane(normal: [f32; 3], distance: f32) -> [u32; 2] {
    let bytes = normal.map(|x| {
        let x = (x + 1.) * 127.;
        let i = if (-2147483648.0..2147483648.0).contains(&x) {
            x as i32
        } else {
            i32::MIN
        };
        i.clamp(0, 255) as u32
    });
    [
        bytes[0] | bytes[1] << 8 | bytes[2] << 16,
        distance.to_bits(),
    ]
}

pub fn probe_words(probe: &bprobe::Probe) -> [u32; 16] {
    let mut words = [0; 16];
    for i in 0..6 {
        let p = if probe.reflection_parallax {
            probe.virtual_planes[i]
        } else {
            let mut p = [0., 0., 0., f32::from_bits(0x49742400)];
            p[i / 2] = if i % 2 == 0 { -1. } else { 1. };
            p
        };
        words[i * 2..i * 2 + 2].copy_from_slice(&pack_plane(p[..3].try_into().unwrap(), p[3]));
    }
    if probe.reflection_parallax {
        words[12..15].copy_from_slice(&probe.position.map(f32::to_bits));
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantized_planes_match_executable() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/reflection_plane.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let w: Vec<_> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            assert_eq!(
                pack_plane(
                    std::array::from_fn(|i| f32::from_bits(w[i])),
                    f32::from_bits(w[3])
                ),
                [w[4], w[5]],
                "case{count}"
            );
            count += 1;
        }
        assert_eq!(count, 128);
    }
}
