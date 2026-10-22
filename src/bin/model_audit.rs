use dishonored2_rust::archive;
use dishonored2_rust::bmd6;
use dishonored2_rust::bmodel;
use dishonored2_rust::entities;
use dishonored2_rust::material;
use dishonored2_rust::physworld;
use dishonored2_rust::texture;
use std::collections::HashMap;
use std::error::Error;
use std::fs;

// Diagnostic geometry only: use the same authored column basis as the client.
fn placed_vertex(placement: &entities::PlacedModel, local: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        placement.position[row]
            + (0..3)
                .map(|column| {
                    placement.rotation[column][row] * placement.scale[column] * local[column]
                })
                .sum::<f32>()
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let sample = fs::read("artifacts/life_preserver_01.bmodel")?;
    let parsed = bmodel::parse(&sample)?;
    println!(
        "sample: {} surfaces; {} vertices; {} triangles",
        parsed.surfaces.len(),
        parsed
            .surfaces
            .iter()
            .map(|s| s.positions.len())
            .sum::<usize>(),
        parsed
            .surfaces
            .iter()
            .map(|s| s.indices.len() / 3)
            .sum::<usize>()
    );
    let base_path = dishonored2_rust::assets::installation::game_base()?;
    let base = base_path.as_path();
    let catalog = archive::Catalog::open(base)?;
    let mut args = std::env::args().skip(1);
    let entity_path = args
        .next()
        .unwrap_or_else(|| "artifacts/tutorial_p.entities".into());
    let filter = args.next();
    if entity_path == "--navmesh-audit" {
        let path = filter.ok_or("--navmesh-audit needs a catalog path")?;
        let bytes = catalog.read(&path)?.ok_or("catalog path not found")?;
        let file = physworld::parse_embedded_havok(&bytes)?;
        println!(
            "{} types, {} items, SDK {}; inventory {:?}",
            file.types.len(),
            file.items.len(),
            file.sdk_version,
            file.item_inventory()
        );
        for ty in &file.types {
            if ty.name == "hkaiNavMesh"
                || ty.name == "hkaiNavMesh::Face"
                || ty.name == "hkaiNavMesh::Edge"
            {
                println!(
                    "type {} size={} fields={:?}",
                    ty.name,
                    ty.byte_size,
                    ty.fields
                        .iter()
                        .map(|field| (&field.name, field.offset))
                        .collect::<Vec<_>>()
                );
            }
        }
        for (index, item) in file.items.iter().enumerate().take(20) {
            println!(
                "item {index}: {} count={} offset={}",
                file.types[item.type_index].name, item.count, item.offset
            );
        }
        if let Some(point) = args.next() {
            let coordinates = point
                .split(',')
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()?;
            if coordinates.len() != 3 {
                return Err("navmesh point needs x,y,z".into());
            }
            let item_of = |name: &str| {
                file.items
                    .iter()
                    .find(|item| file.types[item.type_index].name == name)
                    .ok_or_else(|| format!("navmesh has no {name} item"))
            };
            let [faces, edges, vertices] = [
                item_of("hkaiNavMesh::Face")?,
                item_of("hkaiNavMesh::Edge")?,
                item_of("hkVector4")?,
            ];
            let mut nearby = 0usize;
            for number in 0..faces.count {
                let at = file.data_start + faces.offset + number * 16;
                let first = u32::from_le_bytes(bytes[at..at + 4].try_into()?) as usize;
                let count = u16::from_le_bytes(bytes[at + 8..at + 10].try_into()?) as usize;
                if first + count > edges.count {
                    return Err("navmesh face exceeds edge table".into());
                }
                let mut low = [f32::INFINITY; 3];
                let mut high = [f32::NEG_INFINITY; 3];
                for edge in first..first + count {
                    let at = file.data_start + edges.offset + edge * 20;
                    let vertex = u32::from_le_bytes(bytes[at..at + 4].try_into()?) as usize;
                    if vertex >= vertices.count {
                        return Err("navmesh edge exceeds vertex table".into());
                    }
                    let at = file.data_start + vertices.offset + vertex * 16;
                    for axis in 0..3 {
                        let value =
                            f32::from_le_bytes(bytes[at + axis * 4..at + axis * 4 + 4].try_into()?);
                        low[axis] = low[axis].min(value);
                        high[axis] = high[axis].max(value);
                    }
                }
                let gap = (0..3)
                    .map(|axis| {
                        (coordinates[axis] - coordinates[axis].clamp(low[axis], high[axis])).powi(2)
                    })
                    .sum::<f32>()
                    .sqrt();
                if gap < 2.0 {
                    nearby += 1;
                    if nearby <= 12 {
                        println!("near nav face {number} edges={count} bounds={low:?}..{high:?} gap={gap}");
                    }
                }
            }
            println!("Navmesh faces with AABB within 2m of {coordinates:?}: {nearby}");
        }
        return Ok(());
    }
    if entity_path == "--hkx-audit" {
        let path = filter.ok_or("--hkx-audit needs a catalog path")?;
        let bytes = catalog.read(&path)?.ok_or("catalog path not found")?;
        let file = physworld::parse_hkx(&bytes)?;
        if args.next().as_deref() == Some("--schema") {
            for (index, ty) in file.types.iter().enumerate() {
                if ty.name.contains("Compressed")
                    || ty.name.contains("StaticTree")
                    || ty.name.contains("StaticMesh")
                    || ty.name == "hkAabb"
                {
                    println!(
                        "TYPE {index} {} size={} parent={} fields={:?}",
                        ty.name,
                        ty.byte_size,
                        ty.parent,
                        ty.fields
                            .iter()
                            .map(|field| (
                                &field.name,
                                field.offset,
                                &file.types[field.type_index].name
                            ))
                            .collect::<Vec<_>>()
                    );
                }
            }
            return Ok(());
        }
        println!(
            "{} types, {} items, SDK {}; inventory {:?}",
            file.types.len(),
            file.items.len(),
            file.sdk_version,
            file.item_inventory()
        );
        for (index, item) in file.items.iter().enumerate().take(8) {
            println!(
                "item {index}: {} count={} offset={}",
                file.types[item.type_index].name, item.count, item.offset
            );
        }
        let root_type = &file.types[file.items[1].type_index];
        println!(
            "root size={} fields={:?}",
            root_type.byte_size,
            root_type
                .fields
                .iter()
                .map(|field| (&field.name, field.offset))
                .collect::<Vec<_>>()
        );
        let mut ty = file.items[1].type_index;
        while ty != 0 {
            let entry = &file.types[ty];
            println!(
                "  {} size={} fields={:?}",
                entry.name,
                entry.byte_size,
                entry
                    .fields
                    .iter()
                    .map(|field| (&field.name, field.offset))
                    .collect::<Vec<_>>()
            );
            ty = entry.parent;
        }
        let mut ty = file.items[4].type_index;
        while ty != 0 {
            let entry = &file.types[ty];
            println!(
                "  convex {} size={} fields={:?}",
                entry.name,
                entry.byte_size,
                entry
                    .fields
                    .iter()
                    .map(|field| (&field.name, field.offset))
                    .collect::<Vec<_>>()
            );
            ty = entry.parent;
        }
        let mut ty = file.items[2].type_index;
        while ty != 0 {
            let entry = &file.types[ty];
            println!(
                "  instance {} size={} fields={:?}",
                entry.name,
                entry.byte_size,
                entry
                    .fields
                    .iter()
                    .map(|field| (&field.name, field.offset))
                    .collect::<Vec<_>>()
            );
            ty = entry.parent;
        }
        let collision = file.standalone_shapes(&bytes)?;
        println!(
            "{} convex shapes, {} compressed meshes, {} triangles, unknown={:?}, compressed_failures={:?}",
            collision.convex.len(),
            collision.compressed.len(),
            collision.triangles.len(),
            collision.unknown_shapes,
            collision.compressed_decode_failures
        );
        return Ok(());
    }
    if entity_path == "--tags" {
        let path = filter.ok_or("--tags needs a catalog path")?;
        let bytes = catalog.read(&path)?.ok_or("catalog path not found")?;
        for tag in [
            b"TAG0", b"SDKV", b"DATA", b"TYPE", b"TSTR", b"FSTR", b"TNAM", b"TBOD", b"INDX",
            b"ITEM", b"TCM0",
        ] {
            let offsets: Vec<_> = bytes
                .windows(4)
                .enumerate()
                .filter_map(|(at, word)| (word == tag).then_some(at))
                .collect();
            println!("{}: {offsets:?}", std::str::from_utf8(tag)?);
        }
        return Ok(());
    }
    if entity_path == "--hex" {
        let path = filter.ok_or("--hex needs a catalog path")?;
        let bytes = catalog.read(&path)?.ok_or("catalog path not found")?;
        println!(
            "{} bytes; prefix {:02x?}; suffix {:02x?}",
            bytes.len(),
            &bytes[..bytes.len().min(96)],
            &bytes[bytes.len().saturating_sub(32)..]
        );
        return Ok(());
    }
    if entity_path == "--read" {
        let path = filter.ok_or("--read needs a catalog path")?;
        let bytes = catalog.read(&path)?.ok_or("catalog path not found")?;
        println!("{} bytes\n{}", bytes.len(), String::from_utf8_lossy(&bytes));
        return Ok(());
    }
    if entity_path == "--model" {
        let path = filter.ok_or("--model needs a catalog path")?;
        let bytes = catalog.read(&path)?.ok_or("catalog path not found")?;
        let model = bmodel::parse(&bytes)?;
        for (index, surface) in model.surfaces.iter().enumerate() {
            let mut low = [f32::INFINITY; 3];
            let mut high = [f32::NEG_INFINITY; 3];
            for point in &surface.positions {
                for axis in 0..3 {
                    low[axis] = low[axis].min(point[axis]);
                    high[axis] = high[axis].max(point[axis]);
                }
            }
            println!(
                "surface {index}: {} vertices={} triangles={} bounds={low:?}..{high:?}",
                surface.material_path,
                surface.positions.len(),
                surface.indices.len() / 3
            );
        }
        return Ok(());
    }
    if entity_path == "--paths" {
        let term = filter.ok_or("--paths needs a path fragment")?;
        for path in catalog.paths_containing(&term) {
            println!("{path}");
        }
        return Ok(());
    }
    if entity_path == "--physics-assets" {
        for suffix in [".hkt", ".bphysworld", ".bphysics", ".hkx"] {
            let paths = catalog.paths_with_suffix(suffix);
            println!("{}: {} assets", suffix, paths.len());
            for path in paths
                .into_iter()
                .filter(|path| filter.as_ref().is_none_or(|term| path.contains(term)))
                .take(30)
            {
                println!("  {path}");
            }
        }
        return Ok(());
    }
    if entity_path == "--catalog" {
        let mut total = 0usize;
        let mut decoded = 0usize;
        let mut binding_mismatches = 0usize;
        let mut nonempty_tails = 0usize;
        let mut first_failures = Vec::new();
        for path in catalog
            .paths_with_suffix(".bmodel")
            .into_iter()
            .filter(|path| path.starts_with("generated/model/"))
        {
            total += 1;
            let Some(bytes) = catalog.read(&path)? else {
                continue;
            };
            match bmodel::parse(&bytes) {
                Ok(model) => {
                    decoded += 1;
                    if model.material_bindings.len() != model.surfaces.len()
                        || model
                            .material_bindings
                            .iter()
                            .zip(&model.surfaces)
                            .any(|(binding, surface)| binding != &surface.material_path)
                    {
                        binding_mismatches += 1;
                    }
                    if model.bytes_after_bindings != 0 {
                        nonempty_tails += 1;
                    }
                }
                Err(error) if first_failures.len() < 12 => first_failures.push((path, error)),
                Err(_) => {}
            }
        }
        println!("BML1 catalog: {decoded}/{total} parsed; {binding_mismatches} binding differences; {nonempty_tails} nonempty tails");
        for (path, error) in first_failures {
            println!("  {path}: {error}");
        }
        return Ok(());
    }
    let inspect_transparency = filter.as_deref() == Some("--transparent");
    if let Some(term) = filter.as_deref() {
        println!("CATALOG paths containing {term}:");
        for path in catalog
            .paths_containing(term)
            .into_iter()
            .filter(|path| {
                path.contains("generated/model/") || path.contains("generated/basemodel/")
            })
            .take(30)
        {
            println!("  {path}");
            if path.ends_with(".bmd6model") {
                if let Some(bytes) = catalog.read(&path)? {
                    let surface_count = bytes
                        .get(32..36)
                        .map(|raw| u32::from_le_bytes(raw.try_into().unwrap()));
                    let btri_offsets: Vec<_> = bytes
                        .windows(4)
                        .enumerate()
                        .filter_map(|(at, chunk)| (chunk == b"BTRI").then_some(at))
                        .collect();
                    println!("  BMD6 bytes={} signature={:02x?} surface_count={surface_count:?} BTRI={btri_offsets:?}",
                        bytes.len(), bytes.get(0..4).unwrap_or(&[]));
                    if let Some(&btri) = btri_offsets.first() {
                        println!(
                            "  BTRI first 36 payload bytes={:02x?}",
                            bytes.get(btri + 0x44..btri + 0x44 + 36).unwrap_or(&[])
                        );
                    }
                }
            }
        }
    }
    let data = fs::read(&entity_path)?;
    let doc = entities::parse(&String::from_utf8_lossy(&data))?;
    if filter.as_deref() == Some("--blueprint-audit") {
        let placements = entities::render_placements(&doc);
        let blueprints: Vec<_> = placements
            .iter()
            .filter(|placement| placement.resource_path.ends_with(".blueprint.decl"))
            .collect();
        let mut children = 0usize;
        let mut missing_models = 0usize;
        let mut failures = Vec::new();
        for placement in &blueprints {
            let Some(bytes) = catalog.read(&placement.resource_path)? else {
                failures.push(format!("missing declaration {}", placement.resource_path));
                continue;
            };
            match entities::expand_blueprint(placement, &String::from_utf8_lossy(&bytes)) {
                Ok(items) => {
                    children += items.len();
                    for item in items {
                        if !catalog.contains(&item.resource_path) {
                            missing_models += 1;
                        }
                    }
                }
                Err(error) => failures.push(format!("{}: {error}", placement.resource_path)),
            }
        }
        println!(
            "{} blueprint placements, {} child placements, {} missing compiled children, {} errors",
            blueprints.len(),
            children,
            missing_models,
            failures.len()
        );
        for error in failures.into_iter().take(20) {
            println!("  {error}");
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--blueprint-placements") {
        for placement in entities::render_placements(&doc)
            .into_iter()
            .filter(|item| item.resource_path.ends_with(".blueprint.decl"))
        {
            println!(
                "{} {:?} {}",
                placement.id, placement.position, placement.resource_path
            );
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--md6-audit") {
        let placements: Vec<_> = entities::render_placements(&doc)
            .into_iter()
            .filter(|item| item.resource_path.ends_with(".bmd6model"))
            .collect();
        let mut unique = std::collections::BTreeSet::new();
        let mut missing = 0;
        let mut unsupported = 0;
        let mut skeletal = 0;
        let mut renderable = 0;
        for item in &placements {
            if !unique.insert(&item.resource_path) {
                continue;
            }
            let mesh_path = bmd6::resolve_compiled_mesh(&item.resource_path, |decl| {
                catalog
                    .read(decl)
                    .ok()
                    .flatten()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            })?;
            match catalog.read(&mesh_path)? {
                None => {
                    missing += 1;
                    println!("missing {}", item.resource_path);
                }
                Some(bytes) => match bmd6::inspect(&bytes) {
                    Ok(model) if !model.has_decoded_geometry => {
                        unsupported += 1;
                        println!("geometry pending {mesh_path}");
                    }
                    Ok(model) if model.has_skeleton => {
                        skeletal += 1;
                        println!("rest-pose only {mesh_path}");
                    }
                    Ok(_) => renderable += 1,
                    Err(error) => {
                        unsupported += 1;
                        println!("unsupported {}: {error}", item.resource_path);
                    }
                },
            }
        }
        println!(
            "MD6 placements={} unique={} static={} skeletal-rest-pose={} unsupported={} missing={}",
            placements.len(),
            unique.len(),
            renderable,
            skeletal,
            unsupported,
            missing
        );
        return Ok(());
    }
    if filter.as_deref() == Some("--near") {
        let values = args
            .next()
            .ok_or("--near needs x,y,z,radius")?
            .split(',')
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 4 || !values.iter().all(|value| value.is_finite()) || values[3] < 0.0 {
            return Err("--near needs finite x,y,z and nonnegative radius".into());
        }
        let mut placements = entities::render_placements(&doc);
        let mut cursor = 0;
        while cursor < placements.len() {
            let placement = &placements[cursor];
            if placement.resource_path.ends_with(".blueprint.decl") {
                if placement.id.matches("#blueprint[").count() >= 16 {
                    return Err(format!("Blueprint nesting exceeds16: {}", placement.id).into());
                }
                let bytes = catalog
                    .read(&placement.resource_path)?
                    .ok_or("missing blueprint")?;
                placements.extend(entities::expand_blueprint(
                    placement,
                    &String::from_utf8_lossy(&bytes),
                )?);
            }
            cursor += 1;
        }
        let mut models = HashMap::new();
        let mut nearby = Vec::new();
        for placement in &placements {
            if !placement.resource_path.ends_with(".bmodel") {
                continue;
            }
            if !models.contains_key(&placement.resource_path) {
                let bytes = catalog
                    .read(&placement.resource_path)?
                    .ok_or("missing placed model")?;
                models.insert(placement.resource_path.clone(), bmodel::parse(&bytes)?);
            }
            let model = &models[&placement.resource_path];
            let mut low = [f32::INFINITY; 3];
            let mut high = [f32::NEG_INFINITY; 3];
            for local in model.surfaces.iter().flat_map(|surface| &surface.positions) {
                let point = placed_vertex(placement, *local);
                for axis in 0..3 {
                    low[axis] = low[axis].min(point[axis]);
                    high[axis] = high[axis].max(point[axis]);
                }
            }
            let gap = (0..3)
                .map(|axis| (values[axis] - values[axis].clamp(low[axis], high[axis])).powi(2))
                .sum::<f32>()
                .sqrt();
            if gap <= values[3] {
                nearby.push((gap, placement, low, high));
            }
        }
        nearby.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (gap, placement, low, high) in nearby {
            println!("NEAR gap={gap:.5} id={} bounds={low:?}..{high:?} position={:?} scale={:?} rotation={:?} model={}",
                placement.id, placement.position, placement.scale, placement.rotation, placement.resource_path);
        }
        return Ok(());
    }
    let placed = entities::placed_models(&doc);
    println!(
        "{} placed models; {} explicit maxVisibleRange",
        placed.len(),
        placed
            .iter()
            .filter(|item| item.max_visible_range.is_some())
            .count()
    );
    if filter.as_deref() == Some("--placements") {
        let term = args.next().ok_or("--placements needs a path fragment")?;
        for object in &placed {
            if object.resource_path.contains(&term) {
                println!(
                    "{} position={:?} scale={:?} rotation={:?} model={} overrides={:?} maxVisibleRange={:?}",
                    object.id,
                    object.position,
                    object.scale,
                    object.rotation,
                    object.resource_path,
                    object.custom_materials,
                    object.max_visible_range
                );
            }
        }
        return Ok(());
    }
    if filter.as_deref() == Some("--missing-textures") {
        let mut missing = std::collections::BTreeMap::<(String, String), usize>::new();
        let mut checked = std::collections::HashMap::<String, bool>::new();
        for object in &placed {
            let Some(bytes) = catalog.read(&object.resource_path)? else {
                continue;
            };
            let Ok(model) = bmodel::parse(&bytes) else {
                continue;
            };
            for (index, surface) in model.surfaces.iter().enumerate() {
                let path = object
                    .custom_materials
                    .get(index)
                    .and_then(Option::as_deref)
                    .unwrap_or(&surface.material_path);
                if material::is_null_material(path) {
                    continue;
                }
                let valid = if let Some(&valid) = checked.get(path) {
                    valid
                } else {
                    let decl = format!("generated/decls/material/{path}.decl");
                    let valid = if let Some(bytes) = catalog.read(&decl)? {
                        if let Some(image) = material::diffuse_image(&bytes) {
                            let stem = image.strip_suffix("_mip0").unwrap_or(&image);
                            if let Some(bytes) = catalog.read(&image)? {
                                texture::decode_bc7(&bytes).is_ok()
                            } else if let Some(bytes) = catalog.read(stem)? {
                                texture::decode_inline_bc7(&bytes).is_ok()
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    checked.insert(path.to_owned(), valid);
                    valid
                };
                if !valid {
                    *missing
                        .entry((object.resource_path.clone(), path.to_owned()))
                        .or_default() += 1;
                }
            }
        }
        println!(
            "{} unique placed-model materials checked; {} unresolved model/material pairs",
            checked.len(),
            missing.len()
        );
        for ((model, material), count) in missing {
            println!("{count:>4} {model} | {material}");
        }
        return Ok(());
    }
    let mut unique = HashMap::new();
    let mut binding_mismatches = Vec::new();
    let mut nonempty_tails = Vec::new();
    for object in placed {
        if unique.contains_key(&object.resource_path) {
            continue;
        }
        let state = match catalog.read(&object.resource_path)? {
            Some(data) => match bmodel::parse(&data) {
                Ok(model) => {
                    if model.material_bindings.len() != model.surfaces.len()
                        || model
                            .material_bindings
                            .iter()
                            .zip(&model.surfaces)
                            .any(|(binding, surface)| binding != &surface.material_path)
                    {
                        binding_mismatches.push(object.resource_path.clone());
                    }
                    if model.bytes_after_bindings != 0 {
                        nonempty_tails
                            .push((object.resource_path.clone(), model.bytes_after_bindings));
                    }
                    if filter
                        .as_ref()
                        .is_some_and(|term| object.resource_path.contains(term))
                    {
                        println!(
                            "BINDINGS {} trailing_bytes={} {:?}",
                            model.material_bindings.len(),
                            model.bytes_after_bindings,
                            model.material_bindings
                        );
                    }
                    if inspect_transparency {
                        for surface in &model.surfaces {
                            let decl =
                                format!("generated/decls/material/{}.decl", surface.material_path);
                            let Some(bytes) = catalog.read(&decl)? else {
                                continue;
                            };
                            let mode = material::transparency(&bytes);
                            if mode == material::Transparency::Opaque {
                                continue;
                            }
                            let mut lo = [f32::INFINITY; 3];
                            let mut hi = [f32::NEG_INFINITY; 3];
                            for vertex in &surface.positions {
                                for axis in 0..3 {
                                    lo[axis] = lo[axis].min(vertex[axis]);
                                    hi[axis] = hi[axis].max(vertex[axis]);
                                }
                            }
                            let extent = (0..3)
                                .map(|axis| hi[axis] - lo[axis])
                                .fold(0.0f32, f32::max);
                            if mode == material::Transparency::Blend && extent > 1.5 {
                                println!("LARGE_BLEND extent={extent:.2} id={} position={:?} model={} material={}",
                                    object.id, object.position, object.resource_path, surface.material_path);
                            }
                        }
                    }
                    if filter
                        .as_ref()
                        .is_some_and(|term| object.resource_path.contains(term))
                    {
                        println!("MODEL {}", object.resource_path);
                        for surface in &model.surfaces {
                            let mut lo = [f32::INFINITY; 3];
                            let mut hi = [f32::NEG_INFINITY; 3];
                            for vertex in &surface.positions {
                                for axis in 0..3 {
                                    lo[axis] = lo[axis].min(vertex[axis]);
                                    hi[axis] = hi[axis].max(vertex[axis]);
                                }
                            }
                            println!("  MATERIAL {} bounds={lo:?}..{hi:?}", surface.material_path);
                            let decl =
                                format!("generated/decls/material/{}.decl", surface.material_path);
                            if let Some(bytes) = catalog.read(&decl)? {
                                println!("  DECL {}", String::from_utf8_lossy(&bytes));
                            }
                        }
                    }
                    if model
                        .surfaces
                        .iter()
                        .any(|surface| surface.material_path.contains("models/templates/colors/"))
                    {
                        println!("TEMPLATE COLOR MODEL {}", object.resource_path);
                    }
                    format!("OK {} surfaces", model.surfaces.len())
                }
                Err(error) => format!("PARSE {error}"),
            },
            None => "MISSING".into(),
        };
        unique.insert(object.resource_path, state);
    }
    let ok = unique
        .values()
        .filter(|state| state.starts_with("OK"))
        .count();
    println!("{ok}/{} unique models parsed", unique.len());
    println!(
        "Binding mismatches: {} {:?}; nonempty tails: {} {:?}",
        binding_mismatches.len(),
        binding_mismatches.iter().take(5).collect::<Vec<_>>(),
        nonempty_tails.len(),
        nonempty_tails.iter().take(5).collect::<Vec<_>>()
    );
    let mut failures: Vec<_> = unique
        .into_iter()
        .filter(|(_, state)| !state.starts_with("OK"))
        .collect();
    failures.sort();
    for (path, state) in failures.into_iter().take(40) {
        println!("{state} {path}");
    }
    Ok(())
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn placed_point_uses_column_basis_and_signed_scale() {
        let placement = entities::PlacedModel {
            id: "test".into(),
            resource_path: String::new(),
            position: [10.0, 20.0, 30.0],
            rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            scale: [-2.0, 3.0, 4.0],
            custom_materials: Vec::new(),
            max_visible_range: None,
        };
        assert_eq!(
            placed_vertex(&placement, [1.0, 2.0, 3.0]),
            [4.0, 18.0, 42.0]
        );
    }
}
