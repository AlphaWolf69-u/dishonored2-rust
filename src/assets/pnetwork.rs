//! Sequential probe-network parsing using the installed record layout.
//! The original loader reads the stream through 1402b15e0 / 1403843f0.
use std::io::{self, Read};

struct Input<R> { source: R, position: usize, limit: usize }
impl<R: Read> Input<R> {
    fn read(&mut self, output: &mut [u8]) -> Result<(), String> {
        let end = self.position.checked_add(output.len()).ok_or("probe-network offset overflow")?;
        if end > self.limit { return Err("truncated probe-network data".into()); }
        self.source.read_exact(output).map_err(|e| e.to_string())?;
        self.position = end;
        Ok(())
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut out = [0; N]; self.read(&mut out)?; Ok(out)
    }
    fn u8(&mut self) -> Result<u8, String> { Ok(self.array::<1>()?[0]) }
    fn u32(&mut self) -> Result<u32, String> { Ok(u32::from_le_bytes(self.array()?)) }
    fn skip(&mut self, mut length: usize) -> Result<(), String> {
        let mut scratch = [0u8; 8192];
        while length != 0 { let n = length.min(scratch.len()); self.read(&mut scratch[..n])?; length -= n; }
        Ok(())
    }
    fn name(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        if length > 1 << 20 { return Err(format!("unreasonable probe resource name length {length}")); }
        if length > self.limit.saturating_sub(self.position) { return Err("truncated probe resource name".into()); }
        let mut bytes = vec![0; length]; self.read(&mut bytes)?;
        let mut value = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        value.truncate(value.trim_end_matches('\0').len());
        Ok(value)
    }
}
fn word(bytes: &[u8], at: usize) -> u32 { u32::from_le_bytes(bytes[at..at+4].try_into().unwrap()) }

fn read_names<R: Read>(input: &mut Input<R>) -> Result<(u8, Vec<String>, Vec<String>), String> {
    let header = input.array::<4>()?;
    if &header[..3] != b"PNW" { return Err("invalid or truncated PNW header".into()); }
    let version = header[3];
    if version < 5 { return Err(format!("unsupported probe-network version {version}")); }
    let count = input.u32()? as usize;
    if count > 1_000_000 { return Err(format!("unreasonable probe-network count {count}")); }
    let mut names = Vec::with_capacity(count);
    for _ in 0..count { if version < 6 { input.skip(12)?; } names.push(input.name()?); }
    let count = input.u32()? as usize;
    if count > 1_000_000 { return Err(format!("unreasonable secondary probe-network count {count}")); }
    let mut secondary = Vec::with_capacity(count);
    for _ in 0..count {
        if version < 6 { input.skip(12)?; }
        secondary.push(input.name()?);
        if version < 6 { input.skip(4)?; }
    }
    Ok((version, names, secondary))
}

pub fn ordered_probe_names(bytes: &[u8]) -> Result<(u8, Vec<String>, Vec<String>, usize), String> {
    let mut input = Input { source: io::Cursor::new(bytes), position: 0, limit: bytes.len() };
    let (version, names, secondary) = read_names(&mut input)?;
    Ok((version, names, secondary, input.position))
}

#[derive(Clone, Copy, Debug)]
pub struct Tetrahedron {
    pub vertex_indices: [u32; 4],
    pub probe_indices: [u16; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct TetrahedronVertex {
    pub position: [f32; 3],
    pub probe_weights: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub probe_name: String,
    pub first_tetrahedron: usize,
    pub tetrahedron_count: usize,
    pub planes: Vec<([f32; 3], [f32; 3], bool)>,
    /// Neighbor cell IDs, in the same order as the separating planes.
    pub neighbors: Vec<Option<usize>>,
    pub reflection_probe: u32,
    pub reflection_cell: Option<usize>,
    pub occluder_samples: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct RuntimeTetrahedron {
    pub shader_index: usize,
    pub cell_index: u32,
    pub positions: [[f32; 3]; 4],
}
#[derive(Clone, Debug)]
pub struct SpatialData {
    pub cells: Vec<Cell>,
    pub runtime_tetrahedra: Vec<RuntimeTetrahedron>,
    pub voronoi_version: u32,
    pub cell_count: usize,
    pub tetrahedra: Vec<Tetrahedron>,
    pub vertices: Vec<TetrahedronVertex>,
    pub bytes_consumed: usize,
}

#[derive(Clone, Debug)]
pub struct VertexReferences {
    pub cell_probe_indices: Vec<[u32; 4]>,
    pub instance_offsets: Vec<u32>,
    pub tetrahedron_indices: Vec<u32>,
    pub bytes_consumed: usize,
}


fn read_records<R: Read, const N: usize>(input: &mut Input<R>, mut remaining: usize, mut visit: impl FnMut(&[u8])) -> Result<(), String> {
    let mut buffer = [0u8; 65536];
    while remaining != 0 {
        let count = remaining.min(buffer.len() / N);
        let bytes = count * N;
        input.read(&mut buffer[..bytes])?;
        for record in buffer[..bytes].chunks_exact(N) { visit(record); }
        remaining -= count;
    }
    Ok(())
}

fn read_spatial<R: Read>(input: &mut Input<R>) -> Result<SpatialData, String> {
    let voronoi_version = input.u32()?;
    if voronoi_version < 9 { return Err(format!("unsupported Voronoi data version {voronoi_version}; reader is verified for v9")); }
    let data_feature = input.u8()?;
    let cell_count = input.u32()? as usize;
    if cell_count > 1_000_000 { return Err(format!("unreasonable Voronoi cell count {cell_count}")); }
    let mut cells = Vec::with_capacity(cell_count);
    for cell_index in 0..cell_count {
        let name_len = input.u32()? as usize;
        if name_len > input.limit.saturating_sub(input.position) { return Err("truncated Voronoi cell name".into()); }
        let mut raw = vec![0u8; name_len]; input.read(&mut raw)?;
        let mut probe_name = String::from_utf8(raw).map_err(|e| e.to_string())?;
        probe_name.truncate(probe_name.trim_end_matches('\0').len());
        let fixed = input.array::<29>()?;
        let first_tetrahedron = word(&fixed, 9) as usize;
        let reflection_probe = word(&fixed, 4);
        let reflection_cell = word(&fixed, 17);
        let reflection_cell = (reflection_cell != u32::MAX).then_some(reflection_cell as usize);
        let tetrahedron_count = word(&fixed, 13) as usize;
        let neighbor_records = word(&fixed, 25) as usize;
        if neighbor_records > 1_000_000 { return Err(format!("unreasonable Voronoi neighbor count {neighbor_records}")); }
        let mut planes = Vec::new();
        let mut neighbors = Vec::new();
        for neighbor_index in 0..neighbor_records {
            let mut fixed = [0u8; 46];
            let count = 44 + usize::from(voronoi_version >= 2) + usize::from(voronoi_version >= 7);
            input.read(&mut fixed[..count])?;
            let neighbor = word(&fixed, 0);
            neighbors.push((neighbor != u32::MAX).then_some(neighbor as usize));
            let f = |at| f32::from_bits(word(&fixed, at));
            planes.push(([f(8), f(12), f(16)], [f(20), f(24), f(28)], voronoi_version >= 7 && fixed[45] != 0));
            let vertex_count = input.u32()? as usize;
            if vertex_count > 1_000_000 { return Err(format!("unreasonable Voronoi neighbor vertex count {vertex_count} at cell {cell_index}, neighbor {neighbor_index}, offset {:#x}, feature {data_feature}", input.position)); }
            let extra = usize::from(voronoi_version < 6) * 4 + usize::from(voronoi_version < 4) * 4;
            input.skip(vertex_count.checked_mul(12 + extra).ok_or("Voronoi vertex count overflow")?)?;
        }
        let face_count = input.u32()? as usize;
        if face_count > 1_000_000 { return Err(format!("unreasonable Voronoi face count {face_count}")); }
        let mut occluder_samples = Vec::with_capacity(face_count);
        for _ in 0..face_count { occluder_samples.push(input.u32()? as usize); }
        input.skip(24)?;
        cells.push(Cell { probe_name, first_tetrahedron, tetrahedron_count, planes, neighbors,
            reflection_probe, reflection_cell, occluder_samples });
    }
    let tetrahedron_count = input.u32()? as usize;
    if tetrahedron_count > 10_000_000 { return Err(format!("unreasonable Voronoi tetrahedron count {tetrahedron_count}")); }
    let mut runtime_tetrahedra = Vec::with_capacity(tetrahedron_count);
    read_records::<_, 56>(input, tetrahedron_count, |raw| {
        let mut positions = [[0.; 3]; 4];
        for i in 0..4 { for c in 0..3 { positions[i][c] = f32::from_bits(word(raw, 8 + (i * 3 + c) * 4)); } }
        runtime_tetrahedra.push(RuntimeTetrahedron { shader_index: word(raw, 0) as usize,
            cell_index: word(raw, 4), positions });
    })?;
    let tetrahedron_bytes = input.u32()? as usize;
    if tetrahedron_bytes % 24 != 0 { return Err(format!("tetrahedron shader buffer has invalid byte size {tetrahedron_bytes}")); }
    if tetrahedron_bytes > input.limit.saturating_sub(input.position) { return Err("truncated tetrahedron shader buffer".into()); }
    let mut tetrahedra = Vec::with_capacity(tetrahedron_bytes / 24);
    read_records::<_, 24>(input, tetrahedron_bytes / 24, |raw| {
        let first = word(raw, 16); let second = word(raw, 20);
        tetrahedra.push(Tetrahedron { vertex_indices: [word(raw, 0), word(raw, 4), word(raw, 8), word(raw, 12)],
            probe_indices: [first as u16, (first >> 16) as u16, second as u16, (second >> 16) as u16] });
    })?;
    let vertex_bytes = input.u32()? as usize;
    if vertex_bytes % 16 != 0 { return Err(format!("tetrahedron-vertex buffer has invalid byte size {vertex_bytes}")); }
    if vertex_bytes > input.limit.saturating_sub(input.position) { return Err("truncated tetrahedron-vertex shader buffer".into()); }
    let mut vertices = Vec::with_capacity(vertex_bytes / 16);
    read_records::<_, 16>(input, vertex_bytes / 16, |raw| {
        let packed = word(raw, 12);
        vertices.push(TetrahedronVertex { position: [f32::from_bits(word(raw, 0)), f32::from_bits(word(raw, 4)), f32::from_bits(word(raw, 8))],
            probe_weights: [packed as u8, (packed >> 8) as u8, (packed >> 16) as u8, (packed >> 24) as u8] });
    })?;
    if tetrahedra.iter().any(|t| t.vertex_indices.iter().any(|&i| i as usize >= vertices.len())) {
        return Err("tetrahedron references a vertex outside its shader buffer".into());
    }
    if cells.iter().any(|c| c.first_tetrahedron.checked_add(c.tetrahedron_count).is_none_or(|end| end > runtime_tetrahedra.len()))
        || runtime_tetrahedra.iter().any(|t| t.shader_index >= tetrahedra.len()) {
        return Err("Voronoi CPU query references an invalid tetrahedron".into());
    }
    Ok(SpatialData { voronoi_version, cell_count, cells, runtime_tetrahedra, tetrahedra, vertices, bytes_consumed: input.position })
}

pub fn spatial_data(bytes: &[u8], offset: usize) -> Result<SpatialData, String> {
    if offset > bytes.len() { return Err("truncated probe-network data".into()); }
    let mut input = Input { source: io::Cursor::new(&bytes[offset..]), position: offset, limit: bytes.len() };
    read_spatial(&mut input)
}

fn read_words<R: Read>(input: &mut Input<R>, size: usize) -> Result<Vec<u32>, String> {
    if size > input.limit.saturating_sub(input.position) { return Err("truncated probe-network word buffer".into()); }
    let mut words = Vec::with_capacity(size / 4);
    let mut scratch = [0u8; 65536];
    let mut left = size;
    while left != 0 {
        let n = left.min(scratch.len());
        input.read(&mut scratch[..n])?;
        words.extend(scratch[..n].chunks_exact(4).map(|v| u32::from_le_bytes(v.try_into().unwrap())));
        left -= n;
    }
    Ok(words)
}
fn read_references<R: Read>(input: &mut Input<R>, secondary_probe_count: usize) -> Result<VertexReferences, String> {
    let rows = input.u32()? as usize; let columns = input.u32()? as usize;
    if rows > 1_000_000 || columns > 4 { return Err(format!("invalid probe-index table dimensions {rows}x{columns}")); }
    let count = rows.checked_mul(columns).ok_or("probe-index table size overflow")?;
    let bytes = count.checked_mul(4).ok_or("probe-index table size overflow")?;
    if bytes > input.limit.saturating_sub(input.position) { return Err("truncated probe-index table".into()); }
    let mut cell_probe_indices = vec![[u32::MAX; 4]; rows];
    for i in 0..count {
        let value = input.u32()?;
        cell_probe_indices[i / columns][i % columns] = if secondary_probe_count != 0 && value != u32::MAX && value as usize >= secondary_probe_count { 0 } else { value };
    }
    let size = ((input.u32()? as usize) >> 2) * 4;
    let tetrahedron_indices = read_words(input, size)?;
    let status = input.u8()?;
    if status != 0 { return Err("probe-network vertex lighting unavailable (native load status 9)".into()); }
    let size = ((input.u32()? as usize) >> 2) * 4;
    let instance_offsets = read_words(input, size)?;
    Ok(VertexReferences { cell_probe_indices, instance_offsets, tetrahedron_indices, bytes_consumed: input.position })
}

pub fn vertex_references(bytes: &[u8], offset: usize, secondary_probe_count: usize) -> Result<VertexReferences, String> {
    if offset > bytes.len() { return Err("truncated probe-network data".into()); }
    let mut input = Input { source: io::Cursor::new(&bytes[offset..]), position: offset, limit: bytes.len() };
    read_references(&mut input, secondary_probe_count)
}

/// Parse one uncompressed network in stream order without retaining the input.
pub(crate) fn read_stream(source: &mut dyn Read, length: usize) -> Result<(Vec<String>, Vec<String>, SpatialData, VertexReferences), String> {
    let mut input = Input { source, position: 0, limit: length };
    let (_, names, secondary) = read_names(&mut input)?;
    let spatial = read_spatial(&mut input)?;
    let references = read_references(&mut input, secondary.len())?;
    // Match the existing full-member acquisition: validate even unused trailing bytes.
    let tail = length - input.position;
    input.skip(tail)?;
    Ok((names, secondary, spatial, references))
}

/// Stable diagnostic fingerprint of the parsed network, including every
/// retained table entry. Called only when explicitly requested.
pub(crate) fn fingerprint(names: &[String], secondary: &[String], data: &SpatialData, refs: &VertexReferences) -> u32 {
    fn u64v(c: &mut flate2::Crc, v: u64) { c.update(&v.to_le_bytes()); }
    fn u32v(c: &mut flate2::Crc, v: u32) { c.update(&v.to_le_bytes()); }
    fn string(c: &mut flate2::Crc, v: &str) { u64v(c,v.len() as u64); c.update(v.as_bytes()); }
    let mut c=flate2::Crc::new();
    u64v(&mut c,names.len() as u64); for v in names { string(&mut c,v); }
    u64v(&mut c,secondary.len() as u64); for v in secondary { string(&mut c,v); }
    u32v(&mut c,data.voronoi_version); u64v(&mut c,data.cell_count as u64); u64v(&mut c,data.bytes_consumed as u64);
    u64v(&mut c,data.cells.len() as u64);
    for cell in &data.cells {
        string(&mut c,&cell.probe_name); u64v(&mut c,cell.first_tetrahedron as u64); u64v(&mut c,cell.tetrahedron_count as u64);
        u64v(&mut c,cell.planes.len() as u64);
        for (position,normal,skip) in &cell.planes { for v in position.iter().chain(normal) { u32v(&mut c,v.to_bits()); } c.update(&[u8::from(*skip)]); }
        u64v(&mut c,cell.neighbors.len() as u64); for v in &cell.neighbors { u64v(&mut c,v.map_or(u64::MAX,|x|x as u64)); }
        u32v(&mut c,cell.reflection_probe); u64v(&mut c,cell.reflection_cell.map_or(u64::MAX,|x|x as u64));
        u64v(&mut c,cell.occluder_samples.len() as u64); for &v in &cell.occluder_samples { u64v(&mut c,v as u64); }
    }
    u64v(&mut c,data.runtime_tetrahedra.len() as u64);
    for t in &data.runtime_tetrahedra { u64v(&mut c,t.shader_index as u64); u32v(&mut c,t.cell_index); for p in &t.positions { for v in p { u32v(&mut c,v.to_bits()); } } }
    u64v(&mut c,data.tetrahedra.len() as u64);
    for t in &data.tetrahedra { for v in t.vertex_indices { u32v(&mut c,v); } for v in t.probe_indices { c.update(&v.to_le_bytes()); } }
    u64v(&mut c,data.vertices.len() as u64);
    for v in &data.vertices { for x in v.position { u32v(&mut c,x.to_bits()); } c.update(&v.probe_weights); }
    u64v(&mut c,refs.cell_probe_indices.len() as u64); for row in &refs.cell_probe_indices { for &v in row { u32v(&mut c,v); } }
    let mut bytes = [0u8; 65536];
    for values in [&refs.tetrahedron_indices, &refs.instance_offsets] {
        u64v(&mut c,values.len() as u64);
        for chunk in values.chunks(bytes.len() / 4) {
            for (v, target) in chunk.iter().zip(bytes.chunks_exact_mut(4)) { target.copy_from_slice(&v.to_le_bytes()); }
            c.update(&bytes[..chunk.len() * 4]);
        }
    }
    u64v(&mut c,refs.bytes_consumed as u64);
    c.sum()
}

/// Diagnostic CPU time of the calling thread in 100 ns units.
#[cfg(windows)]
pub(crate) fn thread_cpu_100ns() -> Option<u64> {
    #[repr(C)]
    struct FileTime { low: u32, high: u32 }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn GetThreadTimes(thread: isize, create: *mut FileTime, exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    }
    let mut create=FileTime{low:0,high:0}; let mut exit=FileTime{low:0,high:0};
    let mut kernel=FileTime{low:0,high:0}; let mut user=FileTime{low:0,high:0};
    // The pseudo-handle names the calling thread; each FILETIME points to
    // writable stack storage for the duration of the system call.
    let ok=unsafe { GetThreadTimes(GetCurrentThread(), &mut create, &mut exit, &mut kernel, &mut user) };
    if ok == 0 { return None; }
    Some(((kernel.high as u64) << 32 | kernel.low as u64) + ((user.high as u64) << 32 | user.low as u64))
}
#[cfg(not(windows))]
pub(crate) fn thread_cpu_100ns() -> Option<u64> { None }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_v7_ordered_names() {
        let bytes = b"PNW\x07\x01\0\0\0\x03\0\0\0foo\0\0\0\0";
        let (version, names, secondary, end) = ordered_probe_names(bytes).unwrap();
        assert_eq!(version, 7);
        assert_eq!(names, ["foo"]);
        assert!(secondary.is_empty());
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn reads_vertex_reference_tail() {
        let mut bytes = Vec::new();
        for word in [1u32, 2, 7, 9, 8, 11, 22] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        for word in [12u32, 3, 4, 5] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let tail = vertex_references(&bytes, 0, 8).unwrap();
        assert_eq!(tail.cell_probe_indices, [[7, 0, u32::MAX, u32::MAX]]);
        assert_eq!(tail.tetrahedron_indices, [11, 22]);
        assert_eq!(tail.instance_offsets, [3, 4, 5]);
        assert_eq!(tail.bytes_consumed, bytes.len());
        bytes[28] = 1;
        assert!(vertex_references(&bytes, 0, 8)
            .unwrap_err()
            .contains("status 9"));
    }

    #[test]
    fn word_buffers_floor_the_declared_size() {
        let mut bytes = Vec::new();
        for word in [0u32, 4, 5, 123] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let tail = vertex_references(&bytes, 0, 0).unwrap();
        assert_eq!(tail.tetrahedron_indices, [123]);
        assert!(tail.instance_offsets.is_empty());
        assert_eq!(tail.bytes_consumed, bytes.len());
        assert!(vertex_references(&bytes[..bytes.len() - 1], 0, 0).is_err());
    }
}
