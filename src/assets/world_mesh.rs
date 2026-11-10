//! Static-world BWM geometry. Other BWM sections (materials and instances) follow later.

/// FUN_1402d1a00: first semantic/index match; missing uses UINT32_MAX.
/// A validated slice represents the native packed low-24-bit element count.
pub fn find_vertex_element(elements: &[[u8; 8]], semantic: u8, index: u8) -> u32 {
    elements
        .iter()
        .position(|e| e[4] == semantic && e[5] == index)
        .map_or(u32::MAX, |i| i as u32)
}

// Live declaration 8, captured from initialized DAT_1432d55a0. Stream 0
// contains positions; stream 1 contains both UVs, N/T and color (20 bytes).
const BWM_ELEMENTS: [[u8; 8]; 6] = [
    [0, 1, 7, 9, 0, 0, 0, 0],
    [0, 1, 7, 5, 4, 0, 0, 1],
    [0, 1, 7, 5, 4, 1, 4, 1],
    [0, 1, 4, 10, 1, 0, 8, 1],
    [0, 1, 4, 10, 2, 0, 12, 1],
    [0, 1, 5, 10, 5, 0, 16, 1],
];

#[derive(Debug)]
struct Geometry {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[half::f16; 2]>,
    // Declaration 8 stream 1 after UV0: UV1 half2, N/T SNORM8x4, colour UNORM8x4.
    channels: Vec<[u8; 16]>,
    indices: Vec<u16>,
}

#[derive(Debug)]
pub struct Mesh {
    geometry: std::sync::Arc<Geometry>,
    positions: std::ops::Range<usize>,
    attributes: std::ops::Range<usize>,
    indices: std::ops::Range<usize>,
}

impl Mesh {
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.geometry.positions[self.positions.clone()]
    }

    pub fn uvs(&self) -> &[[half::f16; 2]] {
        &self.geometry.uvs[self.attributes.clone()]
    }

    pub fn packed_channels(&self) -> &[[u8; 16]] {
        &self.geometry.channels[self.attributes.clone()]
    }

    pub fn indices(&self) -> &[u16] {
        &self.geometry.indices[self.indices.clone()]
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        positions: Vec<[f32; 3]>,
        uvs: Vec<[half::f16; 2]>,
        channels: Vec<[u8; 16]>,
        indices: Vec<u16>,
    ) -> Self {
        Self {
            positions: 0..positions.len(),
            attributes: 0..uvs.len(),
            indices: 0..indices.len(),
            geometry: std::sync::Arc::new(Geometry { positions, uvs, channels, indices }),
        }
    }
}

#[derive(Debug)]
pub struct WorldMesh {
    /// 295fa0's immutable receiver tree belongs to this loaded WORLD, shared
    /// by shadow and projected-decal consumers. Geometry release preserves it.
    pub(crate) spatial_query_cache: std::sync::OnceLock<std::sync::Arc<crate::local_caster_query::LocalWorldQuery>>,
    pub meshes: Vec<Mesh>,
    /// Row-major 3x4 affine transforms in game coordinates (Z up).
    pub matrices: Vec<[f32; 12]>,
    /// BWM v21 bounds and Umbra IDs, indexed alongside `matrices`.
    pub visibility: Vec<WorldVisibility>,
    /// Four BWM instance lists handled by the original renderer after Umbra.
    pub auxiliary_instance_groups: [Vec<usize>; 4],
    /// A visible parent suppresses these child BWM instances after the query.
    pub visibility_groups: Vec<WorldVisibilityGroup>,
    /// BWM v21 render-instance indices for each Tome object index.
    pub tome_object_instances: Vec<Vec<usize>>,
    /// Separate world-record list passed to the original spatial query path.
    pub spatial_query_instances: Vec<usize>,
    pub objects: Vec<WorldObject>,
}

/// Typed 20-byte descriptor assembled at FUN_1402996f0's gather boundary.
/// `end` is the cumulative end in the shared ID stream, not an instance count.
/// Native leaves the auxiliary offset undefined when flags48 bit6 is clear;
/// retaining that distinction avoids interpreting uninitialized native bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldShadowPacket {
    pub object: usize,
    pub start: u32,
    pub end: u32,
    pub auxiliary_start: Option<u16>,
    /// Native +0x1e is the `splineId` shader scalar, not a matrix-buffer
    /// selector or wind index. FUN_1402996f0 passes packet+14 to 310920 with
    /// DAT_14229a818; its parameter-name record at14229a808 is "splineId".
    /// Evidence: artifacts/ghidra-tool-output/738f61e7cb814816ef3126f6.json,
    /// 79ae630c5d8dc7287311754b.json. Keep the serialized value even for
    /// compiled non-spline programs that do not consume this uniform.
    pub visibility_field: u16,
    ///2996f0 reads native WORLD object+10 bit0: serialized flipped,
    ///not the separate serialized LOD at native object+c.
    pub flipped_key: u32,
}

/// Original DAT_1431af678..684 (all four words are zero). PE bytes and
/// Ghidra xrefs e74ea42ea0f4c16a414cce62 verify only reads/data references.
/// FUN_140183820 is the ordinary array-copy equivalent of this default.
/// FUN_1402996f0 appends TWO copies per wind-enabled shadow instance and
/// uploads them unchanged to world+0x188/+0x1c8. Forward color wind-manager
/// evaluation is not a dependency of this shadow gather.
pub const WORLD_SHADOW_AUXILIARY_DEFAULT: [u32; 4] = [0; 4];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorldShadowGather {
    pub instance_ids: Vec<u32>,
    pub auxiliary: Vec<[u32; 8]>,
    pub packets: Vec<WorldShadowPacket>,
}

impl WorldShadowGather {
    /// Original r_sortShadowsCCW defaults to 1 (FUN_1413647f0, string
    /// DAT_141abefb8="1"). FUN_1402996f0 sorts only the packet descriptors;
    /// selected-ID/auxiliary buffers and their cumulative offsets stay put.
    pub fn sort_for_shadow_draw(&mut self, sort_shadows_ccw: bool) {
        if sort_shadows_ccw {
            sort_shadow_packets(&mut self.packets);
        }
    }
}

/// FUN_1402ab630: descending signed key at packet+0x10, midpoint-to-end
/// pivot, native equality swaps, selection-sort leaves <=4 and 128-entry
/// bounded work stack. std sort would not preserve equal-key permutations.
fn sort_shadow_packets(packets: &mut [WorldShadowPacket]) {
    sort_shadow_records(packets, |a, b| {
        (a.flipped_key as i32).partial_cmp(&(b.flipped_key as i32))
    });
}

/// Shared native midpoint/end partition and short selection algorithm.
/// 2ab630 compares signed WORLD packet keys;3348e0 compares floating local
/// light priority differences. The caller preserves unordered comparisons.
pub(crate) fn sort_shadow_records<T>(
    packets: &mut [T],
    compare: impl Fn(&T, &T) -> Option<std::cmp::Ordering>,
) {
    use std::cmp::Ordering::{Equal, Greater, Less};
    if packets.len() < 2 {
        return;
    }
    let mut work = [(0usize, 0usize); 128];
    let mut depth = 0;
    let (mut low, mut high) = (0, packets.len() - 1);
    loop {
        if high - low > 3 && depth < 127 {
            packets.swap((low + high) / 2, high);
            let (mut left, mut right) = (low, high - 1);
            loop {
                while left < right && compare(&packets[left], &packets[high]) == Some(Greater) {
                    left += 1;
                }
                while right > left && compare(&packets[right], &packets[high]) == Some(Less) {
                    right -= 1;
                }
                if right <= left {
                    break;
                }
                packets.swap(left, right);
                left += 1;
                right -= 1;
                if left >= right {
                    break;
                }
            }
            while right > low
                && matches!(compare(&packets[right], &packets[high]), Some(Less | Equal))
            {
                right -= 1;
            }
            while left < high
                && matches!(
                    compare(&packets[left], &packets[high]),
                    Some(Greater | Equal)
                )
            {
                left += 1;
            }
            packets.swap(high, left);
            if right - low < high - left {
                work[depth] = (left, high);
                high = right;
            } else {
                work[depth] = (low, right);
                low = left;
            }
            depth += 1;
            continue;
        }
        while low < high {
            let mut smallest = low;
            for next in low + 1..=high {
                if compare(&packets[next], &packets[smallest]) == Some(Less) {
                    smallest = next;
                }
            }
            packets.swap(smallest, high);
            high -= 1;
        }
        if depth == 0 {
            break;
        }
        depth -= 1;
        (low, high) = work[depth];
    }
}

impl WorldMesh {
    pub fn spatial_query(&self) -> std::sync::Arc<crate::local_caster_query::LocalWorldQuery> {
        let create = || {
            let _tag = crate::platform::allocation_tags::tag("WORLD spatial queries");
            std::sync::Arc::new(crate::local_caster_query::LocalWorldQuery::new(self))
        };
        // Diagnostic acquisition A/B of the former per-consumer trees.
        if std::env::var("DH2_DEBUG_UNSHARED_WORLD_QUERIES").as_deref() == Ok("1") { return create(); }
        self.spatial_query_cache.get_or_init(create).clone()
    }
    /// FUN_140295e40: append the midpoint of every record belonging to a
    /// needsWindBuffers (material +0x48 bit6) object, in file/object order.
    /// Runtime +0x1c receives that appended signed-short index; serialized
    /// +0x1e is not changed. Overlapping ranges append again, just as native.
    /// `centers` is the real shared owner's existing list, not a zero-based
    /// replacement. Non-wind records retain their prior runtime indices.
    /// FUN_140294690's successful bounded append is represented by Vec::push;
    /// allocator growth/failure policy is outside this method's scope. Inputs
    /// which would turn the native signed-short address negative are rejected.
    pub fn prepare_wind_centers(
        &self,
        material_flags48: &[u8],
        centers: &mut Vec<[f32; 3]>,
        runtime_wind_indices: &mut [i16],
    ) -> Result<(), String> {
        if material_flags48.len() != self.objects.len()
            || runtime_wind_indices.len() != self.visibility.len()
        {
            return Err("world wind-center domain mismatch".into());
        }
        let mut count = 0usize;
        for (object, flags) in self.objects.iter().zip(material_flags48) {
            if object.matrix_start > object.matrix_end || object.matrix_end > self.visibility.len()
            {
                return Err(
                    "world wind-center object range is outside visibility records".into(),
                );
            }
            if flags & 0x40 != 0 {
                count = count
                    .checked_add(object.matrix_end - object.matrix_start)
                    .ok_or("world wind-center count overflow")?;
            }
        }
        if count != 0
            && centers
                .len()
                .checked_add(count)
                .is_none_or(|end| end > i16::MAX as usize + 1)
        {
            return Err(
                "world wind-center index exceeds signed-short address domain".into(),
            );
        }
        centers
            .try_reserve(count)
            .map_err(|error| format!("world wind-center allocation failed: {error}"))?;
        for (object, flags) in self.objects.iter().zip(material_flags48) {
            if flags & 0x40 == 0 {
                continue;
            }
            for index in object.matrix_start..object.matrix_end {
                let bounds = self.visibility[index].bounds;
                runtime_wind_indices[index] = centers.len() as i16;
                // DAT_141ab3764=0.5, independently checked by the executable
                // fixture. Do not reorder to low+(high-low)/2 or midpoint().
                centers.push(std::array::from_fn(|axis| {
                    (bounds[1][axis] + bounds[0][axis]) * 0.5
                }));
            }
        }
        Ok(())
    }

    /// The actual shadow gather's zero auxiliary values, NOT forward color
    /// wind-volume samples. Sorting is an explicit later original step.
    pub fn gather_shadow_packets(
        &self,
        mask: &[u8],
        material_flags48: &[u8],
    ) -> Result<WorldShadowGather, String> {
        self.gather_shadow_packets_with_auxiliary(mask, material_flags48, WORLD_SHADOW_AUXILIARY_DEFAULT)
    }

    /// FUN_1402996f0, 140299795..140299a09: gather in object/file order.
    /// Flags are the real resolved native material +0x48 bytes; a missing
    /// program/material is NOT permission to guess them from a file name.
    /// FUN_1402974c0 (chunk_000107.c:2532..2536) reads each 30-byte FILE
    /// record as 24+4+2 bytes into runtime offsets 0,+0x18,+0x1e, initializing
    /// the separate +0x1c wind-center index to -1. Thus trailing_field IS the
    /// native +0x1e packet word (`splineId`); FUN_140295e40 only overwrites
    /// +0x1c. Native also binds splineInfos at world+0x138 separately from
    /// matrices. Do not apply splineId as an extra affine-matrix index.
    /// `auxiliary_value` is the actual owner vector DAT_1431af678..684.
    /// The optional idSort_ShadowWorld, buffer uploads, material/program binds
    /// and indexed submission occur AFTER this boundary and are not replaced
    /// by sorting these packets or by the rigid-model shader.
    pub fn gather_shadow_packets_with_auxiliary(
        &self,
        mask: &[u8],
        material_flags48: &[u8],
        auxiliary_value: [u32; 4],
    ) -> Result<WorldShadowGather, String> {
        let count = self.visibility.len();
        if count > 0x00ff_ffff
            || mask.len() != count
            || material_flags48.len() != self.objects.len()
        {
            return Err("world shadow gather domain mismatch".into());
        }
        // Validate before starting: native objects always reference their first
        // visibility record, even when that object's selection turns out empty.
        if self.objects.iter().any(|o| {
            o.matrix_start >= count || o.matrix_start > o.matrix_end || o.matrix_end > count
        }) {
            return Err("world shadow object range is outside visibility records".into());
        }
        let mut result = WorldShadowGather::default();
        for (index, (object, &flags)) in self.objects.iter().zip(material_flags48).enumerate() {
            if flags & 1 == 0 {
                continue;
            }
            let start = result.instance_ids.len();
            let auxiliary_start = (flags & 0x40 != 0).then_some(result.auxiliary.len() as u16);
            let visibility_field = self.visibility[object.matrix_start].trailing_field;
            for instance in object.matrix_start..object.matrix_end {
                if mask[instance] == 0 {
                    continue;
                }
                result.instance_ids.push(instance as u32);
                if flags & 0x40 != 0 {
                    result
                        .auxiliary
                        .push(std::array::from_fn(|i| auxiliary_value[i & 3]));
                }
            }
            let end = result.instance_ids.len();
            if end != start {
                // These are packed low-24-bit native array lengths. Malformed
                // overlapping source ranges must not silently wrap GPU offsets.
                if end > 0x00ff_ffff || result.auxiliary.len() > 0x00ff_ffff {
                    return Err("world shadow gather exceeds packed count domain".into());
                }
                result.packets.push(WorldShadowPacket {
                    object: index,
                    start: start as u32,
                    end: end as u32,
                    auxiliary_start,
                    visibility_field,
                    flipped_key: u32::from(object.flipped),
                });
            }
        }
        Ok(result)
    }

    /// Root box of FUN_140295fa0 -> FUN_140293580 -> FUN_140293800.
    /// The input is EVERY serialized 30-byte visibility record (native stride32), in file order,
    /// not current color-visible geometry, mesh vertices or the Tome header.
    /// Native world+0x400 points to this tree's root; FUN_14035f5b0 reads its
    /// +0x10/+0x20 vectors for the auxiliary whole-view shadow projection.
    /// Only this root reduction is represented here, not the tree's partition,
    /// leaf index allocation or spatial traversal.
    pub fn static_query_bounds(&self) -> Option<[[f32; 3]; 2]> {
        static_query_bounds(self.visibility.iter().map(|record| &record.bounds))
    }

    /// FUN_14032de00's masks passed synchronously to FUN_1402996f0.
    /// Domain: FUN_140295e10's packed visibility/transform count, NOT meshes.
    /// Native +0x228/+0x230 are objects; +0x3e0/+0x3e8 are group parents.
    /// This selects world records, not material programs or final draw packets.
    pub fn for_each_whole_view_shadow_batch(
        &self,
        mut submit: impl FnMut(&[u8]),
    ) -> Result<(), String> {
        let count = self.visibility.len();
        if count > 0x00ff_ffff {
            return Err("world shadow count exceeds packed 24-bit domain".into());
        }
        if self
            .objects
            .iter()
            .any(|object| object.matrix_start > object.matrix_end || object.matrix_end > count)
            || self
                .visibility_groups
                .iter()
                .any(|group| group.parent >= count)
        {
            return Err("world shadow range/parent exceeds visibility domain".into());
        }
        // For a 24-bit count, native ceil(float(count)/4096) is exactly this.
        let batches = count.div_ceil(4096);
        let exclusions: Vec<usize> = self
            .visibility_groups
            .iter()
            .map(|group| group.parent)
            .collect();
        let mut mask = vec![0u8; count];
        for batch in 0..batches {
            mask.fill(0);
            let start = batch * 4096;
            mask[start..count.min(start + 4096)].fill(1);
            // Do NOT replace this overlap window with the 4096-byte mask
            // interval. Native R15D is the ceil result; RBX advances by R15D
            // (14032e04d..14032e08d,14032e0e0,14032e19c..14032e1a2).
            let window = batch * batches;
            for object in &self.objects {
                if object.lod & 1 != 0
                    && window < object.matrix_end
                    && object.matrix_start < window + batches
                {
                    mask[object.matrix_start..object.matrix_end].fill(0);
                }
            }
            for &parent in &exclusions {
                mask[parent] = 0;
            }
            submit(&mask);
        }
        Ok(())
    }
}

/// MINPS/MAXPS choose the incoming operand for equality/unordered inputs.
/// Keep serialized order and signed zero; f32::min/max have different rules.
fn static_query_bounds<'a>(
    bounds: impl IntoIterator<Item = &'a [[f32; 3]; 2]>,
) -> Option<[[f32; 3]; 2]> {
    let mut result = [
        [f32::from_bits(0x7149_f2ca); 3], // DAT_141ae8a70
        [f32::from_bits(0xf149_f2ca); 3], // DAT_141ae8a74
    ];
    let mut present = false;
    for bounds in bounds {
        present = true;
        for axis in 0..3 {
            if !(result[0][axis] < bounds[0][axis]) {
                result[0][axis] = bounds[0][axis];
            }
            if !(result[1][axis] > bounds[1][axis]) {
                result[1][axis] = bounds[1][axis];
            }
        }
    }
    present.then_some(result)
}

#[derive(Debug)]
pub struct WorldVisibility {
    pub bounds: [[f32; 3]; 2],
    pub umbra_id: u32,
    /// File +28, copied by 1402974c0/140296240 into runtime +0x1e.
    /// Runtime +0x1c is a different, nonserialized wind-center index.
    pub trailing_field: u16,
}

#[derive(Debug)]
pub struct WorldVisibilityGroup {
    pub parent: usize,
    pub children: Vec<usize>,
}

#[derive(Debug)]
pub struct WorldObject {
    pub mesh_index: usize,
    pub material_path: String,
    pub matrix_start: usize,
    pub matrix_end: usize,
    pub flipped: bool,
    pub lod: usize,
}

fn region(bytes: &[u8], start: usize, size: usize) -> Result<&[u8], String> {
    bytes
        .get(start..start.checked_add(size).ok_or("offset overflow")?)
        .ok_or_else(|| format!("section at {start} of {size} bytes exceeds file"))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    let data: [u8; 2] = region(bytes, at, 2)?.try_into().unwrap();
    Ok(u16::from_le_bytes(data))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    let data: [u8; 4] = region(bytes, at, 4)?.try_into().unwrap();
    Ok(u32::from_le_bytes(data))
}

fn usize_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    usize::try_from(u32_at(bytes, at)?).map_err(|_| "size does not fit usize".into())
}

fn counted_indices(bytes: &[u8], at: &mut usize, limit: usize) -> Result<Vec<usize>, String> {
    let count = usize_at(bytes, *at)?;
    *at = at.checked_add(4).ok_or("offset overflow")?;
    let data = region(
        bytes,
        *at,
        count.checked_mul(4).ok_or("index size overflow")?,
    )?;
    let mut indices = Vec::with_capacity(count);
    for word in data.chunks_exact(4) {
        let index = u32::from_le_bytes(word.try_into().unwrap()) as usize;
        if index >= limit {
            return Err(format!("world visibility index {index} exceeds {limit}"));
        }
        indices.push(index);
    }
    *at += data.len();
    Ok(indices)
}

fn section_size(input: &mut dyn std::io::Read, remaining: &mut usize) -> Result<usize, String> {
    let mut size = [0; 4];
    input.read_exact(&mut size).map_err(|error| error.to_string())?;
    let size = u32::from_le_bytes(size) as usize;
    *remaining = remaining.checked_sub(4).and_then(|left| left.checked_sub(size))
        .ok_or("BWM section exceeds its member")?;
    Ok(size)
}

fn read_records<const STRIDE: usize>(
    input: &mut dyn std::io::Read,
    mut size: usize,
    cancelled: &impl Fn() -> bool,
    mut decode: impl FnMut(&[u8]),
) -> Result<(), String> {
    if size % STRIDE != 0 {
        return Err("misaligned world geometry section".into());
    }
    // A bounded buffer shared by every record in this section. All current
    // BWM record strides divide this extent.
    let mut scratch = [0u8; 61_440];
    while size > 0 {
        if cancelled() {
            return Err("world geometry acquisition cancelled".into());
        }
        let count = size.min(scratch.len());
        let chunk = &mut scratch[..count];
        input.read_exact(chunk).map_err(|error| error.to_string())?;
        for record in chunk.chunks_exact(STRIDE) {
            decode(record);
        }
        size -= count;
    }
    Ok(())
}

fn mesh_range(offset: usize, count: usize, stride: usize, len: usize) -> Result<std::ops::Range<usize>, String> {
    if offset % stride != 0 {
        return Err("misaligned BWM mesh offset".into());
    }
    let start = offset / stride;
    let end = start.checked_add(count).filter(|&end| end <= len)
        .ok_or("BWM mesh range exceeds its section")?;
    Ok(start..end)
}

pub fn parse(bytes: &[u8]) -> Result<WorldMesh, String> {
    parse_stream(&mut std::io::Cursor::new(bytes), bytes.len(), &|| false)
}

/// Decode the installed member directly into shared geometry and map records.
pub fn load(
    catalog: &crate::archive::Catalog,
    path: &str,
    cancelled: impl Fn() -> bool,
) -> Result<WorldMesh, String> {
    let world = if std::env::var_os("DH2_DEBUG_BUFFER_WORLD_MEMBER").is_some_and(|v| v == "1") {
        let bytes = {
            let _tag = crate::platform::allocation_tags::tag("BWM whole decoded member");
            catalog.read(path).map_err(|error| error.to_string())?
        };
        bytes.as_deref().map(|bytes| {
            parse_stream(&mut std::io::Cursor::new(bytes), bytes.len(), &cancelled)
        }).transpose()?
    } else {
        catalog.with_stream(path, |input, size| parse_stream(input, size, &cancelled))?
    };
    world.ok_or_else(|| format!("missing indexed world {path}"))
}

fn parse_stream(
    input: &mut dyn std::io::Read,
    size: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<WorldMesh, String> {
    let mut header = [0; 12];
    input.read_exact(&mut header).map_err(|error| error.to_string())?;
    if &header[..4] != b"BWM1" {
        return Err("not a BWM1 file".into());
    }
    if u16_at(&header, 4)? != 21 || u16_at(&header, 6)? != 1 || u32_at(&header, 8)? != 2 {
        return Err("unsupported BWM version".into());
    }
    let mut remaining = size.checked_sub(header.len()).ok_or("BWM header exceeds its member")?;

    let position_bytes = section_size(input, &mut remaining)?;
    let mut positions = Vec::with_capacity(position_bytes / 12);
    read_records::<12>(input, position_bytes, cancelled, |record| {
        positions.push(std::array::from_fn(|axis| {
            f32::from_le_bytes(record[axis * 4..axis * 4 + 4].try_into().unwrap())
        }));
    })?;

    let attribute_bytes = section_size(input, &mut remaining)?;
    let mut uvs = Vec::with_capacity(attribute_bytes / 20);
    let mut channels = Vec::with_capacity(attribute_bytes / 20);
    let channel_offsets = [(4, 1), (1, 0), (2, 0), (5, 0)].map(|(semantic, index)| {
        BWM_ELEMENTS[find_vertex_element(&BWM_ELEMENTS, semantic, index) as usize][6] as usize
    });
    read_records::<20>(input, attribute_bytes, cancelled, |record| {
        uvs.push([
            half::f16::from_le_bytes(record[0..2].try_into().unwrap()),
            half::f16::from_le_bytes(record[2..4].try_into().unwrap()),
        ]);
        let mut packed = [0; 16];
        for (slot, offset) in channel_offsets.into_iter().enumerate() {
            packed[slot * 4..slot * 4 + 4].copy_from_slice(&record[offset..offset + 4]);
        }
        channels.push(packed);
    })?;

    let index_bytes = section_size(input, &mut remaining)?;
    let mut indices = Vec::with_capacity(index_bytes / 2);
    read_records::<2>(input, index_bytes, cancelled, |record| {
        indices.push(u16::from_le_bytes(record.try_into().unwrap()));
    })?;

    let matrix_bytes = section_size(input, &mut remaining)?;
    let mut decoded_matrices = Vec::with_capacity(matrix_bytes / 48);
    read_records::<48>(input, matrix_bytes, cancelled, |record| {
        decoded_matrices.push(std::array::from_fn(|axis| {
            f32::from_le_bytes(record[axis * 4..axis * 4 + 4].try_into().unwrap())
        }));
    })?;
    if decoded_matrices.iter().flatten().any(|value| !value.is_finite()) {
        return Err("non-finite world transform".into());
    }

    let other = section_size(input, &mut remaining)? as u64;
    let skipped = std::io::copy(&mut std::io::Read::take(&mut *input, other), &mut std::io::sink())
        .map_err(|error| error.to_string())?;
    if skipped != other {
        return Err("truncated BWM auxiliary section".into());
    }
    if cancelled() {
        return Err("world geometry acquisition cancelled".into());
    }
    let mut tail = vec![0; remaining];
    input.read_exact(&mut tail).map_err(|error| error.to_string())?;
    let bytes = &tail;

    let geometry = std::sync::Arc::new(Geometry { positions, uvs, channels, indices });
    let count = usize_at(bytes, 0)?;
    if count > 100_000 {
        return Err("unreasonable mesh count".into());
    }
    let mut at = 4;
    let mut meshes = Vec::with_capacity(count);
    for mesh_number in 0..count {
        let record = region(bytes, at, 41)?;
        at += 41;
        let u32_field = |offset| usize_at(record, offset);
        if u32_field(0)? != 2
            || u32_field(8)? != 12
            || u32_field(16)? != 20
            || u32_field(20)? != 0x1234_5678
            || u32_field(36)? != 3
            || record[40] != 1
        {
            return Err(format!("mesh {mesh_number} has an unsupported layout"));
        }
        let vertex_count = u32_field(32)?;
        let index_count = u32_field(28)?;
        if index_count % 3 != 0 {
            return Err(format!("mesh {mesh_number} has incomplete triangles"));
        }
        let mesh = Mesh {
            positions: mesh_range(u32_field(4)?, vertex_count, 12, geometry.positions.len())?,
            attributes: mesh_range(u32_field(12)?, vertex_count, 20, geometry.uvs.len())?,
            indices: mesh_range(u32_field(24)?, index_count, 2, geometry.indices.len())?,
            geometry: geometry.clone(),
        };
        if mesh.positions().iter().flatten().any(|value| !value.is_finite()) {
            return Err(format!("mesh {mesh_number} contains non-finite coordinates"));
        }
        if mesh.uvs().iter().flatten().any(|value| !value.is_finite()) {
            return Err(format!("mesh {mesh_number} has non-finite UVs"));
        }
        if let Some(&index) = mesh.indices().iter().find(|&&index| index as usize >= vertex_count) {
            return Err(format!("mesh {mesh_number} references vertex {index} of {vertex_count}"));
        }
        meshes.push(mesh);
    }
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
        eprintln!(
            "MEMORY BWM sections: source_bytes={size} geometry_bytes={} record_tail_bytes={remaining} mesh_ranges={count}",
            position_bytes + attribute_bytes + index_bytes,
        );
    }
    let object_count = usize_at(bytes, at)?;
    if object_count > 100_000 {
        return Err("unreasonable object count".into());
    }
    at += 4;
    let mut objects = Vec::with_capacity(object_count);
    for object_number in 0..object_count {
        let matrix_start = usize_at(bytes, at)?;
        let matrix_end = usize_at(bytes, at + 4)?;
        let mesh_index = usize_at(bytes, at + 8)?;
        if matrix_start > matrix_end
            || matrix_end > decoded_matrices.len()
            || mesh_index >= meshes.len()
        {
            return Err(format!(
                "object {object_number} references missing matrix or mesh"
            ));
        }
        let flipped = *region(bytes, at + 12, 1)?.first().unwrap();
        if flipped > 1 {
            return Err(format!("object {object_number} has invalid flip flag"));
        }
        let path_length = usize_at(bytes, at + 13)?;
        if path_length > 4096 {
            return Err(format!(
                "object {object_number} has unreasonable material path"
            ));
        }
        let material_path = std::str::from_utf8(region(bytes, at + 17, path_length)?)
            .map_err(|_| format!("object {object_number} material path is not UTF-8"))?
            .to_string();
        let lod = usize_at(bytes, at + 17 + path_length)?;
        if lod > 100 {
            return Err(format!("object {object_number} has unreasonable LOD"));
        }
        at += 21 + path_length;
        objects.push(WorldObject {
            mesh_index,
            material_path,
            matrix_start,
            matrix_end,
            flipped: flipped != 0,
            lod,
        });
    }
    // FUN_1402974c0 reads this BWM v21 section into the world's +0x390
    // array: 24 bytes of bounds, a 32-bit Umbra ID, and a 16-bit field.
    let visibility_count = usize_at(bytes, at)?;
    at += 4;
    if visibility_count != decoded_matrices.len() {
        return Err(format!(
            "{visibility_count} visibility records for {} transforms",
            decoded_matrices.len()
        ));
    }
    let records = region(
        bytes,
        at,
        visibility_count
            .checked_mul(30)
            .ok_or("visibility size overflow")?,
    )?;
    let mut visibility = Vec::with_capacity(visibility_count);
    for (number, record) in records.chunks_exact(30).enumerate() {
        let mut bounds = [[0.0; 3]; 2];
        for (index, value) in bounds.iter_mut().flatten().enumerate() {
            *value = f32::from_le_bytes(record[index * 4..index * 4 + 4].try_into().unwrap());
            if !value.is_finite() {
                return Err(format!("visibility record {number} has non-finite bounds"));
            }
        }
        let umbra_id = u32::from_le_bytes(record[24..28].try_into().unwrap());
        let trailing_field = u16::from_le_bytes(record[28..30].try_into().unwrap());
        visibility.push(WorldVisibility {
            bounds,
            umbra_id,
            trailing_field,
        });
    }
    at += records.len();
    // FUN_1402974c0 then reads four auxiliary lists, variable-length group
    // records, Tome-indexed instance lists at world +0x408, and a separate
    // world-record list at +0x418.
    let aux_counts = (0..4)
        .map(|offset| usize_at(bytes, at + offset * 4))
        .collect::<Result<Vec<_>, _>>()?;
    at += 16;
    let mut auxiliary_instance_groups = std::array::from_fn(|_| Vec::new());
    for (group, count) in aux_counts.into_iter().enumerate() {
        let size = count.checked_mul(4).ok_or("auxiliary list size overflow")?;
        for word in region(bytes, at, size)?.chunks_exact(4) {
            let index = u32::from_le_bytes(word.try_into().unwrap()) as usize;
            if index >= visibility_count {
                return Err(format!(
                    "auxiliary visibility index {index} exceeds {visibility_count}"
                ));
            }
            auxiliary_instance_groups[group].push(index);
        }
        at += size;
    }
    let grouped_count = usize_at(bytes, at)?;
    at += 4;
    let mut visibility_groups = Vec::with_capacity(grouped_count);
    for _ in 0..grouped_count {
        let parent = usize_at(bytes, at)?;
        if parent >= visibility_count {
            return Err(format!(
                "visibility group parent {parent} exceeds {visibility_count}"
            ));
        }
        let count = usize_at(bytes, at + 4)?;
        at += 8;
        let size = count.checked_mul(4).ok_or("grouped list size overflow")?;
        let mut children = Vec::with_capacity(count);
        for word in region(bytes, at, size)?.chunks_exact(4) {
            let index = u32::from_le_bytes(word.try_into().unwrap()) as usize;
            if index >= visibility_count {
                return Err(format!(
                    "visibility group child {index} exceeds {visibility_count}"
                ));
            }
            children.push(index);
        }
        visibility_groups.push(WorldVisibilityGroup { parent, children });
        at += size;
    }
    let tome_count = usize_at(bytes, at)?;
    at += 4;
    if tome_count > bytes.len().saturating_sub(at) / 4 {
        return Err("Tome mapping count exceeds BWM length".into());
    }
    let mut tome_object_instances = Vec::with_capacity(tome_count);
    for _ in 0..tome_count {
        tome_object_instances.push(counted_indices(bytes, &mut at, visibility_count)?);
    }
    let spatial_query_instances = counted_indices(bytes, &mut at, visibility_count)?;
    if at != bytes.len() {
        return Err(format!("{} unparsed BWM bytes", bytes.len() - at));
    }
    Ok(WorldMesh {
        spatial_query_cache: Default::default(),
        meshes,
        matrices: decoded_matrices,
        visibility,
        tome_object_instances,
        auxiliary_instance_groups,
        visibility_groups,
        spatial_query_instances,
        objects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shadow_mask_world(count: usize) -> WorldMesh {
        WorldMesh {
            spatial_query_cache: Default::default(),
            meshes: Vec::new(),
            matrices: Vec::new(),
            objects: Vec::new(),
            visibility: (0..count)
                .map(|index| WorldVisibility {
                    bounds: [[0.0; 3]; 2],
                    umbra_id: index as u32,
                    trailing_field: 0,
                })
                .collect(),
            auxiliary_instance_groups: Default::default(),
            visibility_groups: Vec::new(),
            tome_object_instances: Vec::new(),
            spatial_query_instances: Vec::new(),
        }
    }

    fn shadow_object(start: usize, end: usize, lod: usize) -> WorldObject {
        WorldObject {
            mesh_index: 0,
            material_path: String::new(),
            matrix_start: start,
            matrix_end: end,
            flipped: lod & 1 != 0,
            lod,
        }
    }

    #[test]
    fn shadow_packet_sort_matches_executable_including_ties() {
        let mut cases = 0;
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/world_shadow_sort_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let count = words.next().unwrap() as usize;
            let mut read_packets = || {
                (0..count)
                    .map(|_| {
                        let object = words.next().unwrap() as usize;
                        let start = words.next().unwrap();
                        let end = words.next().unwrap();
                        let pair = words.next().unwrap();
                        let flipped_key = words.next().unwrap();
                        WorldShadowPacket {
                            object,
                            start,
                            end,
                            auxiliary_start: Some(pair as u16),
                            visibility_field: (pair >> 16) as u16,
                            flipped_key,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let input = read_packets();
            let expected = read_packets();
            assert!(words.next().is_none());
            let mut gather = WorldShadowGather {
                instance_ids: vec![7, 13, 29],
                auxiliary: vec![[0xdeadbeef; 8]],
                packets: input.clone(),
            };
            gather.sort_for_shadow_draw(false);
            assert_eq!(gather.packets, input);
            gather.sort_for_shadow_draw(true);
            assert_eq!(gather.packets, expected, "packet sort case{case}");
            assert_eq!(gather.instance_ids, [7, 13, 29]);
            assert_eq!(gather.auxiliary, [[0xdeadbeef; 8]]);
            cases += 1;
        }
        assert_eq!(cases, 384);
    }

    #[test]
    fn shadow_gather_uploads_zero_auxiliary_not_forward_wind_samples() {
        let mut world = shadow_mask_world(2);
        world.objects.push(shadow_object(0, 2, 0));
        let gathered = world
            .gather_shadow_packets(&[1, 1], &[0x41])
            .unwrap();
        assert_eq!(gathered.auxiliary, [[0; 8]; 2]);
        assert_eq!(gathered.packets[0].auxiliary_start, Some(0));
    }

    #[test]
    fn world_wind_centers_match_executable() {
        let mut cases = 0;
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/world_wind_center_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let mut next = || {
                words
                    .next()
                    .expect("truncated original wind-center fixture")
            };
            let count = next() as usize;
            let objects = next() as usize;
            let prefix = next() as usize;
            let flags: Vec<_> = (0..objects).map(|_| next() as u8).collect();
            let mut world = shadow_mask_world(count);
            for _ in 0..objects {
                let start = next() as usize;
                let end = next() as usize;
                let lod = next() as usize;
                world.objects.push(shadow_object(start, end, lod));
            }
            for record in &mut world.visibility {
                record.bounds =
                    std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(next())));
            }
            let mut centers: Vec<[f32; 3]> = (0..prefix)
                .map(|_| std::array::from_fn(|_| f32::from_bits(next())))
                .collect();
            let output_len = next() as usize;
            let expected_indices: Vec<_> = (0..count).map(|_| next() as u16).collect();
            let expected_centers: Vec<[u32; 3]> = (0..output_len)
                .map(|_| std::array::from_fn(|_| next()))
                .collect();
            assert!(words.next().is_none());
            let mut indices = vec![-1; count];
            world
                .prepare_wind_centers(&flags, &mut centers, &mut indices)
                .unwrap();
            assert_eq!(
                indices.iter().map(|&v| v as u16).collect::<Vec<_>>(),
                expected_indices,
                "indices case{case}"
            );
            assert_eq!(
                centers
                    .iter()
                    .map(|v| v.map(f32::to_bits))
                    .collect::<Vec<_>>(),
                expected_centers,
                "midpoint bits case{case}"
            );
            cases += 1;
        }
        assert_eq!(cases, 256);
    }

    #[test]
    fn world_wind_centers_reject_invalid_address_domain_before_mutation() {
        let mut world = shadow_mask_world(2);
        world.objects.push(shadow_object(0, 2, 0));
        let mut indices = [-1; 2];
        let mut centers = vec![[1.0, 2.0, 3.0]; i16::MAX as usize];
        assert!(world
            .prepare_wind_centers(&[0x40], &mut centers, &mut indices)
            .is_err());
        assert_eq!(indices, [-1; 2]);
        assert_eq!(centers.len(), i16::MAX as usize);
        world
            .prepare_wind_centers(&[0], &mut centers, &mut indices)
            .unwrap();
        assert_eq!(indices, [-1; 2]);
        world.objects[0].matrix_end = 3;
        assert!(world
            .prepare_wind_centers(&[0x40], &mut centers, &mut indices)
            .is_err());
        assert!(world
            .prepare_wind_centers(&[], &mut centers, &mut indices)
            .is_err());
    }

    #[test]
    fn shadow_packet_gather_matches_executable_prefix() {
        let mut cases = 0;
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/world_shadow_gather_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let mut words = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap());
            let mut next = || {
                words
                    .next()
                    .expect("truncated original world gather fixture")
            };
            let count = next() as usize;
            let objects = next() as usize;
            let auxiliary = std::array::from_fn(|_| next());
            let flags: Vec<_> = (0..objects).map(|_| next() as u8).collect();
            let mut world = shadow_mask_world(count);
            for _ in 0..objects {
                let start = next() as usize;
                let end = next() as usize;
                let flipped = next() & 1 != 0;
                let mut object = shadow_object(start, end, 0);
                object.flipped = flipped;
                world.objects.push(object);
            }
            for record in &mut world.visibility {
                record.trailing_field = next() as u16;
            }
            let mask: Vec<_> = (0..count).map(|_| next() as u8).collect();
            let lengths: [usize; 3] = std::array::from_fn(|_| next() as usize);
            let expected_ids: Vec<_> = (0..lengths[0]).map(|_| next()).collect();
            let expected_auxiliary: Vec<[u32; 8]> = (0..lengths[1])
                .map(|_| std::array::from_fn(|_| next()))
                .collect();
            let expected_packets: Vec<_> = (0..lengths[2])
                .map(|_| {
                    let object = next() as usize;
                    let start = next();
                    let end = next();
                    let auxiliary = next();
                    let visibility_field = next() as u16;
                    let flipped_key = next();
                    WorldShadowPacket {
                        object,
                        start,
                        end,
                        auxiliary_start: (auxiliary != u32::MAX).then_some(auxiliary as u16),
                        visibility_field,
                        flipped_key,
                    }
                })
                .collect();
            assert!(words.next().is_none(), "extra fixture words in case {case}");
            let expected = WorldShadowGather {
                instance_ids: expected_ids,
                auxiliary: expected_auxiliary,
                packets: expected_packets,
            };
            assert_eq!(
                world
                    .gather_shadow_packets_with_auxiliary(&mask, &flags, auxiliary)
                    .unwrap(),
                expected,
                "original executable case {case}"
            );
            cases += 1;
        }
        assert_eq!(cases, 256);
    }

    #[test]
    fn shadow_packets_keep_file_order_cumulative_offsets_and_first_visibility_word() {
        let mut world = shadow_mask_world(9);
        world.objects = vec![
            shadow_object(0, 3, 2),
            shadow_object(3, 5, 1),
            shadow_object(5, 7, 3),
            shadow_object(7, 9, 0),
        ];
        // The loader maps each serialized trailing word directly to runtime +0x1e.
        for (index, record) in world.visibility.iter_mut().enumerate() {
            record.trailing_field = 100 + index as u16;
        }
        let gathered = world
            .gather_shadow_packets_with_auxiliary(
                &[0, 2, 1, 1, 0, 0, 0, 1, 1],
                &[1, 0x41, 1, 0],
                [0x8000_0000, 0x7fc0_1234, 0x3f80_0000, 0xffff_ffff],
            )
            .unwrap();
        assert_eq!(gathered.instance_ids, [1, 2, 3]);
        assert_eq!(
            gathered.auxiliary,
            [[
                0x8000_0000,
                0x7fc0_1234,
                0x3f80_0000,
                0xffff_ffff,
                0x8000_0000,
                0x7fc0_1234,
                0x3f80_0000,
                0xffff_ffff,
            ]]
        );
        assert_eq!(
            gathered.packets,
            [
                WorldShadowPacket {
                    object: 0,
                    start: 0,
                    end: 2,
                    auxiliary_start: None,
                    visibility_field: 100,
                    flipped_key: 0
                },
                WorldShadowPacket {
                    object: 1,
                    start: 2,
                    end: 3,
                    auxiliary_start: Some(0),
                    visibility_field: 103,
                    flipped_key: 1
                },
            ]
        );
        // All-zero selections and flags with bit0 clear emit no descriptors.
        assert_eq!(
            world
                .gather_shadow_packets_with_auxiliary(&[0; 9], &[0xff; 4], [0; 4])
                .unwrap(),
            WorldShadowGather::default()
        );
        assert_eq!(
            world
                .gather_shadow_packets_with_auxiliary(&[1; 9], &[0xfe; 4], [0; 4])
                .unwrap(),
            WorldShadowGather::default()
        );
    }

    #[test]
    fn whole_view_masks_feed_world_packet_selection_without_color_visibility() {
        let mut world = shadow_mask_world(8);
        world.objects = vec![shadow_object(0, 4, 1), shadow_object(4, 8, 2)];
        world.visibility_groups.push(WorldVisibilityGroup {
            parent: 5,
            children: vec![6, 7],
        });
        let mut batches = Vec::new();
        world
            .for_each_whole_view_shadow_batch(|mask| {
                batches.push(world.gather_shadow_packets_with_auxiliary(mask, &[1, 1], [0; 4]).unwrap());
            })
            .unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].instance_ids, [4, 6, 7]);
        assert_eq!(batches[0].packets[0].object, 1);
        assert_eq!(
            (batches[0].packets[0].start, batches[0].packets[0].end),
            (0, 3)
        );
        assert!(world.gather_shadow_packets_with_auxiliary(&[], &[1, 1], [0; 4]).is_err());
        assert!(world.gather_shadow_packets_with_auxiliary(&[1; 8], &[], [0; 4]).is_err());
    }

    #[test]
    fn whole_view_shadow_masks_use_lod_low_bit_and_exclude_group_parents() {
        let mut world = shadow_mask_world(8);
        world.objects = vec![shadow_object(0, 4, 1), shadow_object(4, 8, 2)];
        world.visibility_groups.push(WorldVisibilityGroup {
            parent: 5,
            children: vec![6, 7],
        });
        let mut masks = Vec::new();
        world
            .for_each_whole_view_shadow_batch(|mask| masks.push(mask.to_vec()))
            .unwrap();
        assert_eq!(masks, [vec![0, 0, 0, 0, 1, 0, 1, 1]]);
    }

    #[test]
    fn whole_view_shadow_masks_preserve_ceil_stride_overlap() {
        let mut world = shadow_mask_world(4100);
        world.objects = vec![shadow_object(1, 3, 3), shadow_object(4096, 4098, 1)];
        world.visibility_groups.push(WorldVisibilityGroup {
            parent: 4099,
            children: vec![4098],
        });
        let mut masks = Vec::new();
        world
            .for_each_whole_view_shadow_batch(|mask| masks.push(mask.to_vec()))
            .unwrap();
        assert_eq!(masks.len(), 2);
        assert_eq!(&masks[0][..4], &[1, 0, 0, 1]);
        assert!(masks[0][4..4096].iter().all(|&value| value == 1));
        assert!(masks[0][4096..].iter().all(|&value| value == 0));
        assert!(masks[1][..4096].iter().all(|&value| value == 0));
        // Odd LOD in this far range is NOT cleared: the native overlap
        // windows are [0,2) and [2,4), not the mask's [4096,4100).
        assert_eq!(&masks[1][4096..], &[1, 1, 1, 0]);
    }

    #[test]
    fn whole_view_shadow_masks_empty_world_submits_nothing() {
        let world = shadow_mask_world(0);
        let mut called = false;
        world
            .for_each_whole_view_shadow_batch(|_| called = true)
            .unwrap();
        assert!(!called);
    }

    #[test]
    fn vertex_element_lookup_preserves_first_match_and_missing_sentinel() {
        assert_eq!(find_vertex_element(&BWM_ELEMENTS, 4, 1), 2);
        assert_eq!(find_vertex_element(&BWM_ELEMENTS, 5, 0), 5);
        assert_eq!(find_vertex_element(&BWM_ELEMENTS, 4, 3), u32::MAX);
        assert_eq!(find_vertex_element(&[], 0, 0), u32::MAX);
        assert_eq!(find_vertex_element(&[BWM_ELEMENTS[2]; 2], 4, 1), 0);
    }

    #[test]
    #[ignore = "requires local initialized-game registry capture"]
    fn vertex_element_lookup_matches_all_live_declarations() {
        let text = std::fs::read_to_string("artifacts/runtime-vertex-declarations.txt").unwrap();
        let mut verified = 0;
        for line in text.lines().filter(|l| l.starts_with("decl_raw ")) {
            let hex = line.split_whitespace().last().unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            let elements = bytes
                .chunks_exact(8)
                .map(|e| e.try_into().unwrap())
                .collect::<Vec<[u8; 8]>>();
            for (index, element) in elements.iter().enumerate() {
                assert_eq!(
                    find_vertex_element(&elements, element[4], element[5]),
                    index as u32
                );
                verified += 1;
            }
            assert_eq!(find_vertex_element(&elements, 255, 255), u32::MAX);
        }
        assert_eq!(verified, 35);
    }

    #[test]
    fn static_shadow_query_bounds_match_root() {
        let mut checked = 0;
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/static_shadow_bounds_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            let count = words[0] as usize;
            assert_eq!(words.len(), 3 + count * 8 + 6);
            assert_eq!(&words[1..3], &[0x7149_f2ca, 0xf149_f2ca]);
            let bounds: Vec<[[f32; 3]; 2]> = words[3..3 + count * 8]
                .chunks_exact(8)
                .map(|row| {
                    std::array::from_fn(|side| {
                        std::array::from_fn(|axis| f32::from_bits(row[side * 3 + axis]))
                    })
                })
                .collect();
            let result = static_query_bounds(bounds.iter()).unwrap();
            let actual: Vec<u32> = result.iter().flatten().map(|v| v.to_bits()).collect();
            assert_eq!(actual, words[3 + count * 8..], "root case {case}");
            checked += 1;
        }
        assert_eq!(checked, 192);
        assert_eq!(static_query_bounds(std::iter::empty()), None);
    }

    #[test]
    fn rejects_truncated_file() {
        assert!(parse(b"BWM1").is_err());
    }
    #[test]
    fn rejects_wrong_signature() {
        assert!(parse(&[0; 12]).is_err());
    }

    #[test]
    fn decodes_v21_umbra_id_for_transform_index() {
        let mut bytes = b"BWM1".to_vec();
        bytes.extend_from_slice(&21u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        for size in [0u32, 0, 0, 48, 0] {
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.resize(bytes.len() + size as usize, 0);
        }
        bytes.extend_from_slice(&0u32.to_le_bytes()); // meshes
        bytes.extend_from_slice(&0u32.to_le_bytes()); // objects
        bytes.extend_from_slice(&1u32.to_le_bytes()); // visibility records
        for value in [-2.0f32, -1.0, 0.0, 2.0, 3.0, 4.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes.extend_from_slice(&0xffffu16.to_le_bytes());
        for count in [1u32, 0, 0, 0] {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes()); // first auxiliary instance index
        bytes.extend_from_slice(&1u32.to_le_bytes()); // grouped records
        bytes.extend_from_slice(&0u32.to_le_bytes()); // parent index
        bytes.extend_from_slice(&1u32.to_le_bytes()); // child count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // child index
        bytes.extend_from_slice(&1u32.to_le_bytes()); // Tome object count
        bytes.extend_from_slice(&1u32.to_le_bytes()); // instance count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // instance index
        bytes.extend_from_slice(&0u32.to_le_bytes()); // spatial-query list
        let world = parse(&bytes).unwrap();
        assert_eq!(
            world.visibility[0].bounds,
            [[-2.0, -1.0, 0.0], [2.0, 3.0, 4.0]]
        );
        assert_eq!(
            world.static_query_bounds(),
            Some(world.visibility[0].bounds)
        );
        assert_eq!(world.visibility[0].umbra_id, 0x1234_5678);
        assert_eq!(world.visibility[0].trailing_field, 0xffff);
        assert_eq!(world.tome_object_instances, vec![vec![0]]);
        assert_eq!(
            world.auxiliary_instance_groups,
            [vec![0], vec![], vec![], vec![]]
        );
        assert_eq!(world.visibility_groups.len(), 1);
        assert_eq!(world.visibility_groups[0].parent, 0);
        assert_eq!(world.visibility_groups[0].children, [0]);
        assert!(world.spatial_query_instances.is_empty());
    }
}
