#[path = "../entities.rs"]
mod entities;
#[path = "../physworld.rs"]
mod physworld;

use std::{env, fs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let verbose = env::args().any(|argument| argument == "--verbose");
    let path = env::args()
        .nth(1)
        .ok_or("usage: physworld-audit <extracted .bphysworld>")?;
    let bytes = fs::read(&path)?;
    let world = physworld::parse(&bytes)?;
    if env::args().any(|argument| argument == "--schema") {
        for typ in &world.types {
            if typ.name.contains("Compressed")
                || typ.name.contains("StaticTree")
                || typ.name.contains("StaticMesh")
                || typ.name.contains("ShapeInstance")
                || typ.name.contains("Primitive")
                || typ.name.contains("Palette")
            {
                println!(
                    "TYPE {} size={} parent={}",
                    typ.name, typ.byte_size, typ.parent
                );
                for field in &typ.fields {
                    println!(
                        "  {} +{:#x} type={}",
                        field.name, field.offset, world.types[field.type_index].name
                    );
                }
            }
        }
        return Ok(());
    }
    if env::args().any(|argument| argument == "--body-shapes") {
        let root = world.data_start + world.items[1].offset;
        let body_ref = u32::from_le_bytes(bytes[root + 0x38..root + 0x3c].try_into()?) as usize;
        let bodies = &world.items[body_ref];
        println!(
            "body item {body_ref} type={} stride={} count={}",
            world.types[bodies.type_index].name,
            world.types[bodies.type_index].byte_size,
            bodies.count
        );
        for number in 0..bodies.count {
            let at = world.data_start
                + bodies.offset
                + number * world.types[bodies.type_index].byte_size;
            let shape_ref = u32::from_le_bytes(bytes[at..at + 4].try_into()?) as usize;
            let shape_type = world
                .items
                .get(shape_ref)
                .map(|item| world.types[item.type_index].name.as_str())
                .unwrap_or("invalid");
            let word = |offset| {
                u32::from_le_bytes(bytes[at + offset..at + offset + 4].try_into().unwrap())
            };
            println!("body {number}: shape item {shape_ref} type={shape_type} flags={:#x} control={:#x} filter={:#x} material={} name_ref={} motion={}", word(8), word(12), word(16), word(20), word(24), word(40));
        }
        return Ok(());
    }
    if env::args().any(|argument| argument == "--unknown-shapes") {
        for (index, item) in world.items.iter().enumerate() {
            if world.types[item.type_index].name != "hknpConvexShape" {
                continue;
            }
            let at = world.data_start + item.offset + 64;
            let reference = u32::from_le_bytes(bytes[at..at + 4].try_into()?) as usize;
            let vertices = world.items.get(reference);
            let mut points: Vec<[f32; 3]> = Vec::new();
            if let Some(v) = vertices {
                for n in 0..v.count {
                    let at = world.data_start + v.offset + n * 16;
                    points.push(std::array::from_fn(|axis| {
                        f32::from_le_bytes(
                            bytes[at + axis * 4..at + axis * 4 + 4].try_into().unwrap(),
                        )
                    }));
                }
            }
            let volume6 = if points.len() == 4 {
                let a: [f32; 3] = std::array::from_fn(|i| points[1][i] - points[0][i]);
                let b: [f32; 3] = std::array::from_fn(|i| points[2][i] - points[0][i]);
                let c: [f32; 3] = std::array::from_fn(|i| points[3][i] - points[0][i]);
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0])
            } else {
                f32::NAN
            };
            println!("generic convex item {index} vertices ref={reference} type={} count={} volume6={volume6} points={points:?}",
                vertices.map(|v| world.types[v.type_index].name.as_str()).unwrap_or("invalid"),
                vertices.map(|v| v.count).unwrap_or(0));
        }
        return Ok(());
    }
    for ty in &world.types {
        if ty.name == "hknpBodyCinfo" || ty.name == "hknpConvexShape" {
            println!(
                "Body-Cinfo type: {} bytes; fields {:?}",
                ty.byte_size,
                ty.fields
                    .iter()
                    .map(|field| (field.name.as_str(), field.offset))
                    .collect::<Vec<_>>()
            );
        }
    }
    let poses = world.body_poses(&bytes)?;
    if env::args().any(|argument| argument == "--body-poses") {
        for (index, pose) in poses.iter().enumerate() {
            println!(
                "body {index} position={:?} orientation={:?}",
                pose.position, pose.orientation
            );
        }
        return Ok(());
    }
    let rotated: Vec<_> = poses
        .iter()
        .enumerate()
        .filter(|(_, pose)| {
            let q = pose.orientation;
            q[0].abs() + q[1].abs() + q[2].abs() > 1e-4 || (q[3].abs() - 1.0).abs() > 1e-4
        })
        .collect();
    println!(
        "Body poses: {} total, {} non-identity orientations; examples {:?}",
        poses.len(),
        rotated.len(),
        rotated.iter().take(5).collect::<Vec<_>>()
    );
    if let Some(shape) = env::args().skip_while(|arg| arg != "--shape").nth(1) {
        inspect_compressed_shape(&world, &bytes, shape.parse()?)?;
        return Ok(());
    }
    let collision = world.collision_shapes(&bytes)?;
    if let Some(raw) = env::args().skip_while(|arg| arg != "--triangle").nth(1) {
        let values = raw
            .split(',')
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 9 {
            return Err("--triangle requires nine coordinates".into());
        }
        let vertices: [[f32; 3]; 3] =
            std::array::from_fn(|vertex| std::array::from_fn(|axis| values[vertex * 3 + axis]));
        for triangle in &collision.triangles {
            if triangle
                .vertices
                .iter()
                .zip(vertices)
                .all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.0001))
            {
                let index = triangle.source_shape_item;
                let item = &world.items[index];
                println!(
                    "MATCH shape={index} type={} data_offset={} triangle={:?} bodies={:?}",
                    world.types[item.type_index].name,
                    item.offset,
                    triangle.vertices,
                    collision.shape_bodies.get(&index)
                );
                for (parent, item) in world.items.iter().enumerate() {
                    if world.types[item.type_index].name != "hknpCompoundShape" {
                        continue;
                    }
                    let mut kind = item.type_index;
                    let offset = loop {
                        if let Some(field) = world.types[kind]
                            .fields
                            .iter()
                            .find(|field| field.name == "instances")
                        {
                            break field.offset;
                        }
                        kind = world.types[kind].parent;
                        if kind == 0 {
                            return Err("compound instances schema missing".into());
                        }
                    };
                    let at = world.data_start + item.offset + offset;
                    let list = u32::from_le_bytes(bytes[at..at + 4].try_into()?) as usize;
                    if list == 0 {
                        continue;
                    }
                    let list = &world.items[list];
                    for n in 0..list.count {
                        let at = world.data_start
                            + list.offset
                            + n * world.types[list.type_index].byte_size;
                        let child =
                            u32::from_le_bytes(bytes[at + 0x50..at + 0x54].try_into()?) as usize;
                        if child == index {
                            println!(
                                "PARENT compound={parent} instance={n} shapeTag={} scaleW={:#x}",
                                u16::from_le_bytes(bytes[at + 0x58..at + 0x5a].try_into()?),
                                u32::from_le_bytes(bytes[at + 0x4c..at + 0x50].try_into()?)
                            );
                        }
                    }
                }
            }
        }
        return Ok(());
    }
    if let Some(raw) = env::args().skip_while(|arg| arg != "--point").nth(1) {
        let coordinates: Vec<f32> = raw.split(',').map(str::parse).collect::<Result<_, _>>()?;
        if coordinates.len() != 3 {
            return Err("--point requires x,y,z".into());
        }
        let point = [coordinates[0], coordinates[1], coordinates[2]];
        let distance = |low: [f32; 3], high: [f32; 3]| -> f32 {
            (0..3)
                .map(|axis| (point[axis] - point[axis].clamp(low[axis], high[axis])).powi(2))
                .sum::<f32>()
                .sqrt()
        };
        let mut nearby = Vec::new();
        for shape in &collision.compressed {
            let d = distance(shape.bounds_min, shape.bounds_max);
            if d < 20.0 {
                nearby.push((
                    d,
                    "mesh",
                    shape.source_shape_item,
                    shape.bounds_min,
                    shape.bounds_max,
                ));
            }
        }
        for shape in &collision.convex {
            let low = std::array::from_fn(|axis| {
                shape
                    .vertices
                    .iter()
                    .map(|v| v[axis])
                    .fold(f32::INFINITY, f32::min)
            });
            let high = std::array::from_fn(|axis| {
                shape
                    .vertices
                    .iter()
                    .map(|v| v[axis])
                    .fold(f32::NEG_INFINITY, f32::max)
            });
            let d = distance(low, high);
            if d < 20.0 {
                nearby.push((d, "convex", shape.source_shape_item, low, high));
            }
        }
        nearby.sort_by(|a, b| a.0.total_cmp(&b.0));
        println!(
            "Point {point:?}: {} authored shape bounds within 2m, {} within 20m",
            nearby.iter().filter(|entry| entry.0 < 2.0).count(),
            nearby.len()
        );
        for entry in nearby.iter().take(20) {
            println!("  {entry:?}");
        }
        let mut triangle_near = 0usize;
        let mut triangle_closest = (f32::INFINITY, 0usize);
        for triangle in &collision.triangles {
            let low = std::array::from_fn(|axis| {
                triangle
                    .vertices
                    .iter()
                    .map(|v| v[axis])
                    .fold(f32::INFINITY, f32::min)
            });
            let high = std::array::from_fn(|axis| {
                triangle
                    .vertices
                    .iter()
                    .map(|v| v[axis])
                    .fold(f32::NEG_INFINITY, f32::max)
            });
            let d = distance(low, high);
            if d < 2.0 {
                triangle_near += 1;
            }
            if d < triangle_closest.0 {
                triangle_closest = (d, triangle.source_shape_item);
            }
        }
        println!("  decoded triangle AABBs within 2m: {triangle_near}; closest triangle AABB: {triangle_closest:?}");
        let mut column_hits = Vec::new();
        for triangle in &collision.triangles {
            let [a, b, c] = triangle.vertices;
            let ab = [b[0] - a[0], b[1] - a[1]];
            let ac = [c[0] - a[0], c[1] - a[1]];
            let det = ab[0] * ac[1] - ac[0] * ab[1];
            if det.abs() < 1e-7 {
                continue;
            }
            let dx = point[0] - a[0];
            let dy = point[1] - a[1];
            let u = (dx * ac[1] - ac[0] * dy) / det;
            let v = (ab[0] * dy - dx * ab[1]) / det;
            if u < -1e-5 || v < -1e-5 || u + v > 1.00001 {
                continue;
            }
            let height = a[2] + u * (b[2] - a[2]) + v * (c[2] - a[2]);
            if (height - point[2]).abs() < 10.0 {
                column_hits.push((height, triangle.source_shape_item));
            }
        }
        column_hits.sort_by(|a, b| (a.0 - point[2]).abs().total_cmp(&(b.0 - point[2]).abs()));
        println!(
            "  decoded vertical column hits within 10m Z: {}; closest: {:?}",
            column_hits.len(),
            column_hits.iter().take(15).collect::<Vec<_>>()
        );
        return Ok(());
    }
    let start = fs::read(std::path::Path::new(&path).with_extension("entities"))
        .ok()
        .and_then(|bytes| entities::parse(&String::from_utf8_lossy(&bytes)).ok())
        .and_then(|document| entities::player_start(&document))
        .map(|player| player.position);
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    let mut vertices = 0usize;
    let mut near_5 = 0usize;
    let mut near_20 = 0usize;
    for shape in &collision.convex {
        vertices += shape.vertices.len();
        let mut shape_low = [f32::INFINITY; 3];
        let mut shape_high = [f32::NEG_INFINITY; 3];
        for vertex in &shape.vertices {
            for axis in 0..3 {
                low[axis] = low[axis].min(vertex[axis]);
                high[axis] = high[axis].max(vertex[axis]);
                shape_low[axis] = shape_low[axis].min(vertex[axis]);
                shape_high[axis] = shape_high[axis].max(vertex[axis]);
            }
        }
        if let Some(spawn) = start {
            let distance = (0..3)
                .map(|axis| {
                    (spawn[axis] - spawn[axis].clamp(shape_low[axis], shape_high[axis])).powi(2)
                })
                .sum::<f32>()
                .sqrt();
            if distance < 5.0 {
                near_5 += 1;
            }
            if distance < 20.0 {
                near_20 += 1;
            }
        }
    }
    println!("Reachable collision: {} convex shapes, {vertices} vertices, {} compressed meshes, {} decoded triangles, {} mesh decode failures, unknown={:?}; bounds {low:?} .. {high:?}",
        collision.convex.len(), collision.compressed.len(), collision.triangles.len(), collision.compressed_decode_failures.len(), collision.unknown_shapes);
    let mut low_bytes = std::collections::BTreeMap::<u16, usize>::new();
    let mut high_byte_valid = 0usize;
    let mut high_byte_invalid_examples = Vec::new();
    for error in &collision.compressed_decode_failures {
        let Some((_, rest)) = error.split_once("shared vertex index ") else {
            continue;
        };
        let Some((raw, rest)) = rest.split_once(" >= ") else {
            continue;
        };
        let Some(count) = rest.split_whitespace().next() else {
            continue;
        };
        let (Ok(raw), Ok(count)) = (raw.parse::<usize>(), count.parse::<usize>()) else {
            continue;
        };
        *low_bytes.entry((raw & 0xff) as u16).or_default() += 1;
        if (raw >> 8) < count {
            high_byte_valid += 1;
        } else if high_byte_invalid_examples.len() < 12 {
            high_byte_invalid_examples.push((raw, count));
        }
    }
    println!("Invalid shared-index first failures: low-byte histogram {low_bytes:?}; high-byte candidate in range {high_byte_valid}; invalid examples {high_byte_invalid_examples:?}");
    let mut failed_shapes = std::collections::BTreeSet::new();
    for error in &collision.compressed_decode_failures {
        let Some(rest) = error.strip_prefix("compressed shape ") else {
            continue;
        };
        let Some(shape) = rest
            .split_whitespace()
            .next()
            .and_then(|part| part.parse::<usize>().ok())
        else {
            continue;
        };
        failed_shapes.insert(shape);
    }
    let mut proxy_shapes = 0usize;
    let mut surface_shapes = 0usize;
    let mut surface_primitives = 0usize;
    let mut surface_examples = Vec::new();
    for shape in failed_shapes {
        let (_, surfaces) = world.compressed_surface_primitives(&bytes, shape)?;
        if surfaces == 0 {
            proxy_shapes += 1;
        } else {
            surface_shapes += 1;
            surface_primitives += surfaces;
            surface_examples.push((shape, surfaces));
        }
    }
    println!("Failed shape topology: {proxy_shapes} contain only point/line primitives; {surface_shapes} contain {surface_primitives} potential surface primitives");
    println!("Failed surface shape items: {surface_examples:?}");
    for error in collision.compressed_decode_failures.iter().take(10) {
        println!("  decode failure: {error}");
    }
    if verbose {
        for shape in &collision.compressed {
            if collision.compressed_decode_failures.iter().any(|error| {
                error.starts_with(&format!("compressed shape {} ", shape.source_shape_item))
            }) {
                println!(
                    "  undecoded mesh {} bounds {:?} .. {:?}",
                    shape.source_shape_item, shape.bounds_min, shape.bounds_max
                );
            }
        }
    }
    if let Some(spawn) = start {
        println!(
            "Near player start {spawn:?}: {near_5} convex AABBs within 5m; {near_20} within 20m"
        );
    }
    for (index, shape) in collision.convex.iter().take(3).enumerate() {
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for vertex in &shape.vertices {
            for axis in 0..3 {
                low[axis] = low[axis].min(vertex[axis]);
                high[axis] = high[axis].max(vertex[axis]);
            }
        }
        println!(
            "  reachable convex #{index} source item {} AABB {low:?} .. {high:?}",
            shape.source_shape_item
        );
    }
    let mut compressed_near = 0usize;
    for shape in &collision.compressed {
        let Some(spawn) = start else {
            break;
        };
        let distance = (0..3)
            .map(|axis| {
                (spawn[axis] - spawn[axis].clamp(shape.bounds_min[axis], shape.bounds_max[axis]))
                    .powi(2)
            })
            .sum::<f32>()
            .sqrt();
        if distance < 5.0 {
            compressed_near += 1;
            println!(
                "  compressed near spawn: item {} AABB {:?} .. {:?}",
                shape.source_shape_item, shape.bounds_min, shape.bounds_max
            );
        }
    }
    if start.is_some() {
        println!("Compressed meshes near spawn (<5m AABB): {compressed_near}");
    }
    let compressed_bounds: std::collections::HashMap<usize, ([f32; 3], [f32; 3])> = collision
        .compressed
        .iter()
        .map(|shape| {
            (
                shape.source_shape_item,
                (shape.bounds_min, shape.bounds_max),
            )
        })
        .collect();
    let mut outside = 0usize;
    let mut degenerate = 0usize;
    let mut triangles_near_spawn = 0usize;
    for triangle in &collision.triangles {
        if let Some((min, max)) = compressed_bounds.get(&triangle.source_shape_item) {
            if triangle.vertices.iter().any(|vertex| {
                (0..3).any(|axis| vertex[axis] < min[axis] - 0.1 || vertex[axis] > max[axis] + 0.1)
            }) {
                outside += 1;
            }
        }
        let ab: [f32; 3] =
            std::array::from_fn(|axis| triangle.vertices[1][axis] - triangle.vertices[0][axis]);
        let ac: [f32; 3] =
            std::array::from_fn(|axis| triangle.vertices[2][axis] - triangle.vertices[0][axis]);
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        if cross.iter().map(|value| value * value).sum::<f32>() < 1e-10 {
            degenerate += 1;
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in &triangle.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex[axis]);
                max[axis] = max[axis].max(vertex[axis]);
            }
        }
        if let Some(spawn) = start {
            let distance = (0..3)
                .map(|axis| (spawn[axis] - spawn[axis].clamp(min[axis], max[axis])).powi(2))
                .sum::<f32>()
                .sqrt();
            if distance < 5.0 {
                triangles_near_spawn += 1;
            }
        }
    }
    println!("Triangle audit: {outside} outside their compressed-shape bounds; {degenerate} degenerate; {triangles_near_spawn} AABBs within 5m of spawn");
    if !verbose {
        return Ok(());
    }
    let reached_convex: std::collections::HashSet<_> = collision
        .convex
        .iter()
        .map(|shape| shape.source_shape_item)
        .collect();
    let reached_mesh: std::collections::HashSet<_> = collision
        .compressed
        .iter()
        .map(|shape| shape.source_shape_item)
        .collect();
    for name in [
        "hknpConvexPolytopeShape",
        "hknpConvexShape",
        "hknpCompressedMeshShape",
    ] {
        let all: Vec<_> = world
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (world.types[item.type_index].name == name).then_some(index)
            })
            .collect();
        let reached = if name == "hknpCompressedMeshShape" {
            &reached_mesh
        } else {
            &reached_convex
        };
        let missing: Vec<_> = all
            .iter()
            .filter(|index| !reached.contains(index))
            .take(20)
            .collect();
        println!(
            "Shape reachability {name}: {}/{} unique reached; first missing {missing:?}",
            all.iter().filter(|index| reached.contains(index)).count(),
            all.len()
        );
    }
    println!("Havok SDK {} | {} materials | {} material-map entries | {} types | {} items | {} data bytes",
        world.sdk_version, world.materials.len(), world.material_map.len(),
        world.types.len(), world.items.len(), world.data_size);
    for (name, count) in world.item_inventory() {
        if name.contains("Shape") || name.contains("PhysicsSystem") || name.contains("Body") {
            println!("  {name}: {count} items");
        }
    }
    for (index, typ) in world.types.iter().enumerate() {
        if [
            "hknpPhysicsSystemData",
            "hknpBodyCinfo",
            "hknpPhysicsSystemData::bodyCinfoWithAttachment",
            "hknpCompressedMeshShape",
            "hknpCompressedMeshShapeData",
            "hknpCompressedMeshShapeTree",
            "hkcdStaticMeshTreeBase",
            "hkcdStaticMeshTreeBase::Connectivity",
            "hkcdSimdTree",
            "hknpConvexPolytopeShape",
            "hknpCompoundShape",
            "hknpShapeInstance",
        ]
        .iter()
        .any(|name| typ.name == *name)
        {
            println!(
                "\n[{index}] {}: size={} parent={} flags={:#x} subtype={:#x} pointer={}",
                typ.name, typ.byte_size, typ.parent, typ.flags, typ.subtype, typ.pointer
            );
            for field in &typ.fields {
                println!(
                    "  +{:#x} {}: [{}] {} flags={:#x}",
                    field.offset,
                    field.name,
                    field.type_index,
                    world.types[field.type_index].name,
                    field.flags
                );
            }
        }
    }
    for &index in &[
        67usize, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 124, 125, 127, 143, 144, 145,
        146, 147, 148, 149, 157, 163, 164, 167, 171, 181, 182, 183, 190, 192,
    ] {
        let typ = &world.types[index];
        println!(
            "TYPE {index} {} parent={} size={} subtype={:#x} pointer={}:",
            typ.name, typ.parent, typ.byte_size, typ.subtype, typ.pointer
        );
        for field in &typ.fields {
            println!(
                "  +{:#x} {} -> [{}] {}",
                field.offset, field.name, field.type_index, world.types[field.type_index].name
            );
        }
    }
    for &index in &[4usize, 5, 16, 17, 18, 19, 20, 21, 22] {
        let Some(item) = world.items.get(index) else {
            continue;
        };
        let at = world.data_start + item.offset;
        let len = world.types[item.type_index]
            .byte_size
            .max(if index == 16 || index == 19 { 128 } else { 0 })
            .min(256)
            .min(world.data_size - item.offset);
        println!(
            "ITEM {index} {} count={} offset={:#x}: {}",
            world.types[item.type_index].name,
            item.count,
            item.offset,
            bytes[at..at + len]
                .chunks(16)
                .map(|line| line
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(" "))
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    for &index in &[18usize, 21, 22] {
        let item = &world.items[index];
        let at = world.data_start + item.offset;
        for offset in [0x40, 0x50, 0x54, 0x58, 0x60] {
            let reference = u32::from_le_bytes(bytes[at + offset..at + offset + 4].try_into()?);
            let Some(child) = world.items.get(reference as usize) else {
                continue;
            };
            println!("shape item {index} +{offset:#x} -> item {reference}: {} count={} offset={:#x} size={}",
                world.types[child.type_index].name, child.count, child.offset, world.types[child.type_index].byte_size);
            let data = world.data_start + child.offset;
            let length = (child.count * world.types[child.type_index].byte_size).min(64);
            println!(
                "  data: {}",
                bytes[data..data + length]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
    for shape_index in [712usize, 1484] {
        let Some(shape) = world.items.get(shape_index) else {
            continue;
        };
        if world.types[shape.type_index].name != "hknpCompressedMeshShape" {
            continue;
        }
        let at = world.data_start + shape.offset;
        let data_ref = u32::from_le_bytes(bytes[at + 0x80..at + 0x84].try_into()?);
        let data = &world.items[data_ref as usize];
        let data_at = world.data_start + data.offset;
        println!(
            "COMPRESSED item {shape_index} data item {data_ref} at {:#x}",
            data.offset
        );
        for offset in [0x60, 0x70, 0x80, 0x90, 0xa0, 0xb0] {
            let reference =
                u32::from_le_bytes(bytes[data_at + offset..data_at + offset + 4].try_into()?);
            if let Some(item) = world.items.get(reference as usize) {
                let len = (item.count * world.types[item.type_index].byte_size.max(1)).min(48);
                let start = world.data_start + item.offset;
                println!(
                    "  +{offset:#x} item {reference} {} count={} data={}",
                    world.types[item.type_index].name,
                    item.count,
                    bytes[start..start + len]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
        let section_ref = u32::from_le_bytes(bytes[data_at + 0x60..data_at + 0x64].try_into()?);
        let section = &world.items[section_ref as usize];
        let section_at = world.data_start + section.offset;
        for number in 0..section.count.min(2) {
            let at = section_at + number * world.types[section.type_index].byte_size;
            let first_packed = u32::from_le_bytes(bytes[at + 0x48..at + 0x4c].try_into()?);
            let shared = u32::from_le_bytes(bytes[at + 0x4c..at + 0x50].try_into()?);
            let primitive = u32::from_le_bytes(bytes[at + 0x50..at + 0x54].try_into()?);
            println!("  section {number}: firstPacked={first_packed} shared={shared:#x} primitive={primitive:#x} numPacked={} numShared={}", bytes[at+0x58], bytes[at+0x59]);
        }
        let shared_index_ref =
            u32::from_le_bytes(bytes[data_at + 0x80..data_at + 0x84].try_into()?);
        let shared_ref = u32::from_le_bytes(bytes[data_at + 0xa0..data_at + 0xa4].try_into()?);
        let indices = &world.items[shared_index_ref as usize];
        let shared_count = world.items[shared_ref as usize].count;
        let at = world.data_start + indices.offset;
        let values: Vec<usize> = (0..indices.count)
            .map(|i| {
                u16::from_le_bytes(bytes[at + i * 2..at + i * 2 + 2].try_into().unwrap()) as usize
            })
            .collect();
        for (label, f) in [
            ("raw", (|v: usize| v) as fn(usize) -> usize),
            ("shift4", |v: usize| v >> 4),
            ("mask11", |v: usize| v & 0x7ff),
            ("mask12", |v: usize| v & 0xfff),
        ] {
            let invalid = values.iter().filter(|&&v| f(v) >= shared_count).count();
            let max = values.iter().map(|&v| f(v)).max().unwrap_or(0);
            println!("  shared indices {label}: invalid={invalid}/{} max={max} shared_count={shared_count}", values.len());
        }
        let section_at = world.data_start + section.offset;
        let get_float = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let global_min: [f32; 3] = std::array::from_fn(|axis| get_float(data_at + 0x30 + axis * 4));
        let global_max: [f32; 3] = std::array::from_fn(|axis| get_float(data_at + 0x40 + axis * 4));
        let sec_min: [f32; 3] = std::array::from_fn(|axis| get_float(section_at + 0x10 + axis * 4));
        let sec_max: [f32; 3] = std::array::from_fn(|axis| get_float(section_at + 0x20 + axis * 4));
        let shared_at = world.data_start + world.items[shared_ref as usize].offset;
        for (label, f) in [
            ("shift4", (|v: usize| v >> 4) as fn(usize) -> usize),
            ("mask11", |v: usize| v & 0x7ff),
        ] {
            let outside = values
                .iter()
                .filter(|&&v| {
                    let index = f(v);
                    let encoded = u64::from_le_bytes(
                        bytes[shared_at + index * 8..shared_at + index * 8 + 8]
                            .try_into()
                            .unwrap(),
                    );
                    let xyz = [
                        global_min[0]
                            + (encoded & 0x1f_ffff) as f32 / 0x1f_ffff as f32
                                * (global_max[0] - global_min[0]),
                        global_min[1]
                            + ((encoded >> 21) & 0x1f_ffff) as f32 / 0x1f_ffff as f32
                                * (global_max[1] - global_min[1]),
                        global_min[2]
                            + ((encoded >> 42) & 0x3f_ffff) as f32 / 0x3f_ffff as f32
                                * (global_max[2] - global_min[2]),
                    ];
                    (0..3).any(|axis| {
                        xyz[axis] < sec_min[axis] - 0.1 || xyz[axis] > sec_max[axis] + 0.1
                    })
                })
                .count();
            println!(
                "  {label} decoded outside section AABB: {outside}/{}",
                values.len()
            );
        }
    }
    for (index, item) in world.items.iter().enumerate() {
        let name = &world.types[item.type_index].name;
        if name == "hknpPhysicsSystemData"
            || name == "hknpPhysicsSystemData::bodyCinfoWithAttachment"
        {
            let at = world.data_start + item.offset;
            println!(
                "\nITEM {index}: {name} offset={:#x} count={} flags={:#x}",
                item.offset, item.count, item.flags
            );
            if name == "hknpPhysicsSystemData" {
                for offset in [0x18, 0x28, 0x38, 0x48, 0x58, 0x68] {
                    let reference =
                        u32::from_le_bytes(bytes[at + offset..at + offset + 4].try_into()?);
                    println!(
                        "  +{offset:#x} -> item {reference} ({})",
                        world
                            .items
                            .get(reference as usize)
                            .map(|item| world.types[item.type_index].name.as_str())
                            .unwrap_or("missing")
                    );
                }
            } else {
                for body in 0..item.count {
                    let base = at + body * world.types[item.type_index].byte_size;
                    let shape = u32::from_le_bytes(bytes[base..base + 4].try_into()?);
                    let name = u32::from_le_bytes(bytes[base + 0x18..base + 0x1c].try_into()?);
                    let pos: Vec<f32> = (0..3)
                        .map(|axis| {
                            f32::from_le_bytes(
                                bytes[base + 0x30 + axis * 4..base + 0x34 + axis * 4]
                                    .try_into()
                                    .unwrap(),
                            )
                        })
                        .collect();
                    let orientation: Vec<f32> = (0..4)
                        .map(|axis| {
                            f32::from_le_bytes(
                                bytes[base + 0x40 + axis * 4..base + 0x44 + axis * 4]
                                    .try_into()
                                    .unwrap(),
                            )
                        })
                        .collect();
                    println!("  body {body}: shape={shape} ({}) name={name} position={pos:?} orientation={orientation:?}", world.items.get(shape as usize).map(|item| world.types[item.type_index].name.as_str()).unwrap_or("missing"));
                }
            }
        }
    }
    Ok(())
}

fn inspect_compressed_shape(
    world: &physworld::PhysWorld,
    bytes: &[u8],
    shape_index: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let le32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let stride = |mut typ: usize| -> usize {
        loop {
            let info = &world.types[typ];
            if info.byte_size != 0 {
                return info.byte_size;
            }
            if info.parent == 0 {
                return 0;
            }
            typ = info.parent;
        }
    };
    let shape = &world.items[shape_index];
    let at = world.data_start + shape.offset;
    let data_index = le32(at + 0x80);
    let data = &world.items[data_index];
    let data_at = world.data_start + data.offset;
    let floats = |base: usize, count: usize| -> Vec<f32> {
        (0..count)
            .map(|i| f32::from_le_bytes(bytes[base + i * 4..base + i * 4 + 4].try_into().unwrap()))
            .collect()
    };
    println!(
        "  data floats +0x10..0x50: {:?}",
        floats(data_at + 0x10, 16)
    );
    let radius = f32::from_le_bytes(bytes[at + 0x28..at + 0x2c].try_into()?);
    println!("shape {shape_index} data item {data_index} triangles={} convexes={} primitiveKeys={} bitsPerKey={} flatConvex={} convexRadius={radius}",
        le32(at + 0xa0), le32(at + 0xa4), le32(data_at + 0x50), le32(data_at + 0x54), bytes[data_at + 0x5c]);
    let refs: Vec<_> = [0x60, 0x70, 0x80, 0x90, 0xa0]
        .iter()
        .map(|offset| le32(data_at + offset))
        .collect();
    for (offset, &item_index) in [0x60, 0x70, 0x80, 0x90, 0xa0].iter().zip(&refs) {
        let item = &world.items[item_index];
        let typ = &world.types[item.type_index];
        println!(
            "+{offset:#x}: item {item_index} {} count={} stride={} offset={:#x}",
            typ.name,
            item.count,
            stride(item.type_index),
            item.offset
        );
    }
    let sections = &world.items[refs[0]];
    let section_stride = stride(sections.type_index);
    for section_number in 0..sections.count {
        let section = world.data_start + sections.offset + section_number * section_stride;
        println!(
            "  section floats +0x10..0x48: {:?}",
            floats(section + 0x10, 14)
        );
        let first_shared = le32(section + 0x4c) >> 8;
        let count = bytes[section + 0x59] as usize;
        let indices = &world.items[refs[2]];
        let vertex_count = world.items[refs[4]].count;
        let index_stride = stride(indices.type_index);
        let values: Vec<_> = (first_shared..first_shared + count.min(24))
            .map(|i| {
                let at = world.data_start + indices.offset + i * index_stride;
                u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
            })
            .collect();
        println!("section {section_number} shared start={first_shared} count={count} vertex_count={vertex_count} indices={values:?}");
        let primitive_item = &world.items[refs[1]];
        let first_primitive = le32(section + 0x50) >> 8;
        let primitive_count = le32(section + 0x50) & 0xff;
        let primitive_at = world.data_start
            + primitive_item.offset
            + first_primitive * stride(primitive_item.type_index);
        let end = primitive_at + (primitive_count * stride(primitive_item.type_index)).min(48);
        let all_primitive_bytes = &bytes
            [primitive_at..primitive_at + primitive_count * stride(primitive_item.type_index)];
        let max_index = all_primitive_bytes.iter().copied().max().unwrap_or(0);
        let first_out_of_range = all_primitive_bytes
            .chunks_exact(4)
            .enumerate()
            .find(|(_, primitive)| {
                primitive
                    .iter()
                    .any(|&index| index as usize >= bytes[section + 0x58] as usize + count)
            })
            .map(|(index, primitive)| (index, primitive.to_vec()));
        println!(
            "  packed={} primitive start={first_primitive} count={primitive_count} bytes={:?}",
            bytes[section + 0x58],
            &bytes[primitive_at..end]
        );
        println!("  max_index={max_index} first_out_of_range={first_out_of_range:?}");
        let raw = &world.items[refs[4]];
        let shared_at = world.data_start + raw.offset;
        let word_count = (raw.count * 2).min(24);
        let words: Vec<u32> = (0..word_count)
            .map(|i| le32(shared_at + i * 4) as u32)
            .collect();
        println!("  shared words={words:?}");
        let bounds = floats(section + 0x10, 8);
        for block in 0..count / 2 {
            let h = values.get(block * 2).copied().unwrap_or_else(|| {
                let at =
                    world.data_start + indices.offset + (first_shared + block * 2) * index_stride;
                u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
            });
            let start = if let Some(&v) = values.get(block * 2 + 1) {
                v
            } else {
                let at = world.data_start
                    + indices.offset
                    + (first_shared + block * 2 + 1) * index_stride;
                u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
            };
            let n = (h >> 8) as usize;
            let method = (h >> 4) & 3;
            let mut low = [f32::INFINITY; 3];
            let mut high = [f32::NEG_INFINITY; 3];
            if method == 1 {
                for k in 0..n {
                    let v = le32(shared_at + (start as usize * 2 + k) * 4) as u32;
                    let p = [
                        (v & 0x7ff) as f32 / 2047.0,
                        ((v >> 11) & 0x7ff) as f32 / 2047.0,
                        (v >> 22) as f32 / 1023.0,
                    ];
                    for axis in 0..3 {
                        let x = bounds[axis] + p[axis] * (bounds[axis + 4] - bounds[axis]);
                        low[axis] = low[axis].min(x);
                        high[axis] = high[axis].max(x);
                    }
                }
            }
            println!("    block {block}: header={h:#x} start={start} count={n} method={method} decodedBounds={low:?}..{high:?}");
        }
    }
    let collision = world.collision_shapes(bytes)?;
    let prefix = format!("compressed shape {shape_index} ");
    for error in &collision.compressed_decode_failures {
        if error.starts_with(&prefix) {
            println!("decode failure: {error}");
        }
    }
    Ok(())
}
