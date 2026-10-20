//! Inspect the installed map's Umbra Tome without guessing its visibility rules.
use dishonored2_rust::tome;
use dishonored2_rust::world_mesh;
use std::{env, fs};
use tome::object_ids;

// FUN_140a765a0 searches the header-relative gate-ID array at +0x6c,
// whose count is the word at +0x68.
fn gate_ids(bytes: &[u8]) -> Result<Vec<u32>, String> {
    let word = |offset: usize| -> Result<u32, String> {
        let data = bytes
            .get(offset..offset.checked_add(4).ok_or("offset overflow")?)
            .ok_or("truncated Tome gate table")?;
        Ok(u32::from_le_bytes(data.try_into().unwrap()))
    };
    let count = word(0x68)? as usize;
    let offset = word(0x6c)? as usize;
    let end = offset
        .checked_add(count.checked_mul(4).ok_or("gate count overflow")?)
        .ok_or("gate table overflow")?;
    if bytes.get(offset..end).is_none() {
        return Err("Tome gate table exceeds file".into());
    }
    (0..count).map(|index| word(offset + index * 4)).collect()
}

// FUN_140a7c020 gives the query a header-relative u32 table at +0xa0,
// with +0x2c / 32 entries. FUN_140a7bbf0 reads entries as offsets to
// Tome spatial records; zero denotes no record for that index.
fn spatial_record_offsets(bytes: &[u8]) -> Result<Vec<u32>, String> {
    let word = |offset: usize| -> Result<u32, String> {
        let data = bytes
            .get(offset..offset.checked_add(4).ok_or("offset overflow")?)
            .ok_or("truncated Tome spatial table")?;
        Ok(u32::from_le_bytes(data.try_into().unwrap()))
    };
    let bit_count = word(0x2c)? as usize;
    if bit_count % 32 != 0 {
        return Err("Tome spatial count is not divisible by 32".into());
    }
    let count = bit_count / 32;
    let table = word(0xa0)? as usize;
    let end = table
        .checked_add(count.checked_mul(4).ok_or("spatial count overflow")?)
        .ok_or("spatial table overflow")?;
    if bytes.get(table..end).is_none() {
        return Err("Tome spatial table exceeds file".into());
    }
    let offsets = (0..count)
        .map(|index| word(table + index * 4))
        .collect::<Result<Vec<_>, _>>()?;
    if offsets
        .iter()
        .any(|&offset| offset != 0 && offset as usize >= bytes.len())
    {
        return Err("Tome spatial record offset exceeds file".into());
    }
    Ok(offsets)
}

fn spatial_bounds(bytes: &[u8], offset: u32) -> Result<[[f32; 3]; 2], String> {
    let at = offset as usize;
    let data = bytes
        .get(at..at.checked_add(24).ok_or("spatial bounds overflow")?)
        .ok_or("truncated Tome spatial bounds")?;
    let mut values = [0.0f32; 6];
    for (index, value) in values.iter_mut().enumerate() {
        *value = f32::from_le_bytes(data[index * 4..index * 4 + 4].try_into().unwrap());
    }
    if (0..3).any(|axis| {
        !values[axis].is_finite()
            || !values[axis + 3].is_finite()
            || values[axis] > values[axis + 3]
    }) {
        return Err("invalid Tome spatial bounds".into());
    }
    Ok([
        [values[0], values[1], values[2]],
        [values[3], values[4], values[5]],
    ])
}

fn verify_world_bounds(world: &world_mesh::WorldMesh) -> (usize, usize, f32) {
    let mut checked = 0;
    let mut escaped = 0;
    let mut largest_escape = 0.0f32;
    for object in &world.objects {
        let vertices = &world.meshes[object.mesh_index].positions;
        for index in object.matrix_start..object.matrix_end {
            checked += 1;
            let matrix = &world.matrices[index];
            let bounds = &world.visibility[index].bounds;
            let mut escape = 0.0f32;
            for vertex in vertices {
                for axis in 0..3 {
                    let base = axis * 4;
                    let point = matrix[base] * vertex[0]
                        + matrix[base + 1] * vertex[1]
                        + matrix[base + 2] * vertex[2]
                        + matrix[base + 3];
                    escape = escape
                        .max(bounds[0][axis] - point)
                        .max(point - bounds[1][axis]);
                }
            }
            largest_escape = largest_escape.max(escape);
            if escape > 0.02 {
                escaped += 1;
            }
        }
    }
    (checked, escaped, largest_escape)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let path = args.get(1).ok_or(
        "usage: tome-audit <extracted .tome> [matching .bwm] [--point x y z] [--slot-rows] [--verify-bounds]",
    )?;
    let bytes = fs::read(&path)?;
    let word = |offset: usize| -> Result<u32, String> {
        let range = bytes
            .get(offset..offset + 4)
            .ok_or("truncated Tome header")?;
        Ok(u32::from_le_bytes(range.try_into().unwrap()))
    };
    let signature = word(0)?;
    if signature != 0xd600_0012 {
        return Err(format!("unsupported Tome signature/version {signature:#x}").into());
    }
    if word(8)? as usize != bytes.len() {
        return Err("Tome header size differs from extracted resource length".into());
    }
    println!(
        "Tome {path}: {} bytes, version {}",
        bytes.len(),
        signature & 0xff
    );
    if args.iter().any(|argument| argument == "--slot-rows") {
        let rows = tome::SpatialCellSlotRows::parse(&bytes)?;
        let packed_words = rows
            .spatial_record_count()
            .checked_mul(rows.bit_count())
            .and_then(|bits| bits.checked_add(31))
            .ok_or("Tome spatial slot row size overflow")?
            / 32;
        println!(
            "FUN_140a7c020 slot-row descriptor: {} rows × {} bits, {} packed u32 words ({} bytes), data offset {:#x}",
            rows.spatial_record_count(),
            rows.bit_count(),
            packed_words,
            packed_words * 4,
            word(0x98)?,
        );
        return Ok(());
    }
    println!("Header words (offset:value):");
    for offset in (0x2c..=0x9c).step_by(4) {
        println!("  {offset:#04x}: {} ({:#x})", word(offset)?, word(offset)?);
    }
    let bounds: Vec<f32> = (0x14..=0x28)
        .step_by(4)
        .map(|offset| word(offset).map(f32::from_bits))
        .collect::<Result<_, _>>()?;
    println!("Header bounds: {:?} .. {:?}", &bounds[..3], &bounds[3..]);
    let ids = object_ids(&bytes)?;
    let unique = ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .len();
    println!(
        "Object IDs: {} entries, {} distinct; first 8: {:?}",
        ids.len(),
        unique,
        &ids[..ids.len().min(8)]
    );
    let gates = gate_ids(&bytes)?;
    println!(
        "Gate IDs: {} entries, {} distinct; first 8: {:?}",
        gates.len(),
        gates
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        &gates[..gates.len().min(8)]
    );
    let spatial = spatial_record_offsets(&bytes)?;
    let tree = tome::SpatialTree::parse(&bytes)?;
    if let Some(at) = args.iter().position(|arg| arg == "--query-aabb") {
        let values = (1..=6)
            .map(|axis| {
                args.get(at + axis)
                    .ok_or("--query-aabb requires six coordinates")
                    .and_then(|value| {
                        value
                            .parse::<f32>()
                            .map_err(|_| "invalid --query-aabb coordinate")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let query = [
            [values[0], values[1], values[2]],
            [values[3], values[4], values[5]],
        ];
        let selected = tree.record_leaves_intersecting_aabb(query);
        let exhaustive = (0..tree.node_count())
            .filter(|&index| {
                if tree.is_leaf(index) != Some(true) || tree.record_offset(index) == Some(0) {
                    return false;
                }
                let bounds = tree.bounds(index).unwrap();
                (0..3).all(|axis| {
                    bounds[0][axis] <= query[1][axis] && query[0][axis] <= bounds[1][axis]
                })
            })
            .collect::<Vec<_>>();
        let mut selected_sorted = selected.clone();
        selected_sorted.sort_unstable();
        if selected_sorted != exhaustive {
            return Err("AABB tree walk differs from exhaustive leaf scan".into());
        }
        println!(
            "AABB query prefilter: {} spatial record leaves; matches exhaustive scan",
            selected.len()
        );
    }
    if let Some(at) = args.iter().position(|arg| arg == "--query-quad") {
        let values = (1..=12)
            .map(|axis| {
                args.get(at + axis)
                    .ok_or("--query-quad requires four xyz corners")
                    .and_then(|value| {
                        value
                            .parse::<f32>()
                            .map_err(|_| "invalid --query-quad coordinate")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let quad =
            std::array::from_fn(|corner| std::array::from_fn(|axis| values[corner * 3 + axis]));
        let selected = tree.record_leaves_intersecting_quad(quad);
        let exhaustive = (0..tree.node_count())
            .filter(|&index| {
                tree.is_leaf(index) == Some(true)
                    && tree.record_offset(index) != Some(0)
                    && tome::quad_intersects_aabb(tree.bounds(index).unwrap(), quad)
            })
            .collect::<Vec<_>>();
        let mut selected_sorted = selected.clone();
        selected_sorted.sort_unstable();
        if selected_sorted != exhaustive {
            return Err("quad tree walk differs from exhaustive leaf scan".into());
        }
        println!(
            "Four-corner query prefilter: {} spatial record leaves; matches exhaustive scan",
            selected.len()
        );
    }
    if tree.node_count() != spatial.len()
        || tree.record_count() != spatial.iter().filter(|&&offset| offset != 0).count()
        || spatial
            .iter()
            .enumerate()
            .any(|(index, &offset)| tree.record_offset(index) != Some(offset))
    {
        return Err("Tome spatial tree disagrees with offset table".into());
    }
    let validated_spatial_bounds = spatial
        .iter()
        .filter(|&&offset| offset != 0)
        .map(|&offset| spatial_bounds(&bytes, offset))
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "Spatial record offset table: {} entries, {} nonzero; first 8: {:?}",
        spatial.len(),
        validated_spatial_bounds.len(),
        &spatial[..spatial.len().min(8)]
    );
    if args.iter().any(|arg| {
        arg == "--verify-cell-trees"
            || arg == "--verify-portal-codes"
            || arg == "--verify-traversal-records"
            || arg == "--verify-object-candidates"
            || arg == "--verify-gate-links"
    }) {
        let verify_portals = args.iter().any(|arg| arg == "--verify-portal-codes");
        let verify_traversal = args.iter().any(|arg| arg == "--verify-traversal-records");
        let verify_candidates = args.iter().any(|arg| arg == "--verify-object-candidates");
        let verify_gates = args.iter().any(|arg| arg == "--verify-gate-links");
        let initial_gates = if verify_gates {
            Some(tome::initial_gate_states(&bytes)?)
        } else {
            None
        };
        let mut leaf_trees = 0usize;
        let mut local_cells = 0usize;
        let mut maximum_record_cells = (0usize, 0usize);
        let mut local_nodes = 0usize;
        let mut traversal_records = 0usize;
        let mut candidate_count = 0usize;
        let mut streamed_candidate_count = 0usize;
        let mut traversal_classes = (0usize, 0usize, 0usize);
        let mut direct_axes = [0usize; 4];
        let mut inverted_spans = 0usize;
        let mut gated_links = 0usize;
        let mut valid_destinations = 0usize;
        let mut flagged_faces = [0usize; 8];
        let mut flagged_gates = 0usize;
        let mut flagged_examples = Vec::new();
        let mut gate_links = 0usize;
        let mut gate_references = 0usize;
        let mut code_counts = (0usize, 0usize, 0usize);
        let mut portal_results = (0usize, 0usize);
        let mut portal_errors = std::collections::BTreeMap::<String, usize>::new();
        let mut internal_records = 0usize;
        let mut unexpected = std::collections::BTreeMap::<String, usize>::new();
        for (index, &offset) in spatial
            .iter()
            .enumerate()
            .filter(|(_, &offset)| offset != 0)
        {
            match (
                tree.is_leaf(index).unwrap(),
                tome::CellTree::parse(&bytes, offset),
            ) {
                (is_leaf, Ok(cells)) => {
                    if is_leaf {
                        leaf_trees += 1;
                    } else {
                        internal_records += 1;
                    }
                    local_cells += cells.cell_count();
                    if cells.cell_count() > maximum_record_cells.1 {
                        maximum_record_cells = (index, cells.cell_count());
                    }
                    for cell_index in 0..cells.cell_count() {
                        let decoded = cells
                            .cell_bounds(cell_index)
                            .ok_or("missing decoded cell bounds")?;
                        let parent = tree.bounds(index).ok_or("missing spatial record bounds")?;
                        if (0..3).any(|axis| {
                            !decoded[0][axis].is_finite()
                                || !decoded[1][axis].is_finite()
                                || decoded[0][axis] < parent[0][axis] - 0.01
                                || decoded[1][axis] > parent[1][axis] + 0.01
                        }) {
                            return Err(format!("cell {cell_index} in spatial record {index} has invalid decoded bounds").into());
                        }
                    }
                    local_nodes += cells.node_count();
                    traversal_records += cells.traversal_record_count();
                    candidate_count += cells.candidate_count();
                    if verify_candidates {
                        for cell_index in 0..cells.cell_count() {
                            cells.for_each_object_candidate_batch(&bytes, cell_index, |batch| {
                                streamed_candidate_count = streamed_candidate_count
                                    .checked_add(batch.len())
                                    .ok_or("streamed Tome candidate count overflow")?;
                                Ok(())
                            })?;
                        }
                    }
                    if verify_traversal || verify_gates {
                        for cell_index in 0..cells.cell_count() {
                            for entry in cells.traversal_records(&bytes, cell_index)? {
                                if verify_traversal {
                                    if let Some(target) = tome::direct_spatial_link_target(entry) {
                                        if (target as usize) < tree.node_count() {
                                            traversal_classes.0 += 1;
                                            tree.direct_link_destination(&bytes, entry)?.ok_or(
                                                "direct link lost its spatial destination",
                                            )?;
                                            valid_destinations += 1;
                                        } else {
                                            traversal_classes.2 += 1;
                                        }
                                        if let Some(geometry) = tome::packed_link_geometry(entry) {
                                            if geometry.destination
                                                != tome::LinkDestination::Spatial(target)
                                            {
                                                return Err(
                                                    "ordinary link has non-spatial destination"
                                                        .into(),
                                                );
                                            }
                                            tome::packed_link_query_bounds(
                                                &bytes, offset, geometry,
                                            )?;
                                            direct_axes[geometry.axis as usize] += 1;
                                            if geometry.spans.iter().any(|span| span[0] > span[1]) {
                                                inverted_spans += 1;
                                            }
                                        } else {
                                            gated_links += 1;
                                            tome::gated_link_query_bounds(&bytes, offset, entry)?
                                                .ok_or("gated link missing table geometry")?;
                                        }
                                    } else {
                                        traversal_classes.1 += 1;
                                        flagged_faces[(entry[0] >> 29) as usize] += 1;
                                        if entry[0] & (1 << 27) != 0 {
                                            flagged_gates += 1;
                                        }
                                        if let Some(geometry) = tome::packed_link_geometry(entry) {
                                            if geometry.destination
                                                != tome::LinkDestination::BoundaryFace(
                                                    (entry[0] >> 29) as u8,
                                                )
                                                || geometry
                                                    .spans
                                                    .iter()
                                                    .any(|span| span[0] > span[1])
                                            {
                                                return Err(
                                                    "invalid flagged packed-link geometry".into()
                                                );
                                            }
                                            tome::packed_link_query_bounds(
                                                &bytes, offset, geometry,
                                            )?;
                                        }
                                        if flagged_examples.len() < 6 {
                                            flagged_examples.push((index, cell_index, entry));
                                        }
                                    }
                                }
                                if verify_gates && entry[0] & (1 << 27) != 0 {
                                    gate_links += 1;
                                    gate_references +=
                                        tome::gate_indices_for_link(&bytes, entry)?.len();
                                    if !tome::gate_link_allows(
                                        &bytes,
                                        entry,
                                        initial_gates.as_deref(),
                                    )? {
                                        return Err(
                                            "Tome gate link fails with original initial state"
                                                .into(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    let counts = cells.code_counts();
                    code_counts.0 += counts.0;
                    code_counts.1 += counts.1;
                    code_counts.2 += counts.2;
                    if verify_portals {
                        for point in cells.portal_leaf_centers() {
                            match cells.cell_for_point(&bytes, point) {
                                Ok(Some(_)) => portal_results.0 += 1,
                                Ok(None) => portal_results.1 += 1,
                                Err(error) => *portal_errors.entry(error).or_default() += 1,
                            }
                        }
                    }
                }
                (is_leaf, result) => {
                    *unexpected
                        .entry(format!("leaf={is_leaf}, result={result:?}"))
                        .or_default() += 1;
                }
            }
        }
        println!("Local Tome cell records: {leaf_trees} leaf trees, {internal_records} internal records, {local_nodes} local nodes, {local_cells} local cells, max record {} has {} cells, {} unexpected layouts",
            maximum_record_cells.0, maximum_record_cells.1,
            unexpected.values().sum::<usize>());
        println!("Cell traversal records: {traversal_records} total");
        if verify_candidates {
            if streamed_candidate_count != candidate_count {
                return Err(format!(
                    "streamed {streamed_candidate_count} Tome candidates, expected {candidate_count}"
                )
                .into());
            }
            println!("Decoded Tome object candidates: {streamed_candidate_count} across {local_cells} cells in batches up to {}; all runs and object indices valid", tome::OBJECT_QUERY_BATCH_SIZE);
        }
        if verify_traversal {
            println!("Traversal record classes: {} normal spatial references, {} flagged, {} out-of-range normal references",
                traversal_classes.0, traversal_classes.1, traversal_classes.2);
            println!("Direct-link destination cell indices validated against target tables: {valid_destinations}");
            println!("Ungated packed geometry axes: {direct_axes:?}; gated table-geometry links: {gated_links}; inverted ungated spans: {inverted_spans}");
            println!("Flagged link face codes: {flagged_faces:?}; gated flagged: {flagged_gates}; first six (spatial record, cell, words): {flagged_examples:?}");
        }
        if verify_gates {
            println!("Gate-conditioned traversal links: {gate_links}, indexed gate references: {gate_references}; all ranges and gate indices valid, all pass original all-set initial gate state");
        }
        println!(
            "Local leaf cell codes: {} direct, {} portal, {} empty",
            code_counts.0, code_counts.1, code_counts.2
        );
        if verify_portals {
            println!("Portal leaf-center lookups: {} cells, {} empty, {} errors; errors {portal_errors:?}",
                portal_results.0, portal_results.1, portal_errors.values().sum::<usize>());
        }
        if !unexpected.is_empty() {
            return Err(format!("unexpected Tome cell-tree layouts: {unexpected:?}").into());
        }
    }
    let mut point_candidates = None;
    if let Some(at) = args.iter().position(|arg| arg == "--point") {
        let point = (1..=3)
            .map(|axis| {
                args.get(at + axis)
                    .ok_or("--point requires x y z")
                    .and_then(|value| {
                        value
                            .parse::<f32>()
                            .map_err(|_| "invalid --point coordinate")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut containing = Vec::new();
        for (index, &offset) in spatial.iter().enumerate() {
            if offset == 0 {
                continue;
            }
            let bounds = spatial_bounds(&bytes, offset)?;
            if (0..3).all(|axis| bounds[0][axis] <= point[axis] && point[axis] <= bounds[1][axis]) {
                containing.push((index, bounds));
            }
        }
        println!(
            "Tome spatial bounds containing point {point:?}: {} records",
            containing.len()
        );
        let query_records = tree.record_leaves_containing_point([point[0], point[1], point[2]]);
        println!(
            "Type-2 geometric point-containment prefilter: {} authored spatial records {:?} (not the full visibility result)",
            query_records.len(),
            query_records
                .iter()
                .map(|&index| (index, tree.record_offset(index)))
                .collect::<Vec<_>>()
        );
        if let Some(leaf) = tree.leaf_for_point([point[0], point[1], point[2]]) {
            let bounds = tree.bounds(leaf).unwrap();
            println!("Global split-tree leaf: index {leaf}, record offset {:?}, bounds {:?} .. {:?}",
                tree.record_offset(leaf), bounds[0], bounds[1]);
            if let Some(offset) = tree.record_offset(leaf).filter(|&offset| offset != 0) {
                let (center, tolerance) = tome::direct_link_query_quantization(
                    &bytes,
                    offset,
                    [point[0], point[1], point[2]],
                )?;
                println!("Direct-Tome link query if point is the camera position: quantized center {center:?}, tolerance {tolerance:?}");
                let cells = tome::CellTree::parse(&bytes, offset)?;
                println!(
                    "Local cell tree: {} nodes, {} cells",
                    cells.node_count(),
                    cells.cell_count()
                );
                match cells.cell_for_point(&bytes, [point[0], point[1], point[2]]) {
                    Ok(Some(cell)) => {
                        let candidates = cells.object_candidates(&bytes, cell.cell_index)?;
                        println!("Local cell: node {}, leaf rank {}, cell index {}, cell ID {}, {} traversal records, {} object candidates, bounds {:?} .. {:?}",
                            cell.tree_node, cell.leaf_rank, cell.cell_index, cell.cell_id,
                            cell.link_count, candidates.len(), cell.bounds[0], cell.bounds[1]);
                        println!("Candidate Tome object indices: {candidates:?}");
                        let candidate_ids = candidates
                            .iter()
                            .map(|&index| ids[index as usize])
                            .collect::<Vec<_>>();
                        println!("Candidate authored object IDs: {candidate_ids:?}");
                        let initial_gates = tome::initial_gate_states(&bytes)?;
                        for (link_number, link) in cells
                            .traversal_records(&bytes, cell.cell_index)?
                            .into_iter()
                            .enumerate()
                        {
                            let destination = tree.direct_link_destination(&bytes, link)?;
                            let target_candidates = destination
                                .map(|target| {
                                    tome::CellTree::parse(&bytes, target.record_offset)?
                                        .object_candidates(&bytes, target.cell_index)
                                })
                                .transpose()?
                                .map(|indices| indices.len());
                            let gated = link[0] & (1 << 27) != 0;
                            let open = if gated {
                                Some(tome::gate_link_allows(&bytes, link, Some(&initial_gates))?)
                            } else {
                                None
                            };
                            let bounds = if gated {
                                tome::gated_link_query_bounds(&bytes, offset, link)?
                            } else {
                                tome::packed_link_geometry(link)
                                    .map(|geometry| {
                                        tome::packed_link_query_bounds(&bytes, offset, geometry)
                                    })
                                    .transpose()?
                            };
                            println!("  Link {link_number}: destination {destination:?}, target object candidates {target_candidates:?}, gated {gated}, initially open {open:?}, query bounds {bounds:?}");
                        }
                        point_candidates = Some(candidates);
                    }
                    Ok(None) => println!("Point has no direct local cell"),
                    Err(reason) => println!("Local cell selection unsupported: {reason}"),
                }
            }
        } else {
            println!("Point is outside the Tome global split tree");
        }
        if let Some((index, bounds)) = containing.iter().min_by(|left, right| {
            let volume = |bounds: &[[f32; 3]; 2]| -> f64 {
                (0..3)
                    .map(|axis| (bounds[1][axis] - bounds[0][axis]) as f64)
                    .product()
            };
            volume(&left.1).total_cmp(&volume(&right.1))
        }) {
            println!(
                "Smallest containing record by bound volume: index {index}, bounds {:?} .. {:?}",
                bounds[0], bounds[1]
            );
        }
    }
    if let Some(world_path) = args.get(2).filter(|arg| !arg.starts_with("--")) {
        let world = world_mesh::parse(&fs::read(&world_path)?)?;
        let mut instance_lods = vec![usize::MAX; world.visibility.len()];
        let mut lod_counts = std::collections::BTreeMap::<usize, (usize, usize)>::new();
        for object in &world.objects {
            instance_lods[object.matrix_start..object.matrix_end].fill(object.lod);
            let count = lod_counts.entry(object.lod).or_default();
            count.0 += 1;
            count.1 += object.matrix_end - object.matrix_start;
        }
        let mut suppression_lods = std::collections::BTreeMap::<(usize, usize), usize>::new();
        for group in &world.visibility_groups {
            for &child in &group.children {
                *suppression_lods
                    .entry((instance_lods[group.parent], instance_lods[child]))
                    .or_default() += 1;
            }
        }
        println!("BWM LOD (object, instance) counts: {lod_counts:?}");
        println!("BWM parent/child LOD suppression pairs: {suppression_lods:?}");
        let id_set: std::collections::HashSet<u32> = ids.iter().copied().collect();
        let matched: Vec<u32> = world
            .visibility
            .iter()
            .map(|record| record.umbra_id)
            .filter(|id| id_set.contains(id))
            .collect();
        let matched_unique = matched
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len();
        println!("BWM {world_path}: {} transforms, {} visibility records, {} matched instances, {} matched unique IDs, {} unmatched instances",
            world.matrices.len(), world.visibility.len(), matched.len(), matched_unique,
            world.visibility.len() - matched.len());
        if world.tome_object_instances.len() != ids.len() {
            return Err(format!(
                "BWM has {} Tome groups for {} Tome objects",
                world.tome_object_instances.len(),
                ids.len()
            )
            .into());
        }
        if let Some(candidates) = &point_candidates {
            for &object_index in candidates {
                let instances = &world.tome_object_instances[object_index as usize];
                println!(
                    "Point candidate Tome index {object_index}, authored ID {}: {} BWM instances",
                    ids[object_index as usize],
                    instances.len()
                );
                for &instance in instances {
                    let object = world
                        .objects
                        .iter()
                        .find(|object| {
                            object.matrix_start <= instance && instance < object.matrix_end
                        })
                        .ok_or("candidate BWM instance has no owning mesh")?;
                    println!(
                        "  instance {instance}, LOD {}, material {}, bounds {:?} .. {:?}",
                        object.lod,
                        object.material_path,
                        world.visibility[instance].bounds[0],
                        world.visibility[instance].bounds[1]
                    );
                }
            }
        }
        let mut mapped = 0;
        let mut id_mismatches = 0;
        let mut duplicate_references = 0;
        let empty_group_ids = ids
            .iter()
            .zip(&world.tome_object_instances)
            .filter_map(|(&id, instances)| instances.is_empty().then_some(id))
            .collect::<std::collections::HashSet<_>>();
        let mut referenced = vec![false; world.visibility.len()];
        for (id, instances) in ids.iter().zip(&world.tome_object_instances) {
            for &index in instances {
                mapped += 1;
                if world.visibility[index].umbra_id != *id {
                    id_mismatches += 1;
                }
                if std::mem::replace(&mut referenced[index], true) {
                    duplicate_references += 1;
                }
            }
        }
        println!("Serialized Tome mapping: {} groups, {mapped} instance references, {id_mismatches} ID mismatches, {duplicate_references} duplicate references; {} separate spatial-query records",
            world.tome_object_instances.len(), world.spatial_query_instances.len());
        for &index in &world.spatial_query_instances {
            let object = world
                .objects
                .iter()
                .find(|object| object.matrix_start <= index && index < object.matrix_end)
                .ok_or("spatial-query BWM instance has no owning mesh")?;
            println!("Separate spatial-query BWM instance {index}: Tome-group member {}, ID {}, LOD {}, material {}, bounds {:?} .. {:?}",
                referenced[index], world.visibility[index].umbra_id, object.lod,
                object.material_path, world.visibility[index].bounds[0], world.visibility[index].bounds[1]);
        }
        let mut covered = referenced.clone();
        let mut auxiliary_counts = [0usize; 4];
        let mut auxiliary_new = [0usize; 4];
        for (group, indices) in world.auxiliary_instance_groups.iter().enumerate() {
            auxiliary_counts[group] = indices.len();
            for &index in indices {
                if !std::mem::replace(&mut covered[index], true) {
                    auxiliary_new[group] += 1;
                }
            }
        }
        let spatial_new = world
            .spatial_query_instances
            .iter()
            .filter(|&&index| !std::mem::replace(&mut covered[index], true))
            .count();
        println!("Post-Tome BWM lists: counts {auxiliary_counts:?}, new instances {auxiliary_new:?}; separate spatial list adds {spatial_new}; {} instances absent from all decoded lists",
            covered.iter().filter(|&&value| !value).count());
        let mut group_parents = std::collections::HashSet::new();
        let mut group_children = std::collections::HashSet::new();
        for group in &world.visibility_groups {
            group_parents.insert(group.parent);
            group_children.extend(group.children.iter().copied());
        }
        println!("Post-query suppression groups: {} records, {} distinct parents ({} outside dispatch lists), {} distinct children ({} outside dispatch lists)",
            world.visibility_groups.len(), group_parents.len(),
            group_parents.iter().filter(|&&index| !covered[index]).count(),
            group_children.len(), group_children.iter().filter(|&&index| !covered[index]).count());
        let mut remaining_by_lod = std::collections::BTreeMap::<usize, usize>::new();
        let mut remaining_materials = std::collections::BTreeMap::<&str, usize>::new();
        let mut remaining_examples = Vec::new();
        for object in &world.objects {
            for index in object.matrix_start..object.matrix_end {
                if !covered[index] {
                    *remaining_by_lod.entry(object.lod).or_default() += 1;
                    *remaining_materials
                        .entry(&object.material_path)
                        .or_default() += 1;
                    if remaining_examples.len() < 8 {
                        remaining_examples.push(index);
                    }
                }
            }
        }
        let mut remaining_materials = remaining_materials.into_iter().collect::<Vec<_>>();
        remaining_materials
            .sort_unstable_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
        println!("BWM instances outside file-backed dispatch: by LOD {remaining_by_lod:?}, first 8 indices {remaining_examples:?}, top 8 materials {:?}",
            remaining_materials.into_iter().take(8).collect::<Vec<_>>());
        if id_mismatches != 0 || duplicate_references != 0 {
            return Err("BWM Tome-index mapping has ID mismatches or duplicate references".into());
        }
        let mut unmatched_by_lod = std::collections::BTreeMap::<usize, usize>::new();
        let mut unmatched_ids = std::collections::BTreeMap::<u32, usize>::new();
        let mut unmatched_materials = std::collections::BTreeMap::<&str, usize>::new();
        let mut unmatched_fields = std::collections::BTreeMap::<u16, usize>::new();
        let mut mapped_fields = std::collections::BTreeMap::<u16, usize>::new();
        for object in &world.objects {
            for index in object.matrix_start..object.matrix_end {
                let id = world.visibility[index].umbra_id;
                if !referenced[index] {
                    *unmatched_by_lod.entry(object.lod).or_default() += 1;
                    *unmatched_ids.entry(id).or_default() += 1;
                    *unmatched_materials
                        .entry(&object.material_path)
                        .or_default() += 1;
                    *unmatched_fields
                        .entry(world.visibility[index].trailing_field)
                        .or_default() += 1;
                } else {
                    *mapped_fields
                        .entry(world.visibility[index].trailing_field)
                        .or_default() += 1;
                }
            }
        }
        println!("BWM instances absent from Tome: by LOD {unmatched_by_lod:?}, {} distinct IDs; first 8 ID/count pairs {:?}",
            unmatched_ids.len(), unmatched_ids.iter().take(8).collect::<Vec<_>>());
        println!(
            "BWM instances absent from Tome with sentinel ID 0xffffffff: {}",
            unmatched_ids.get(&u32::MAX).copied().unwrap_or(0)
        );
        let empty_unmatched = unmatched_ids
            .keys()
            .filter(|id| empty_group_ids.contains(id))
            .count();
        let bwm_ids = world
            .visibility
            .iter()
            .map(|record| record.umbra_id)
            .collect::<std::collections::HashSet<_>>();
        let tome_only_ids = ids.iter().filter(|id| !bwm_ids.contains(id)).count();
        println!("Tome groups with no BWM instances: {}; Tome IDs absent from BWM: {tome_only_ids}; empty-group IDs present in BWM: {}; unmatched BWM IDs also in empty groups: {empty_unmatched}",
            empty_group_ids.len(), empty_group_ids.intersection(&bwm_ids).count());
        let mut materials = unmatched_materials.into_iter().collect::<Vec<_>>();
        materials.sort_unstable_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
        println!(
            "BWM instances absent from Tome: {} materials; top 12 {:#?}",
            materials.len(),
            materials.into_iter().take(12).collect::<Vec<_>>()
        );
        println!("BWM visibility trailing fields: mapped {mapped_fields:?}; unmapped {unmatched_fields:?}");
        if args.iter().any(|argument| argument == "--verify-bounds") {
            let (checked, escaped, largest_escape) = verify_world_bounds(&world);
            println!("Authored bounds: {checked} instances checked, {escaped} exceed by >0.02 game units, largest overrun {largest_escape:.8}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_header_relative_object_id_table() {
        let mut bytes = vec![0; 0x68];
        bytes[0x40..0x44].copy_from_slice(&2u32.to_le_bytes());
        bytes[0x50..0x54].copy_from_slice(&0x60u32.to_le_bytes());
        bytes[0x60..0x64].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes[0x64..0x68].copy_from_slice(&0x89ab_cdefu32.to_le_bytes());
        assert_eq!(object_ids(&bytes).unwrap(), vec![0x1234_5678, 0x89ab_cdef]);
        bytes.truncate(0x67);
        assert!(object_ids(&bytes).is_err());
    }

    #[test]
    fn reads_header_relative_gate_id_table() {
        let mut bytes = vec![0; 0x88];
        bytes[0x68..0x6c].copy_from_slice(&2u32.to_le_bytes());
        bytes[0x6c..0x70].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(&17u32.to_le_bytes());
        bytes[0x84..0x88].copy_from_slice(&23u32.to_le_bytes());
        assert_eq!(gate_ids(&bytes).unwrap(), vec![17, 23]);
        bytes.pop();
        assert!(gate_ids(&bytes).is_err());
    }

    #[test]
    fn reads_header_relative_spatial_record_offsets() {
        let mut bytes = vec![0; 0xc0];
        bytes[0x2c..0x30].copy_from_slice(&64u32.to_le_bytes());
        bytes[0xa0..0xa4].copy_from_slice(&0xb0u32.to_le_bytes());
        bytes[0xb4..0xb8].copy_from_slice(&0xabu32.to_le_bytes());
        assert_eq!(spatial_record_offsets(&bytes).unwrap(), vec![0, 0xab]);
        bytes[0xb4..0xb8].copy_from_slice(&0xc0u32.to_le_bytes());
        assert!(spatial_record_offsets(&bytes).is_err());
    }

    #[test]
    fn validates_spatial_record_bounds() {
        let mut bytes = vec![0; 32];
        for (index, value) in [-3.0f32, -2.0, -1.0, 1.0, 2.0, 3.0].iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            spatial_bounds(&bytes, 0).unwrap(),
            [[-3.0, -2.0, -1.0], [1.0, 2.0, 3.0]]
        );
        bytes[12..16].copy_from_slice(&(-4.0f32).to_le_bytes());
        assert!(spatial_bounds(&bytes, 0).is_err());
    }
}
