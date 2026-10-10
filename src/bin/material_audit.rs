#[path = "../archive.rs"]
mod archive;
#[path = "../material.rs"]
mod material;
#[path = "../texture.rs"]
mod texture;
#[path = "../world_mesh.rs"]
mod world_mesh;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let base = Path::new(r"D:\Programs\Steam\steamapps\common\Dishonored2\base");
    let catalog = archive::Catalog::open(base)?;
    let mut args = std::env::args().skip(1);
    let world_path = args
        .next()
        .unwrap_or_else(|| "artifacts/tutorial_p.bwm".into());
    let filter = args.next();
    let world = world_mesh::parse(&fs::read(&world_path)?)?;
    if filter.as_deref() == Some("--ray") {
        let origin: [f32; 3] = args
            .next()
            .ok_or("--ray needs origin x,y,z")?
            .split(',')
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "--ray origin needs 3 values")?;
        let direction: [f32; 3] = args
            .next()
            .ok_or("--ray needs direction x,y,z")?
            .split(',')
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "--ray direction needs 3 values")?;
        let mut hits = Vec::new();
        for object in world.objects.iter().filter(|object| object.lod == 0) {
            let mesh = &world.meshes[object.mesh_index];
            for matrix_index in object.matrix_start..object.matrix_end {
                let matrix = &world.matrices[matrix_index];
                for face in mesh.indices.chunks_exact(3) {
                    let triangle: [[f32; 3]; 3] = [face[0], face[1], face[2]].map(|index| {
                        let point = mesh.positions[index as usize];
                        std::array::from_fn(|row| {
                            matrix[row * 4] * point[0]
                                + matrix[row * 4 + 1] * point[1]
                                + matrix[row * 4 + 2] * point[2]
                                + matrix[row * 4 + 3]
                        })
                    });
                    if let Some(distance) = ray_triangle(origin, direction, triangle) {
                        if distance < 12.0 {
                            hits.push((
                                distance,
                                object.mesh_index,
                                matrix_index,
                                object.material_path.as_str(),
                            ));
                        }
                    }
                }
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        println!("{} BWM ray hits within 12m", hits.len());
        for hit in hits.into_iter().take(25) {
            println!("{hit:?}");
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--decal-values") {
        let mut values = BTreeMap::<String, usize>::new();
        let mut seen = HashSet::new();
        for object in world.objects.iter().filter(|object| object.lod == 0) {
            if !seen.insert(&object.material_path) {
                continue;
            }
            let decl = format!("generated/decls/material/{}.decl", object.material_path);
            let Some(bytes) = catalog.read(&decl)? else {
                continue;
            };
            let source = String::from_utf8_lossy(&bytes);
            if !source.contains("arkDecal") {
                continue;
            }
            let zbias = source
                .lines()
                .find_map(|line| {
                    let mut fields = line.trim().split_whitespace();
                    (fields.next() == Some("zbias"))
                        .then(|| fields.next().unwrap_or("missing").to_owned())
                })
                .unwrap_or_else(|| "missing".into());
            *values.entry(zbias).or_default() += 1;
        }
        println!("arkDecal zbias values: {values:?}");
        return Ok(());
    }
    if filter.as_deref() == Some("--surface") {
        let x: f32 = args.next().ok_or("--surface needs x")?.parse()?;
        let y: f32 = args.next().ok_or("--surface needs y")?.parse()?;
        let term = args
            .next()
            .ok_or("--surface needs a material path fragment")?;
        let mut hits = Vec::new();
        for object in world
            .objects
            .iter()
            .filter(|object| object.lod == 0 && object.material_path.contains(&term))
        {
            let mesh = &world.meshes[object.mesh_index];
            for matrix in &world.matrices[object.matrix_start..object.matrix_end] {
                for face in mesh.indices.chunks_exact(3) {
                    let triangle: [[f32; 3]; 3] = std::array::from_fn(|corner| {
                        let index = face[corner];
                        let p = mesh.positions[index as usize];
                        std::array::from_fn(|row| {
                            matrix[row * 4] * p[0]
                                + matrix[row * 4 + 1] * p[1]
                                + matrix[row * 4 + 2] * p[2]
                                + matrix[row * 4 + 3]
                        })
                    });
                    if let Some(z) = surface_height(x, y, triangle) {
                        hits.push((z, object.mesh_index, object.material_path.as_str()));
                    }
                }
            }
        }
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("{} visual surface hits at ({x},{y})", hits.len());
        for (z, mesh, material) in hits.into_iter().take(20) {
            println!("z={z:.4} mesh={mesh} {material}");
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--bounds") {
        let term = args
            .next()
            .ok_or("--bounds needs a material path fragment")?;
        for object in world
            .objects
            .iter()
            .filter(|object| object.lod == 0 && object.material_path.contains(&term))
        {
            let mesh = &world.meshes[object.mesh_index];
            for matrix in &world.matrices[object.matrix_start..object.matrix_end] {
                let mut lo = [f32::INFINITY; 3];
                let mut hi = [f32::NEG_INFINITY; 3];
                for position in &mesh.positions {
                    let point: [f32; 3] = std::array::from_fn(|row| {
                        matrix[row * 4] * position[0]
                            + matrix[row * 4 + 1] * position[1]
                            + matrix[row * 4 + 2] * position[2]
                            + matrix[row * 4 + 3]
                    });
                    for axis in 0..3 {
                        lo[axis] = lo[axis].min(point[axis]);
                        hi[axis] = hi[axis].max(point[axis]);
                    }
                }
                println!(
                    "mesh={} tris={} bounds={lo:?}..{hi:?} {}",
                    object.mesh_index,
                    mesh.indices.len() / 3,
                    object.material_path
                );
            }
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--flat-colors") {
        for object in world
            .objects
            .iter()
            .filter(|object| object.lod == 0 && object.material_path.contains("flat_colors"))
        {
            let mesh = &world.meshes[object.mesh_index];
            for matrix in &world.matrices[object.matrix_start..object.matrix_end] {
                let mut lo = [f32::INFINITY; 3];
                let mut hi = [f32::NEG_INFINITY; 3];
                for position in &mesh.positions {
                    let point: [f32; 3] = std::array::from_fn(|row| {
                        matrix[row * 4] * position[0]
                            + matrix[row * 4 + 1] * position[1]
                            + matrix[row * 4 + 2] * position[2]
                            + matrix[row * 4 + 3]
                    });
                    for axis in 0..3 {
                        lo[axis] = lo[axis].min(point[axis]);
                        hi[axis] = hi[axis].max(point[axis]);
                    }
                }
                println!(
                    "mesh={} bounds={lo:?}..{hi:?} {}",
                    object.mesh_index, object.material_path
                );
            }
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--transparent") {
        let mut seen = BTreeMap::<String, material::Transparency>::new();
        let mut candidates = Vec::new();
        for object in world.objects.iter().filter(|object| object.lod == 0) {
            let mode = if let Some(mode) = seen.get(&object.material_path) {
                *mode
            } else {
                let decl = format!("generated/decls/material/{}.decl", object.material_path);
                let mode = catalog
                    .read(&decl)?
                    .map(|bytes| material::transparency(&bytes))
                    .unwrap_or(material::Transparency::Opaque);
                seen.insert(object.material_path.clone(), mode);
                mode
            };
            if mode != material::Transparency::Blend {
                continue;
            }
            let mesh = &world.meshes[object.mesh_index];
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for vertex in &mesh.positions {
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(vertex[axis]);
                    hi[axis] = hi[axis].max(vertex[axis]);
                }
            }
            let extent = (0..3)
                .map(|axis| hi[axis] - lo[axis])
                .fold(0.0f32, f32::max);
            candidates.push((
                extent,
                object.material_path.as_str(),
                object.mesh_index,
                object.matrix_end - object.matrix_start,
            ));
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!(
            "{} blended world objects; largest local extents:",
            candidates.len()
        );
        for (extent, material, mesh, instances) in candidates.into_iter().take(40) {
            println!("extent={extent:.2} mesh={mesh} instances={instances} material={material}");
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--vertex-mask") {
        let mut channels = BTreeMap::<String, Option<usize>>::new();
        let mut candidates = Vec::new();
        for object in world.objects.iter().filter(|object| object.lod == 0) {
            let channel = if let Some(channel) = channels.get(&object.material_path) {
                *channel
            } else {
                let decl = format!("generated/decls/material/{}.decl", object.material_path);
                let channel = catalog
                    .read(&decl)?
                    .and_then(|bytes| material::layer0_vertex_mask_channel(&bytes));
                channels.insert(object.material_path.clone(), channel);
                channel
            };
            let Some(channel) = channel else {
                continue;
            };
            let mesh = &world.meshes[object.mesh_index];
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for vertex in &mesh.positions {
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(vertex[axis]);
                    hi[axis] = hi[axis].max(vertex[axis]);
                }
            }
            let extent = (0..3)
                .map(|axis| hi[axis] - lo[axis])
                .fold(0.0f32, f32::max);
            let alpha_range = mesh
                .packed_channels
                .iter()
                .map(|value| value[channel + 8])
                .fold((u8::MAX, 0u8), |(lo, hi), value| {
                    (lo.min(value), hi.max(value))
                });
            candidates.push((
                extent,
                object.material_path.as_str(),
                object.mesh_index,
                alpha_range,
            ));
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!(
            "{} LOD0 world objects use a layer-zero vertex mask; largest local extents:",
            candidates.len()
        );
        for (extent, material, mesh, range) in candidates.into_iter().take(40) {
            println!("extent={extent:.2} mesh={mesh} mask-range={range:?} material={material}");
        }
        return Ok(());
    }
    let mut lods = BTreeMap::<usize, usize>::new();
    for object in &world.objects {
        *lods.entry(object.lod).or_default() += 1;
    }
    println!("LOD object counts: {lods:?}");
    let lod0: HashSet<_> = world
        .objects
        .iter()
        .filter(|object| object.lod == 0)
        .map(|object| object.mesh_index)
        .collect();
    let lod1: HashSet<_> = world
        .objects
        .iter()
        .filter(|object| object.lod == 1)
        .map(|object| object.mesh_index)
        .collect();
    println!(
        "LOD meshes: {} at 0, {} at 1, {} shared",
        lod0.len(),
        lod1.len(),
        lod0.intersection(&lod1).count()
    );
    let transforms0: HashSet<_> = world
        .objects
        .iter()
        .filter(|object| object.lod == 0)
        .map(|object| (object.matrix_start, object.matrix_end))
        .collect();
    let transforms1: HashSet<_> = world
        .objects
        .iter()
        .filter(|object| object.lod == 1)
        .map(|object| (object.matrix_start, object.matrix_end))
        .collect();
    println!(
        "LOD transform ranges: {} at 0, {} at 1, {} shared",
        transforms0.len(),
        transforms1.len(),
        transforms0.intersection(&transforms1).count()
    );
    let mut count = BTreeMap::<String, (usize, String)>::new();
    let mut relevant_options = BTreeMap::<String, Vec<String>>::new();
    let mut seen = HashSet::new();
    for object in &world.objects {
        if filter
            .as_ref()
            .is_some_and(|term| !object.material_path.contains(term))
        {
            continue;
        }
        if !seen.insert(&object.material_path) {
            continue;
        }
        if filter.is_some() {
            let mesh = &world.meshes[object.mesh_index];
            let channels = (0..16)
                .map(|channel| {
                    let values: HashSet<_> = mesh
                        .packed_channels
                        .iter()
                        .map(|packed| packed[channel])
                        .collect();
                    (
                        values.iter().min().copied().unwrap_or(0),
                        values.iter().max().copied().unwrap_or(0),
                        values.len(),
                    )
                })
                .collect::<Vec<_>>();
            println!(
                "PACKED_CHANNELS mesh={} vertices={} {:?}",
                object.mesh_index,
                mesh.positions.len(),
                channels
            );
        }
        let state = if object.material_path.is_empty() {
            "empty material path".into()
        } else {
            let decl_path = format!("generated/decls/material/{}.decl", object.material_path);
            match catalog.read(&decl_path)? {
                None => format!("missing declaration: {decl_path}"),
                Some(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    if filter.is_some() {
                        println!("MATERIAL {}\n{}", object.material_path, text);
                    }
                    for line in text.lines() {
                        let line = line
                            .split("//")
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_ascii_lowercase();
                        if [
                            "alpha",
                            "transluc",
                            "blend",
                            "depthwrite",
                            "stagesort",
                            "mainprogram",
                        ]
                        .iter()
                        .any(|key| line.contains(key))
                        {
                            relevant_options
                                .entry(line)
                                .or_default()
                                .push(object.material_path.clone());
                        }
                    }
                    match material::diffuse_image(&bytes) {
                        None => format!("no diffusemap: {decl_path}"),
                        Some(image) => match catalog.read(&image)? {
                            None => {
                                let stem = image.strip_suffix("_mip0").unwrap_or(&image);
                                match catalog.read(stem)? {
                                    Some(bytes) => match texture::decode_inline_bc7(&bytes) {
                                        Ok(decoded) => {
                                            if filter.is_some() {
                                                let low = decoded
                                                    .pixels
                                                    .chunks_exact(4)
                                                    .filter(|pixel| pixel[3] < 128)
                                                    .count();
                                                println!(
                                                    "INLINE_ALPHA low={low}/{}",
                                                    decoded.pixels.len() / 4
                                                );
                                            }
                                            format!(
                                                "image inline {}x{}",
                                                decoded.width, decoded.height
                                            )
                                        }
                                        Err(error) => {
                                            format!("undecoded inline image: {stem}: {error}")
                                        }
                                    },
                                    None => format!("missing image: {image}"),
                                }
                            }
                            Some(bytes) => {
                                if filter.is_some() {
                                    if let Ok(image) = texture::decode_bc7(&bytes) {
                                        let counts = image.pixels.chunks_exact(4).fold(
                                            [0usize; 3],
                                            |mut counts, pixel| {
                                                if pixel[3] == 0 {
                                                    counts[0] += 1;
                                                } else if pixel[3] < 128 {
                                                    counts[1] += 1;
                                                } else {
                                                    counts[2] += 1;
                                                }
                                                counts
                                            },
                                        );
                                        println!(
                                            "ALPHA zero={} low={} high={}",
                                            counts[0], counts[1], counts[2]
                                        );
                                        let rgb = image.pixels.chunks_exact(4).step_by(257).fold(
                                            ([u8::MAX; 3], [0u8; 3], [0u64; 3], 0u64),
                                            |mut summary, pixel| {
                                                for channel in 0..3 {
                                                    summary.0[channel] =
                                                        summary.0[channel].min(pixel[channel]);
                                                    summary.1[channel] =
                                                        summary.1[channel].max(pixel[channel]);
                                                    summary.2[channel] += pixel[channel] as u64;
                                                }
                                                summary.3 += 1;
                                                summary
                                            },
                                        );
                                        println!(
                                            "RGB min={:?} max={:?} mean={:?}",
                                            rgb.0,
                                            rgb.1,
                                            rgb.2.map(|value| value / rgb.3.max(1))
                                        );
                                    }
                                }
                                format!(
                                    "image {} bytes {}x{} payload {}",
                                    bytes.len(),
                                    u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                                    u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
                                    u32::from_le_bytes(bytes[16..20].try_into().unwrap())
                                )
                            }
                        },
                    }
                }
            }
        };
        let entry = count
            .entry(state.clone())
            .or_insert((0, object.material_path.clone()));
        entry.0 += 1;
    }
    let successes: usize = count
        .iter()
        .filter(|(state, _)| state.starts_with("image "))
        .map(|(_, (objects, _))| objects)
        .sum();
    println!("{successes} unique world materials have an image");
    let mut missing: Vec<_> = count
        .into_iter()
        .filter(|(state, _)| !state.starts_with("image "))
        .collect();
    missing.sort_by_key(|(_, (objects, _))| std::cmp::Reverse(*objects));
    println!("{} unresolved image paths", missing.len());
    for (state, (objects, example)) in missing.into_iter().take(20) {
        println!("{objects:>4} {state} | {example}");
    }
    for (setting, examples) in relevant_options {
        println!("OPTION {:>4} {setting} | {}", examples.len(), examples[0]);
    }
    Ok(())
}

fn ray_triangle(origin: [f32; 3], direction: [f32; 3], triangle: [[f32; 3]; 3]) -> Option<f32> {
    let [a, b, c] = triangle;
    let sub = |x: [f32; 3], y: [f32; 3]| std::array::from_fn(|i| x[i] - y[i]);
    let cross = |x: [f32; 3], y: [f32; 3]| {
        [
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ]
    };
    let dot = |x: [f32; 3], y: [f32; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(direction, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-7 {
        return None;
    }
    let inverse = 1.0 / det;
    let from_a = sub(origin, a);
    let u = dot(from_a, p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(from_a, e1);
    let v = dot(direction, q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = dot(e2, q) * inverse;
    (distance > 0.001).then_some(distance)
}

fn surface_height(x: f32, y: f32, triangle: [[f32; 3]; 3]) -> Option<f32> {
    let [a, b, c] = triangle;
    let ux = b[0] - a[0];
    let uy = b[1] - a[1];
    let vx = c[0] - a[0];
    let vy = c[1] - a[1];
    let det = ux * vy - vx * uy;
    if det.abs() < 1e-7 {
        return None;
    }
    let dx = x - a[0];
    let dy = y - a[1];
    let u = (dx * vy - vx * dy) / det;
    let v = (ux * dy - dx * uy) / det;
    if u < -1e-5 || v < -1e-5 || u + v > 1.00001 {
        return None;
    }
    Some(a[2] + u * (b[2] - a[2]) + v * (c[2] - a[2]))
}
