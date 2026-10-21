//! Original depth-informed tiled-forward contracts, not Bevy cluster substitutes.
//!
//! FUN_141369c60 registers tlf_TileDim with default "16", integer bounds 2..32.
//! FUN_1402f8b60 uploads floor(log2(tile_dim)) and the four inverse dimensions.
//! Native source lightcontext.renderprog.decl:164-267 supplies 9/23-bit links.
//! Depth/cull shaders and fixed buffer capacity/reset have native GPU fixtures.
//! Live use still needs separate opaque/alpha depth ownership, upstream scene
//! light/shadow inputs, packet-to-light index mapping and forward consumption.
//! Bevy's merged depth and point/spot cluster ranges are not those contracts.

pub const DEFAULT_TILE_DIMENSION: u32 = 16;
pub const END_OF_LIGHT_LIST: u32 = (1 << 23) - 1;
// Installed tlf/common: avoid mapping the saturated endpoint to bin 32.
pub const DEPTH_BIN_SCALE: f32 = 31.999;
// FUN_140371750, analysis/decompiled/chunk_000126.c:4176-4269:
// +c768:448 cull packets; +c778:448 lighting packets; +c770:256 shadows;
// +c790:1,152,000 four-byte links; +c798:one four-byte UAV counter.
// These are original fixed allocations, not derived from tile dimensions.
pub const LIGHT_BUFFER_CAPACITY: usize = 0x1c0;
pub const SHADOW_BUFFER_CAPACITY: usize = 0x100;
pub const LIGHT_NODE_POOL_CAPACITY: usize = 0x119400;
pub const LIGHT_NODE_COUNTER_BYTES: u64 = 4;

/// 2f7260/2fc130's tlf/unprojectZ upload from the logical projection rows.
/// This is distinct from world-fog unprojection: Depth_to_Z negates X again.
pub fn depth_unprojection(projection: &[[f32;4];4]) -> [f32;2] {
    [f32::from_bits(projection[2][3].to_bits()^0x8000_0000),projection[2][2]]
}

/// FUN_1402f7260 admits these records in gathered-light order, before assigning
/// the same index to the culling and 112-byte lighting buffers.
pub fn includes_local_light(view_flags: u8, visibility_mask: u32, kind: i32, flags: u8) -> bool {
    (!(view_flags >> 4) as u32 & visibility_mask & 3) != 0 && kind < 4 && flags & 1 != 0
}

/// FUN_140231590's row-major affine transform, without a homogeneous divide.
pub fn light_view_position(view: &[[f32; 4]; 3], origin: [f32; 3]) -> [f32; 3] {
    [
        ((view[0][1] * origin[1] + origin[0] * view[0][0]) + view[0][2] * origin[2]) + view[0][3],
        ((view[1][0] * origin[0] + view[1][1] * origin[1]) + view[1][2] * origin[2]) + view[1][3],
        ((view[2][0] * origin[0] + view[2][1] * origin[1]) + view[2][2] * origin[2]) + view[2][3],
    ]
}

/// FUN_1402f7260, 1402f78f7..1402f79a9: copy the prepared lighting record into
/// its matching cull packet. The native stores replace only the low 16 flag bits.
pub fn fill_light_cull_packet(
    view_position: [f32; 3],
    lighting: &[u32; 28],
    packet: &mut [u32; 12],
) {
    packet[..3].copy_from_slice(&view_position.map(f32::to_bits));
    for (dst, src) in (3..11).zip([7, 8, 9, 10, 13, 16, 17, 18]) {
        packet[dst] = lighting[src];
    }
    packet[11] = (packet[11] & 0xffff_0000) | (lighting[25] & 0xffff);
}

/// FUN_1402f7260: preserve lighting order, transform each position and round the
/// cull buffer to 64 lanes. Padding records retain only the native flag bits.
/// The allocation has 448 lanes, but INC at 1402f78e8 precedes CMP/JNC at
/// 1402f79a9: 448 selected lights is fatal; 447 may round to 448 cull lanes.
pub fn prepare_light_cull_packets(
    view: &[[f32; 4]; 3],
    lighting: &[[u32; 28]],
    packets: &mut [[u32; 12]],
) -> Result<usize, String> {
    let count = lighting.len();
    let padded = (count + 63) & !63;
    if count >= LIGHT_BUFFER_CAPACITY || padded > packets.len() {
        return Err("Too many lights in view, probably during probe shooting !\n".into());
    }
    for (record, packet) in lighting.iter().zip(packets.iter_mut()) {
        let origin = std::array::from_fn(|axis| f32::from_bits(record[axis]));
        fill_light_cull_packet(light_view_position(view, origin), record, packet);
    }
    for packet in &mut packets[count..padded] {
        packet[..3].fill(0x7f7f_ffff);
        packet[3] = (-1.0_f32).to_bits();
        packet[4..7].copy_from_slice(&[0, 0, 1.0_f32.to_bits()]);
        packet[7..11].fill(0);
        packet[11] = (packet[11] & 0xffff_ff20) | 0xff20;
    }
    Ok(padded)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileGrid {
    pub tile_dimension: u32,
    pub lighting_shift: u32,
    pub dimensions: [u32; 2],
    pub inverse_dimensions: [f32; 4],
}

impl TileGrid {
    pub fn new(width: u32, height: u32, tile_dimension: u32) -> Result<Self, String> {
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(
                "tiled-lighting view dimensions must be positive signed integers".into(),
            );
        }
        if !(2..=32).contains(&tile_dimension) {
            return Err("tlf_TileDim is outside its original 2..32 integer bounds".into());
        }
        let tile = tile_dimension as f32;
        // The original uses the float exponent, even for non-powers of two.
        let lighting_shift = ((tile.to_bits() >> 23) & 255) - 127;
        let dimensions = [
            width.div_ceil(tile_dimension),
            height.div_ceil(tile_dimension),
        ];
        Ok(Self {
            tile_dimension,
            lighting_shift,
            dimensions,
            inverse_dimensions: [
                tile / width as f32,
                tile / height as f32,
                dimensions[0] as f32,
                dimensions[1] as f32,
            ],
        })
    }

    /// Native integer pixel addressing: four horizontal head planes.
    pub fn light_head_coordinate(self, pixel: [u32; 2], light_type: u32) -> [u32; 2] {
        [
            (pixel[0] >> self.lighting_shift) + light_type * self.dimensions[0],
            pixel[1] >> self.lighting_shift,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedLightNode(pub u32);

impl PackedLightNode {
    /// The original shader ORs the index without an extra mask. Producers must
    /// obey its light-count contract; silently truncating a bad index is not it.
    pub fn new(previous_head: u32, light_index: u32) -> Self {
        Self(previous_head.wrapping_shl(9) | light_index)
    }
    pub fn light_index(self) -> u32 {
        self.0 & 511
    }
    pub fn next(self) -> u32 {
        self.0 >> 9
    }
}

/// PC NextLightData/HasLightData state, preserving native linked-list order.
/// Native consumers pass max_light_count=32; the caller supplies it explicitly.
pub fn visit_light_list(
    pool: &[PackedLightNode],
    mut head: u32,
    max_light_count: u32,
    mut visit: impl FnMut(u32),
) -> Result<u32, String> {
    let mut count = 0;
    while count < max_light_count && head != END_OF_LIGHT_LIST {
        let node = *pool
            .get(head as usize)
            .ok_or_else(|| format!("invalid light-list node {head}"))?;
        visit(node.light_index());
        head = node.next();
        count += 1;
    }
    Ok(count)
}

/// Scalar reference for tlf/gatherdepthminmax's reduction, including sky and
/// the original signed-float ULP operations. Inputs are the gathered depths
/// from both original depth surfaces, not a merged nearest-depth substitute.
pub fn reduce_depth_tile(depths: &[f32], unproject: [f32; 2]) -> [f32; 4] {
    let mut near = 0.0_f32;
    let mut far_sky = 1.0_f32;
    let mut far = 1.0_f32;
    for &depth in depths {
        near = near.max(depth);
        far_sky = far_sky.min(depth);
        far = far.min(depth.max(if depth <= 0.0 { 1.0 } else { 0.0 }));
    }
    let view_z = |depth: f32| -unproject[0] / (depth + unproject[1]);
    let near_z = f32::from_bits(view_z(near).to_bits().wrapping_sub(1));
    let sky_z = view_z(far_sky);
    let far_z = f32::from_bits(view_z(far).to_bits().wrapping_add(1));
    let a = 1.0 / (far_z - near_z);
    let b = -near_z * a;
    let mut mask = 0_u32;
    for &depth in depths {
        if depth != 0.0 {
            let bin =
                (view_z(depth).mul_add(a, b).clamp(0.0, 1.0) * DEPTH_BIN_SCALE).trunc() as u32;
            mask |= 1_u32.wrapping_shl(bin);
        }
    }
    [-near_z, -sky_z, -far_z, f32::from_bits(mask)]
}

/// Native IsInside uses inclusive endpoint bins and ignores empty sky bins.
pub fn intersects_depth_bins(a: f32, b: f32, mask: u32, light_near_far: [f32; 2]) -> bool {
    let f = light_near_far.map(|z| z.mul_add(a, b));
    if !(f[0] <= 1.0 && f[1] >= 0.0) {
        return false;
    }
    let bits =
        f.map(|depth| 1_u32.wrapping_shl((depth.clamp(0.0, 1.0) * DEPTH_BIN_SCALE).trunc() as u32));
    let light_mask = (bits[1].wrapping_sub(1) | bits[1]) & !bits[0].wrapping_sub(1);
    mask & light_mask != 0
}

/// Numerical inputs and independently executed original-HLSL GPU output.
#[cfg(test)]
pub(crate) fn depth_case(id: usize) -> (Vec<f32>, [f32; 2], [u32; 4]) {
    let depths = (0..512)
        .map(|pixel| {
            if id == 0 {
                return 0.0;
            }
            let mut d = ((pixel * 37 + id * 53) % 1024) as f32 / 1024.0;
            if id % 7 == 0 && pixel % 11 == 0 {
                d = 0.0;
            }
            if id % 5 == 0 {
                d = if pixel < 256 { 0.5 } else { 0.0 };
            }
            d
        })
        .collect();
    let line = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tlf_depth_cases.txt"
    ))
    .lines()
    .nth(id)
    .unwrap();
    let words: Vec<u32> = line
        .split_whitespace()
        .map(|v| u32::from_str_radix(v, 16).unwrap())
        .collect();
    (
        depths,
        [-0.1, (id % 3) as f32 * 0.00001],
        words.try_into().unwrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cull_packet_rounding_preserves_order_and_padding_fields() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        for count in [0_usize, 1, 63, 64, 65, 156, 192, 447] {
            let lighting = (0..count)
                .map(|i| {
                    let mut r = [0; 28];
                    r[0] = (i as f32).to_bits();
                    r[7] = 1.0_f32.to_bits();
                    r[25] = i as u32;
                    r
                })
                .collect::<Vec<_>>();
            let mut packets = [[0xa5a5_1234; 12]; LIGHT_BUFFER_CAPACITY];
            let padded = prepare_light_cull_packets(&identity, &lighting, &mut packets).unwrap();
            assert_eq!(padded, count.div_ceil(64) * 64);
            for i in 0..count {
                assert_eq!(packets[i][0], (i as f32).to_bits());
                assert_eq!(packets[i][11], 0xa5a5_0000 | i as u32);
            }
            for packet in &packets[count..padded] {
                assert_eq!(
                    *packet,
                    [
                        0x7f7f_ffff,
                        0x7f7f_ffff,
                        0x7f7f_ffff,
                        0xbf80_0000,
                        0,
                        0,
                        0x3f80_0000,
                        0,
                        0,
                        0,
                        0,
                        0xa5a5_ff20
                    ]
                );
            }
            assert!(packets[padded..]
                .iter()
                .all(|packet| *packet == [0xa5a5_1234; 12]));
        }
        let mut packets = [[0; 12]; LIGHT_BUFFER_CAPACITY];
        assert!(prepare_light_cull_packets(
            &identity,
            &[[0; 28]; LIGHT_BUFFER_CAPACITY],
            &mut packets
        )
        .is_err());
        assert!(prepare_light_cull_packets(
            &identity,
            &[[0; 28]; LIGHT_BUFFER_CAPACITY + 1],
            &mut packets
        )
        .is_err());
        assert!(prepare_light_cull_packets(&identity, &[[0; 28]; 65], &mut packets[..64]).is_err());
    }

    #[test]
    #[ignore = "requires artifacts/native-tlf-current-packets.json converted with the live probe"]
    fn cull_packets_match_completed_frame() {
        let fixture =
            std::fs::read_to_string("artifacts/native-tlf-live-packet-fixture.txt").unwrap();
        let mut rows = fixture.lines().map(|line| {
            line.split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>()
        });
        let header = rows.next().unwrap();
        assert_eq!(header.len(), 14);
        let count = header[0] as usize;
        let padded = header[1] as usize;
        let view = std::array::from_fn(|row| {
            std::array::from_fn(|column| f32::from_bits(header[2 + row * 4 + column]))
        });
        let rows = rows.collect::<Vec<_>>();
        assert_eq!(rows.len(), padded);
        let lighting = rows[..count]
            .iter()
            .map(|row| row[..28].try_into().unwrap())
            .collect::<Vec<[u32; 28]>>();
        let mut packets = vec![[0_u32; 12]; padded];
        for (packet, row) in packets.iter_mut().zip(&rows) {
            packet[11] = row[39] & 0xffff_0000;
        }
        assert_eq!(
            prepare_light_cull_packets(&view, &lighting, &mut packets).unwrap(),
            padded
        );
        for (i, (packet, row)) in packets.iter().zip(rows).enumerate() {
            assert_eq!(
                packet.as_slice(),
                &row[28..],
                "frame light/padding index {i}"
            );
        }
    }

    #[test]
    fn cull_packet_copy_and_affine_transform_match_machine_code() {
        let mut checked = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tlf_packet_cases.txt"
        ))
        .lines()
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(words.len(), 70);
            let view = std::array::from_fn(|row| {
                std::array::from_fn(|col| f32::from_bits(words[row * 4 + col]))
            });
            let origin = std::array::from_fn(|i| f32::from_bits(words[12 + i]));
            let position = light_view_position(&view, origin);
            assert_eq!(
                position.map(f32::to_bits),
                words[15..18],
                "case {checked} transform"
            );
            let lighting = words[18..46].try_into().unwrap();
            let mut packet = words[46..58].try_into().unwrap();
            fill_light_cull_packet(position, &lighting, &mut packet);
            assert_eq!(packet, words[58..70], "case {checked} packet");
            checked += 1;
        }
        assert_eq!(checked, 512);
    }

    #[test]
    fn gathered_light_filter_preserves_mode_and_type_rules() {
        assert!(includes_local_light(0, 1, 0, 1));
        assert!(includes_local_light(0, 2, 3, 1));
        assert!(!includes_local_light(0, 0, 0, 1));
        assert!(!includes_local_light(0, 4, 0, 1));
        assert!(!includes_local_light(0, 3, 4, 1));
        assert!(!includes_local_light(0, 3, 0, 0));
        assert!(!includes_local_light(0x10, 1, 0, 1));
        assert!(includes_local_light(0x10, 2, 0, 1));
        assert!(includes_local_light(0x20, 1, 0, 1));
        assert!(!includes_local_light(0x20, 2, 0, 1));
        assert!(!includes_local_light(0x30, 3, 0, 1));
        assert!(includes_local_light(0, 1, -1, 1)); // The original comparison is signed.
    }

    #[test]
    fn renderer_light_buffers_match_initializer_and_capture() {
        assert_eq!(LIGHT_BUFFER_CAPACITY * 48, 21504);
        assert_eq!(
            LIGHT_NODE_POOL_CAPACITY * std::mem::size_of::<u32>(),
            4608000
        );
        assert_eq!(SHADOW_BUFFER_CAPACITY, 256);
        assert_eq!(LIGHT_NODE_COUNTER_BYTES, 4);
    }

    #[test]
    fn default_tile_addresses_and_non_power_two_exponent() {
        let grid = TileGrid::new(1920, 1080, DEFAULT_TILE_DIMENSION).unwrap();
        assert_eq!(grid.dimensions, [120, 68]);
        assert_eq!(grid.lighting_shift, 4);
        assert_eq!(grid.light_head_coordinate([1919, 1079], 3), [479, 67]);
        assert_eq!(TileGrid::new(1919, 1079, 24).unwrap().lighting_shift, 4);
        assert!(TileGrid::new(0, 1080, 16).is_err());
        assert!(TileGrid::new(1920, 1080, 64).is_err());
    }

    #[test]
    fn depth_reduction_matches_hlsl() {
        for id in 0..64 {
            let (depths, params, words) = depth_case(id);
            let result = reduce_depth_tile(&depths, params);
            // FXC/D3D11 div differs from scalar division by one ULP. For a
            // constant-depth tile that changes its zero-width bin location;
            // exact shader masks are checked against all 64 cases on the GPU.
            for i in 0..3 {
                assert!(
                    result[i].to_bits().abs_diff(words[i]) <= 1,
                    "original tile {id} lane {i}"
                );
            }
            if id % 5 != 0 {
                assert_eq!(result[3].to_bits(), words[3], "original tile {id} mask");
            } else {
                assert_eq!(
                    result[3].to_bits().count_ones(),
                    words[3].count_ones(),
                    "original tile {id} constant-depth population"
                );
            }
        }
    }

    #[test]
    fn depth_mask_retains_sky_but_excludes_it_from_bins() {
        let p = [-0.1, 0.00001];
        let tile = reduce_depth_tile(&[1.0, 0.0, 0.5, 0.25], p);
        assert_eq!(
            tile[0].to_bits(),
            (-f32::from_bits((-p[0] / (1.0 + p[1])).to_bits() - 1)).to_bits()
        );
        assert_eq!(tile[1], -10000.0);
        assert_eq!(
            tile[2].to_bits(),
            (-f32::from_bits((-p[0] / (0.25 + p[1])).to_bits() + 1)).to_bits()
        );
        let a = 1.0 / (tile[0] - tile[2]);
        let b = tile[0] * a;
        assert!(intersects_depth_bins(
            a,
            b,
            tile[3].to_bits(),
            [-tile[0], -tile[2]]
        ));
        assert_eq!(reduce_depth_tile(&[0.0; 16], p)[3].to_bits(), 0);
        for bin in 0..32 {
            let depth = bin as f32 / 31.0;
            assert!(intersects_depth_bins(1.0, 0.0, 1 << bin, [depth, depth]));
        }
        assert!(!intersects_depth_bins(1.0, 0.0, u32::MAX, [1.1, 2.0]));
        assert!(!intersects_depth_bins(1.0, 0.0, u32::MAX, [-2.0, -0.1]));
        assert!(intersects_depth_bins(1.0, 0.0, 1 << 16, [0.501, 0.501]));
        assert!(!intersects_depth_bins(1.0, 0.0, 1 << 15, [0.501, 0.501]));
    }

    #[test]
    fn pc_light_links_and_iteration_limit() {
        for index in 0..512 {
            let node = PackedLightNode::new(END_OF_LIGHT_LIST, index);
            assert_eq!(node.light_index(), index);
            assert_eq!(node.next(), END_OF_LIGHT_LIST);
        }
        let pool = [
            PackedLightNode::new(END_OF_LIGHT_LIST, 7),
            PackedLightNode::new(0, 5),
        ];
        let mut found = Vec::new();
        assert_eq!(
            visit_light_list(&pool, 1, 32, |index| found.push(index)).unwrap(),
            2
        );
        assert_eq!(found, [5, 7]);
        assert_eq!(visit_light_list(&pool, 1, 1, |_| {}).unwrap(), 1);
        assert_eq!(
            visit_light_list(&pool, END_OF_LIGHT_LIST, 32, |_| {}).unwrap(),
            0
        );
        assert!(visit_light_list(&pool, 2, 32, |_| {}).is_err());
    }
}
