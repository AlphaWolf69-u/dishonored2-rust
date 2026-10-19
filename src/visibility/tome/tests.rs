#[test]
fn portal_depth_admission_matches_original_pair_masks() {
    let mut count = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/portal_depth_admission_cases.txt"
    ))
    .lines()
    {
        let words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let corners = std::array::from_fn(|i| {
            let pair = i % 4;
            let upper = i >= 4;
            [
                0.,
                0.,
                f32::from_bits(words[usize::from(upper) * 8 + pair]),
                f32::from_bits(words[usize::from(upper) * 8 + 4 + pair]),
            ]
        });
        assert_eq!(
            portal_box_depth_mask(corners),
            words[16],
            "native portal depth case{count}"
        );
        count += 1;
    }
    assert_eq!(count, 512);
}
use super::*;

#[test]
fn query_distance_threshold_matches_original_radius_overflow_and_unordered_cases() {
    let mut count = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/query_distance_threshold_cases.txt"
    ))
    .lines()
    {
        let w = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let mut bytes = [0u8; 16];
        bytes[12..16].copy_from_slice(&w[2].to_le_bytes());
        let radius = if w[0] == 0 { 1.0 } else { f32::from_bits(w[1]) };
        assert_eq!(
            render_query_global_distance_threshold(&bytes, f32::from_bits(w[3]), radius)
                .unwrap()
                .to_bits(),
            w[4],
            "native query distance case{count}"
        );
        count += 1;
    }
    assert_eq!(count, 440);
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn ordinary_global_pass_seeds_authored_eotw_camera_cell() {
    let bytes =
        std::fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = SpatialTree::parse(&bytes).unwrap();
    let distance_scales = tree.global_distance_scales.as_ref().unwrap();
    assert_eq!(distance_scales.len(), tree.node_count());
    assert_eq!(distance_scales[0], -1.0);
    assert!(distance_scales[1] > 0.0);
    assert_eq!(
        SpatialCellSlotRows::parse(&bytes)
            .unwrap()
            .spatial_record_count(),
        tree.node_count()
    );
    let point = [-43.0, 138.0, 44.0];
    let mut worklists = SpatialCellWorklists::new_for_original_query();
    let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let mask = PortalTileMask::full([0, 0, 8, 16]).unwrap();
    let planes = [[0.0, 0.0, 0.0, 1.0]; 6];
    let result = tree
        .seed_ordinary_camera_global_query(
            &bytes,
            None,
            point,
            &planes,
            [0.0; 3],
            0.0,
            &mut worklists,
            &mut pool,
            &mut coverage,
            &mask,
        )
        .unwrap()
        .expect("camera point inside the Tome root");
    assert_eq!(result.nodes_visited, tree.node_count());
    assert!(result
        .seeded_records
        .iter()
        .any(|seed| { seed.spatial_node_index == 903 && seed.selection.cell_id == 1346 }));
    assert_eq!(result.branch_states[1] & 0x8000, 0x8000);
    assert!(result.branch_stack.is_active());
    for seed in &result.seeded_records {
        assert_eq!(
            worklists.pending_cell(seed.pending_slot as usize, seed.selection.cell_index as u16),
            Some(seed.work_item)
        );
    }
    let pose = [
        [1.0, 0.0, 0.0, point[0]],
        [0.0, 1.0, 0.0, point[1]],
        [0.0, 0.0, 1.0, point[2]],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let matrix = original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
    let planes = query_planes_from_rows(matrix, matrix);
    let mut worklists = SpatialCellWorklists::new_for_original_query();
    let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let mut frustum_result = tree
        .seed_ordinary_camera_global_query(
            &bytes,
            None,
            point,
            &planes,
            [0.6; 3],
            ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            &mut worklists,
            &mut pool,
            &mut coverage,
            &mask,
        )
        .unwrap()
        .expect("camera point inside the Tome root");
    assert!(frustum_result.nodes_visited < tree.node_count());
    let mut unbounded_worklists = SpatialCellWorklists::new_for_original_query();
    let mut unbounded_pool = CellWorkNodePool::new_for_original_query().unwrap();
    let mut unbounded_coverage = VisitedCellCoverage::default();
    let unbounded_result = tree
        .seed_ordinary_camera_global_query(
            &bytes,
            None,
            point,
            &planes,
            [0.6; 3],
            0.0,
            &mut unbounded_worklists,
            &mut unbounded_pool,
            &mut unbounded_coverage,
            &mask,
        )
        .unwrap()
        .expect("camera point inside the Tome root");
    assert!(frustum_result.nodes_visited < unbounded_result.nodes_visited);
    assert!(frustum_result
        .seeded_records
        .iter()
        .any(|seed| { seed.spatial_node_index == 903 && seed.selection.cell_id == 1346 }));
    let mut active_nodes = vec![u32::MAX; tree.node_count().div_ceil(32)];
    let activation = frustum_result
        .activate_next_spatial_record(&tree, point, &mut active_nodes, &mut worklists)
        .unwrap()
        .expect("first seeded spatial record");
    assert!(activation.activated_cells > 0);
    assert_eq!(
        frustum_result.branch_states[activation.pending_slot as usize],
        0
    );
    assert!(frustum_result.seeded_records.iter().any(|seed| {
        seed.spatial_node_index == activation.spatial_node_index
            && seed.pending_slot == activation.pending_slot
    }));
    assert!(tree
        .seed_ordinary_camera_global_query(
            &bytes,
            None,
            [100_000.0, 0.0, 0.0],
            &planes,
            [0.6; 3],
            ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            &mut worklists,
            &mut pool,
            &mut coverage,
            &mask,
        )
        .unwrap()
        .is_none());
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn authored_eotw_record_direct_loop_accepts_roof_objects() {
    let bytes =
        std::fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = SpatialTree::parse(&bytes).unwrap();
    let point = [-43.0, 138.0, 44.0];
    let record = tree.record_offset(903).unwrap();
    let cells = CellTree::parse(&bytes, record).unwrap();
    let cell = cells.cell_for_point(&bytes, point).unwrap().unwrap();
    assert_eq!(cell.cell_id, 1346);
    let viewpoint = [point[0], point[1], 100.0];
    let pose = [
        [1.0, 0.0, 0.0, viewpoint[0]],
        [0.0, 1.0, 0.0, viewpoint[1]],
        [0.0, 0.0, 1.0, viewpoint[2]],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let matrix = original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
    let planes = query_planes_from_rows(matrix, matrix);
    let stored_matrix = stored_query_matrix_from_render_rows(matrix);
    let setup = setup_spatial_record_query(&bytes, record, viewpoint, [0.6; 3], &planes).unwrap();
    let portal = PortalTileMask::full([0, 0, 8, 16]).unwrap();
    let query = CellObjectVisibilityQuery {
        record_query_bounds: setup.bounds,
        query_planes: &planes,
        active_planes: &setup.active_planes,
        projection_rows: stored_matrix,
        camera_translation_imprecision: 0.6,
        object_distance_point: viewpoint,
        object_distance_scale_squared: [1.0; 4],
        minimum_area_metric: 25.0 / (1920.0 * 1080.0),
        viewport_tile_bounds: portal.rect,
        portal: &portal,
        depth_grid: None,
        point_occlusion_near_offset: None,
    };
    let mut output = VisibleObjectIndexOutput::new(object_ids(&bytes).unwrap().len(), 32768);
    cells
        .append_visible_object_candidates(&bytes, cell.cell_index, &query, &mut output)
        .unwrap();
    assert_eq!(
        cells.object_candidates(&bytes, cell.cell_index).unwrap(),
        [28684, 28685, 28686]
    );
    assert_eq!(output.indices(), &[28686]);
    assert!(output.area_metrics().iter().all(|area| area.is_some()));
    let direction = LinkQueryDirection::Volume {
        center: setup.link_center,
        tolerance: setup.link_tolerance,
    };
    let gates = initial_gate_states(&bytes).unwrap();
    let links = cells.traversal_records(&bytes, cell.cell_index).unwrap();
    assert_eq!(links.len(), 12);
    let link = links[0];
    assert_eq!(direct_spatial_link_target(link), Some(903));
    assert!(
        traversal_link_precheck(&bytes, link, direction, Some(&gates))
            .unwrap()
            .allows_traversal()
    );
    let face = admitted_link_portal_face(&bytes, record, link, direction, viewpoint, None).unwrap();
    assert_eq!(face, Some(0));
    let (pixels, path) = projected_link_portal_transition(
        &bytes,
        record,
        link,
        face,
        stored_matrix,
        false,
        &setup.active_planes,
        [0, 0, 64, 64],
        &portal,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pixels, [29, 29, 30, 41]);
    assert!(matches!(path, PortalCoveragePath::Clipped { .. }));
    let mut worklists = SpatialCellWorklists::new_for_original_query();
    let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let changed = traverse_same_spatial_direct_link_from_pool(
        &bytes,
        record,
        link,
        direction,
        Some(&gates),
        viewpoint,
        None,
        &PortalQueryProjection::new(stored_matrix, false),
        &setup.active_planes,
        [0, 0, 64, 64],
        &portal,
        903,
        &mut worklists,
        &mut pool,
        &mut coverage,
        |_, _, _| Ok(()),
    )
    .unwrap();
    let target_cell = (link[1] >> 16) as usize;
    assert!(!changed);
    assert_eq!(worklists.cell_states()[target_cell], 0);
    assert!(coverage
        .coverage(TraversalCellId {
            spatial_index: 903,
            cell_index: target_cell
        })
        .is_none());

    let rows = SpatialCellSlotRows::parse(&bytes).unwrap();
    assert_eq!(rows.spatial_record_count(), tree.node_count());
    let mut cross_worklists = SpatialCellWorklists::new_for_original_query();
    let mut cross_pool = CellWorkNodePool::new_for_original_query().unwrap();
    let mut cross_coverage = VisitedCellCoverage::default();
    let mut global = tree
        .seed_ordinary_camera_global_query(
            &bytes,
            None,
            point,
            &[[0.0, 0.0, 0.0, 1.0]; 6],
            [0.6; 3],
            ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            &mut cross_worklists,
            &mut cross_pool,
            &mut cross_coverage,
            &portal,
        )
        .unwrap()
        .unwrap();
    let cross_link = links
        .iter()
        .copied()
        .find(|&link| direct_spatial_link_target(link) == Some(725))
        .unwrap();
    let mut mapper = SpatialCellSlotMapper::default();
    let changed = traverse_cross_spatial_direct_link_from_pool(
        &bytes,
        record,
        cross_link,
        direction,
        Some(&gates),
        viewpoint,
        None,
        &PortalQueryProjection::new(stored_matrix, false),
        &setup.active_planes,
        [0, 0, 64, 64],
        &portal,
        903,
        &rows,
        &mut mapper,
        &mut global.branch_states,
        &mut cross_worklists,
        &mut cross_pool,
        &mut cross_coverage,
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert!(!changed);
    let cross_cell = (cross_link[1] >> 16) as u16;
    let slot = rows
        .resolve(&mut mapper, 725, &global.branch_states)
        .unwrap();
    assert!(cross_worklists
        .pending_cell(slot as usize, cross_cell)
        .is_none());
    assert!(mapper.intermediate_slots().is_empty());
    assert!(cross_coverage
        .coverage(TraversalCellId {
            spatial_index: 725,
            cell_index: cross_cell as usize,
        })
        .is_none());

    let mut loop_worklists = SpatialCellWorklists::new_for_original_query();
    let mut loop_pool = CellWorkNodePool::new_for_original_query().unwrap();
    let mut loop_coverage = VisitedCellCoverage::default();
    let mut loop_global = tree
        .seed_ordinary_camera_global_query(
            &bytes,
            None,
            point,
            &[[0.0, 0.0, 0.0, 1.0]; 6],
            [0.6; 3],
            ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            &mut loop_worklists,
            &mut loop_pool,
            &mut loop_coverage,
            &portal,
        )
        .unwrap()
        .unwrap();
    let camera_seed = loop_global
        .seeded_records
        .iter()
        .find(|seed| seed.spatial_node_index == 903 && seed.selection.cell_id == 1346)
        .unwrap();
    loop_worklists
        .activate_pending_spatial(camera_seed.pending_slot as usize)
        .unwrap();
    loop_global.branch_states[camera_seed.pending_slot as usize] = 0;
    let mut loop_mapper = SpatialCellSlotMapper::default();
    let processed = process_next_active_direct_cell(
        &cells,
        &bytes,
        record,
        903,
        direction,
        &vec![u32::MAX; tree.node_count().div_ceil(32)],
        Some(&gates),
        viewpoint,
        None,
        &PortalQueryProjection::new(stored_matrix, false),
        &setup.active_planes,
        &rows,
        &mut loop_mapper,
        &mut loop_global.branch_states,
        &mut loop_worklists,
        &mut loop_pool,
        &mut loop_coverage,
        None,
        |_, _, _| Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(processed, camera_seed.work_item);
    assert_eq!(loop_pool.recycled_count(), 1);
    assert!(loop_coverage
        .coverage(TraversalCellId {
            spatial_index: 725,
            cell_index: cross_cell as usize,
        })
        .is_none());
    let mut progressed = 1;
    for _ in 0..500 {
        match process_next_active_direct_cell(
            &cells,
            &bytes,
            record,
            903,
            direction,
            &vec![u32::MAX; tree.node_count().div_ceil(32)],
            Some(&gates),
            viewpoint,
            None,
            &PortalQueryProjection::new(stored_matrix, false),
            &setup.active_planes,
            &rows,
            &mut loop_mapper,
            &mut loop_global.branch_states,
            &mut loop_worklists,
            &mut loop_pool,
            &mut loop_coverage,
            None,
            |_, _, _| Ok(()),
        )
        .unwrap()
        {
            Some(_) => progressed += 1,
            None => break,
        }
    }
    assert_eq!(progressed, 1);
    assert!(loop_worklists.active_worklist().is_empty());
    let mut loop_output = VisibleObjectIndexOutput::new(object_ids(&bytes).unwrap().len(), 32768);
    let flushed = loop_pool
        .flush_recycled(
            loop_worklists.cell_states_mut(),
            &mut loop_coverage,
            |item, owner, state| {
                assert_eq!(owner.spatial_index, 903);
                let portal = state.coverage(owner).unwrap();
                let query = CellObjectVisibilityQuery {
                    record_query_bounds: setup.bounds,
                    query_planes: &planes,
                    active_planes: &setup.active_planes,
                    projection_rows: stored_matrix,
                    camera_translation_imprecision: 0.6,
                    object_distance_point: viewpoint,
                    object_distance_scale_squared: [1.0; 4],
                    minimum_area_metric: 25.0 / (1920.0 * 1080.0),
                    viewport_tile_bounds: portal.rect,
                    portal,
                    depth_grid: None,
                    point_occlusion_near_offset: None,
                };
                cells.append_visible_object_candidates(
                    &bytes,
                    item.cell_index as usize,
                    &query,
                    &mut loop_output,
                )
            },
        )
        .unwrap();
    assert_eq!(flushed, 1);
    assert_eq!(loop_output.indices(), &[28686]);
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn ordinary_direct_query_returns_authored_eotw_roof_objects() {
    let bytes =
        std::fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = SpatialTree::parse(&bytes).unwrap();
    let point = [-43.0, 138.0, 48.0];
    let pose = [
        [1.0, 0.0, 0.0, point[0]],
        [0.0, 1.0, 0.0, point[1]],
        [0.0, 0.0, 1.0, point[2]],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let rows = original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
    let gates = initial_gate_states(&bytes).unwrap();
    let input = OrdinaryDirectQueryInput {
        camera_point: point,
        render_rows: rows,
        cropped_rows: rows,
        initial_tile_rect: [0, 0, 8, 16],
        camera_imprecision: [0.6; 3],
        global_distance_threshold: ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
        object_distance_scale: 1.0,
        object_imprecision: 0.6,
        minimum_area_metric: 25.0 / (1920.0 * 1080.0),
        winding_flip: false,
        gate_states: Some(&gates),
        output_capacity: 32768,
    };
    let mut diagnostics = OrdinaryQueryDiagnostics::default();
    let output = ordinary_direct_tome_visible_objects_with_diagnostics(
        &tree,
        &bytes,
        &input,
        &mut diagnostics,
    )
    .unwrap()
    .unwrap();
    assert_eq!(output.indices(), &[28684]);
    assert!(output.area_metrics().iter().all(|area| area.is_some()));
    assert_eq!(diagnostics.stage, "complete");
    assert!(diagnostics.root_accepted);
    assert_eq!(diagnostics.accepted_objects, 1);
    assert!(diagnostics.distance_rejected_objects >= 2);
    assert!(diagnostics.cells_processed > 0);

    // Force a late object-pass error, rather than a setup failure, so
    // the last cell and traversal progress must survive the discarded output.
    let mut corrupt = bytes.clone();
    let bounds_at = word(&bytes, 0x44).unwrap() as usize + 28684 * 0x18;
    corrupt[bounds_at..bounds_at + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    let error = ordinary_direct_tome_visible_objects_with_diagnostics(
        &tree,
        &corrupt,
        &input,
        &mut diagnostics,
    )
    .unwrap_err();
    assert!(error.contains("invalid Tome object bounds"));
    assert_eq!(diagnostics.stage, "object_pass");
    assert!(diagnostics.current_spatial_record.is_some());
    assert!(diagnostics.current_object_cell.is_some());
    assert!(diagnostics.object_passes > 0);
    assert_eq!(diagnostics.accepted_objects, 0);
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn requested_depth_output_matches_cached_and_uncached_eotw_jobs() {
    let bytes = std::fs::read("artifacts/eotw_p_lowchaos.tome").unwrap();
    let tree = SpatialTree::parse(&bytes).unwrap();
    let data = DirectTomeQueryData::parse(&bytes, &tree).unwrap();
    let point = [-43.0, 138.0, 48.0];
    let pose = [
        [1.0, 0.0, 0.0, point[0]],
        [0.0, 1.0, 0.0, point[1]],
        [0.0, 0.0, 1.0, point[2]],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let rows = original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
    let gates = initial_gate_states(&bytes).unwrap();
    let input = OrdinaryDirectQueryInput {
        camera_point: point,
        render_rows: rows,
        cropped_rows: rows,
        initial_tile_rect: [0, 0, 8, 16],
        camera_imprecision: [0.6; 3],
        object_imprecision: 0.6,
        minimum_area_metric: 25.0 / (1920.0 * 1080.0),
        global_distance_threshold: ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
        object_distance_scale: 1.0,
        winding_flip: query_matrix_winding_flip(rows),
        gate_states: Some(&gates),
        output_capacity: ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
    };
    for count in [1, 4] {
        let mut cached_jobs = Vec::new();
        for job in 0..count {
            let cached = ordinary_direct_tome_render_job(&tree, &bytes, &data, &input, job, count);
            let uncached =
                ordinary_direct_tome_render_job_uncached(&tree, &bytes, &input, job, count);
            let left = cached.result.as_ref().unwrap().as_ref().unwrap();
            let right = uncached.result.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(
                left.indices(),
                right.indices(),
                "job{job}/{count} object output"
            );
            assert_eq!(
                left.area_metrics(),
                right.area_metrics(),
                "job{job}/{count} area output"
            );
            assert_eq!(
                cached.state, uncached.state,
                "job{job}/{count} descriptor/grid"
            );
            // Requested output must not feed back into current object tests.
            let local = input.for_render_job(job, count).unwrap();
            let baseline = ordinary_direct_tome_visible_objects_cached_with_diagnostics(
                &tree,
                &bytes,
                &data,
                &local,
                &mut OrdinaryQueryDiagnostics::default(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                left.indices(),
                baseline.indices(),
                "job{job}/{count} output/input separation"
            );
            cached_jobs.push(cached);
        }
        let output = merge_render_query_outputs(&cached_jobs).unwrap().unwrap();
        let state = output.state.as_ref().unwrap();
        let grid = state
            .depth_grid
            .as_ref()
            .expect("requested descriptor must contain generated depth");
        let populated = grid.iter().filter(|&&value| value != 0.0).count();
        assert!(populated > 0, "{count} jobs produced an empty grid");
        assert!(grid
            .iter()
            .all(|&value| value.is_finite() && (0.0..=1.0).contains(&value)));
        assert!(state.maximum_depth > 0.0 && state.maximum_depth <= 1.0);
        let query = RenderAabbQuery::from_state(state).unwrap();
        for &index in output.indices() {
            assert!(
                query
                    .visible(object_bounds(&bytes, index).unwrap())
                    .unwrap(),
                "generated depth rejected accepted object{index} with{count} jobs"
            );
        }
        println!(
            "EOTW depth jobs={count} objects={} populated={populated} maximum={}",
            output.indices().len(),
            state.maximum_depth
        );
    }
}

#[test]
fn grows_and_merges_original_portal_tile_coverage() {
    assert_eq!(portal_tile_rect([9, 5, 24, 12]).unwrap(), [1, 1, 3, 3]);
    assert!(portal_tile_rect([9, 5, 9, 12]).is_err());
    let mut destination = PortalTileMask::new([2, 3, 4, 5]).unwrap();
    destination.bits = vec![1, 2, 4, 8];
    destination.grow_to_include([1, 4, 5, 6]).unwrap();
    assert_eq!(destination.rect, [1, 3, 5, 6]);
    assert_eq!(destination.bits, [0, 1, 2, 0, 0, 4, 8, 0, 0, 0, 0, 0]);
    let mut incoming = PortalTileMask::new([2, 4, 4, 6]).unwrap();
    incoming.bits = vec![4, 8, 16, 32];
    assert!(destination.merge_new_coverage(&incoming).unwrap());
    assert_eq!(destination.bits, [0, 1, 2, 0, 0, 4, 8, 0, 0, 16, 32, 0]);
    assert!(!destination.merge_new_coverage(&incoming).unwrap());
    assert!(destination
        .merge_new_coverage(&PortalTileMask::new([0, 0, 1, 1]).unwrap())
        .is_err());
}

#[test]
fn initial_camera_cell_mask_covers_the_viewport_tile_rectangle() {
    let mask = PortalTileMask::full([3, 5, 6, 7]).unwrap();
    assert_eq!(mask.bits, vec![u32::MAX; 6]);
    assert!(mask.intersects_pixels([24, 20, 48, 28]).unwrap());
    assert!(PortalTileMask::full([4, 5, 4, 7]).is_err());
}

#[test]
fn portal_face_vectors_follow_original_query_context_cofactors_and_winding() {
    let matrix = [
        [1.0, 2.0, 3.0, 4.0],
        [5.0, 6.0, 7.0, 8.0],
        [9.0, 10.0, 11.0, 12.0],
        [13.0, 14.0, 15.0, 16.0],
    ];
    let normal = portal_face_vectors_from_query_matrix(matrix, false).unwrap();
    let flipped = portal_face_vectors_from_query_matrix(matrix, true).unwrap();

    // The first cofactor triple is [-8, 12, -4], the second [16, -24,
    // 8], and the third [-8, 12, -4]. FUN_140a81500 emits the three
    // cyclic axis pairs in face order and swaps each pair by +0x4b4.
    assert_eq!(
        normal[0],
        [
            [8.0, 16.0, -8.0, -16.0],
            [-12.0, -24.0, 12.0, 24.0],
            [4.0, 8.0, -4.0, -8.0]
        ]
    );
    assert_eq!(
        normal[1],
        [
            [16.0, 8.0, -16.0, -8.0],
            [-24.0, -12.0, 24.0, 12.0],
            [8.0, 4.0, -8.0, -4.0]
        ]
    );
    assert_eq!(
        normal[2],
        [
            [8.0, -8.0, -8.0, 8.0],
            [-12.0, 12.0, 12.0, -12.0],
            [4.0, -4.0, -4.0, 4.0]
        ]
    );
    assert_eq!(
        normal[3],
        [
            [-8.0, 8.0, 8.0, -8.0],
            [12.0, -12.0, -12.0, 12.0],
            [-4.0, 4.0, 4.0, -4.0]
        ]
    );
    assert_eq!(
        normal[4],
        [
            [-16.0, -8.0, 16.0, 8.0],
            [24.0, 12.0, -24.0, -12.0],
            [-8.0, -4.0, 8.0, 4.0]
        ]
    );
    assert_eq!(
        normal[5],
        [
            [-8.0, -16.0, 8.0, 16.0],
            [12.0, 24.0, -12.0, -24.0],
            [-4.0, -8.0, 4.0, 8.0]
        ]
    );
    for pair in 0..3 {
        assert_eq!(flipped[pair * 2], normal[pair * 2 + 1]);
        assert_eq!(flipped[pair * 2 + 1], normal[pair * 2]);
    }
    assert!(portal_face_vectors_from_query_matrix([[f32::NAN; 4]; 4], false).is_err());
}

#[test]
fn portal_corner_transform_uses_original_query_matrix_component_order() {
    let matrix = [
        [1.0, 2.0, 3.0, 4.0],
        [5.0, 6.0, 7.0, 8.0],
        [9.0, 10.0, 11.0, 12.0],
        [13.0, 14.0, 15.0, 16.0],
    ];
    assert_eq!(
        transform_portal_corner_by_query_matrix([2.0, 3.0, 4.0], matrix).unwrap(),
        [66.0, 76.0, 86.0, 96.0]
    );
    assert!(transform_portal_corner_by_query_matrix([f32::INFINITY, 0.0, 0.0], matrix).is_err());
}

#[test]
fn caller_builds_raster_corners_in_original_face_cycle() {
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let bounds = [[1.0, 2.0, 3.0], [5.0, 7.0, 11.0]];
    let (normal_face, face_vectors) =
        portal_raster_inputs_from_bounds(bounds, 0, identity, false).unwrap();
    assert_eq!(
        normal_face,
        [
            [1.0, 1.0, 1.0, 1.0],
            [2.0, 7.0, 7.0, 2.0],
            [1.0, 1.0, 1.0, 1.0],
            [4.0, 0.0, 0.0, 0.0],
        ]
    );
    assert_eq!(
        face_vectors,
        portal_face_vectors_from_query_matrix(identity, false).unwrap()[0]
    );

    let (reversed_face, _) = portal_raster_inputs_from_bounds(bounds, 1, identity, false).unwrap();
    assert_eq!(
        reversed_face,
        [
            [1.0, 1.0, 1.0, 1.0],
            [2.0, 2.0, 7.0, 7.0],
            [1.0, 1.0, 1.0, 1.0],
            [4.0, 0.0, 0.0, 0.0],
        ]
    );
    assert_eq!(normal_face[2], [1.0; 4]);
    assert!(portal_raster_inputs_from_bounds(bounds, 6, identity, false).is_err());
}

#[test]
fn real_street_portal_matches_original_raster() {
    let source = PortalTileMask::full([0, 0, 8, 16]).unwrap();
    let mut target = PortalTileMask::new([0, 0, 8, 16]).unwrap();
    let projected = [
        [2.0440674; 4],
        [-4.0475955, -4.0475955, 7.0755935, 7.0755935],
        [4.0101776, -0.48988342, -0.48988342, 4.0101776],
        [-0.5615573, 0.0, 0.0, 0.0],
    ];
    let face = [
        [0.0, 0.0, -0.0, -0.0],
        [1.1231146, -0.0, -1.1231146, 0.0],
        [0.0, 2.379513, -0.0, -2.379513],
    ];
    let changed = target
        .rasterize_projected_box(&source, [0, 0, 49, 64], projected, face)
        .unwrap();
    assert!(!changed);
    assert!(target.bits.iter().all(|&bits| bits == 0));
}

#[test]
fn local_link_face_selection_preserves_direction_and_no_portal_branch() {
    let link = [1 << 29, 50, (40 << 16) | 60, (40 << 16) | 60];
    let volume = |center| LinkQueryDirection::Volume {
        center,
        tolerance: [2, 0, 0, 0],
    };
    assert_eq!(
        admitted_link_portal_face(&[], 0, link, volume([20, 50, 50, 0]), [0.0; 3], None).unwrap(),
        Some(1)
    );
    assert_eq!(
        admitted_link_portal_face(&[], 0, link, volume([50, 50, 50, 0]), [0.0; 3], None).unwrap(),
        None
    );
    assert_eq!(
        admitted_link_portal_face(&[], 0, link, volume([50, 25, 50, 0]), [0.0; 3], None).unwrap(),
        Some(3)
    );
    assert_eq!(
        admitted_link_portal_face(&[], 0, link, volume([50, 75, 50, 0]), [0.0; 3], None).unwrap(),
        Some(2)
    );
    assert_eq!(
        admitted_link_portal_face(
            &[],
            0,
            link,
            LinkQueryDirection::Point {
                axis_sides: [false; 4]
            },
            [0.0; 3],
            None
        )
        .unwrap(),
        Some(1)
    );
}

#[test]
fn local_no_portal_transition_reuses_incoming_tile_extent() {
    let incoming = PortalTileMask::full([2, 3, 4, 5]).unwrap();
    let (pixels, path) = projected_link_portal_transition(
        &[],
        0,
        [0; 4],
        None,
        [[0.0; 4]; 4],
        false,
        &[],
        [0, 0, 64, 64],
        &incoming,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pixels, [16, 12, 32, 20]);
    assert_eq!(path, PortalCoveragePath::Unclipped);
}

#[test]
fn direct_local_link_reaches_unvisited_cell_through_original_work_pool() {
    let link = [
        (1 << 29) | 4,
        (1 << 16) | 50,
        (40 << 16) | 60,
        (40 << 16) | 60,
    ];
    let direction = LinkQueryDirection::Volume {
        center: [50, 50, 50, 0],
        tolerance: [0; 4],
    };
    let destination = TraversalCellId {
        spatial_index: 4,
        cell_index: 1,
    };
    let incoming = PortalTileMask::full([0, 0, 1, 1]).unwrap();
    let mut worklists = SpatialCellWorklists::new(2);
    let mut pool = CellWorkNodePool::new(3).unwrap();
    let mut coverage = VisitedCellCoverage::default();
    assert!(traverse_direct_unvisited_link_from_pool(
        &[],
        0,
        link,
        direction,
        None,
        [0.0; 3],
        None,
        &PortalQueryProjection::new([[0.0; 4]; 4], false),
        &[],
        [0, 0, 8, 4],
        &incoming,
        4,
        destination,
        0,
        &mut worklists,
        &mut pool,
        &mut coverage,
        |_, _, _| Ok(())
    )
    .unwrap());
    assert_eq!(worklists.cell_states()[1], CellTraversalState::QUEUED | 1);
    assert_eq!(pool.free_count(), 1);
    assert_eq!(coverage.coverage(destination).unwrap(), &incoming);
    assert_eq!(worklists.active_worklist().entries().count(), 1);
}

#[test]
fn new_portal_bits_reactivate_a_processed_cell_from_recycled_chain() {
    let destination = TraversalCellId {
        spatial_index: 4,
        cell_index: 1,
    };
    let mut worklists = SpatialCellWorklists::new(2);
    let mut pool = CellWorkNodePool::new(3).unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let mut first = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    first.bits[0] = 1;
    worklists
        .propagate_and_schedule_unvisited_cell_from_pool(
            &mut pool,
            &mut coverage,
            destination,
            0,
            true,
            [0, 0, 8, 4],
            &first,
            PortalCoveragePath::Unclipped,
            |_, _, _| Ok(()),
        )
        .unwrap();
    let item = worklists.active.pop_front().unwrap();
    pool.release_processed(item, worklists.cell_states_mut())
        .unwrap();
    assert_eq!(pool.recycled_count(), 1);
    assert!(!worklists
        .reactivate_processed_active_cell(
            &mut pool,
            &mut coverage,
            destination,
            item,
            [0, 0, 8, 4],
            &first,
            PortalCoveragePath::Unclipped,
        )
        .unwrap());
    assert_eq!(pool.recycled_count(), 1);
    let mut second = first.clone();
    second.bits[0] = 2;
    let link = [
        (1 << 29) | 4,
        (1 << 16) | 50,
        (40 << 16) | 60,
        (40 << 16) | 60,
    ];
    assert!(traverse_same_spatial_direct_link_from_pool(
        &[],
        0,
        link,
        LinkQueryDirection::Volume {
            center: [50, 50, 50, 0],
            tolerance: [0; 4],
        },
        None,
        [0.0; 3],
        None,
        &PortalQueryProjection::new([[0.0; 4]; 4], false),
        &[],
        [0, 0, 8, 4],
        &second,
        4,
        &mut worklists,
        &mut pool,
        &mut coverage,
        |_, _, _| Ok(())
    )
    .unwrap());
    assert_eq!(pool.recycled_count(), 0);
    assert_eq!(
        worklists.cell_states()[1],
        item.entry_id | CellTraversalState::QUEUED
    );
    assert_eq!(
        worklists.active_worklist().entries().collect::<Vec<_>>(),
        [item]
    );
    assert_eq!(coverage.coverage(destination).unwrap().bits, vec![3]);
}

#[test]
fn visited_cell_coverage_requeues_only_for_new_bits_per_spatial_cell() {
    let cell = TraversalCellId {
        spatial_index: 903,
        cell_index: 71,
    };
    let mut visited = VisitedCellCoverage::default();
    let mut first = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    first.bits[0] = 1;
    assert!(visited.merge_incoming(cell, &first).unwrap());
    assert!(!visited.merge_incoming(cell, &first).unwrap());

    let mut next_portal = PortalTileMask::new([1, 0, 2, 1]).unwrap();
    next_portal.bits[0] = 2;
    assert!(visited.merge_incoming(cell, &next_portal).unwrap());
    let coverage = visited.coverage(cell).unwrap();
    assert_eq!(coverage.rect, [0, 0, 2, 1]);
    assert_eq!(coverage.bits, [1, 2]);

    let other_spatial_record = TraversalCellId {
        spatial_index: 569,
        cell_index: 71,
    };
    assert!(visited
        .merge_incoming(other_spatial_record, &next_portal)
        .unwrap());
}

#[test]
fn portal_propagation_uses_direct_merge_or_projected_clip_branch() {
    let cell = TraversalCellId {
        spatial_index: 12,
        cell_index: 3,
    };
    let mut incoming = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    incoming.bits[0] = u32::MAX;
    let mut visited = VisitedCellCoverage::default();

    assert!(visited
        .propagate_portal(cell, [0, 0, 4, 4], &incoming, PortalCoveragePath::Unclipped)
        .unwrap());
    assert!(!visited
        .propagate_portal(cell, [0, 0, 4, 4], &incoming, PortalCoveragePath::Unclipped)
        .unwrap());
    assert_eq!(visited.coverage(cell).unwrap().bits, [u32::MAX]);

    let projected = [
        [-0.9, -0.9, 0.9, 0.9],
        [-0.9, 0.9, 0.9, -0.9],
        [1.0; 4],
        [0.0; 4],
    ];
    let clipped_cell = TraversalCellId {
        spatial_index: 12,
        cell_index: 4,
    };
    assert!(visited
        .propagate_portal(
            clipped_cell,
            [0, 0, 4, 4],
            &incoming,
            PortalCoveragePath::Clipped {
                projected,
                face: [[0.0; 4]; 3],
            },
        )
        .unwrap());
    assert_eq!(visited.coverage(clipped_cell).unwrap().bits, [0x0000_8000]);
    assert!(!visited
        .propagate_portal(
            clipped_cell,
            [0, 0, 4, 4],
            &incoming,
            PortalCoveragePath::Clipped {
                projected,
                face: [[0.0; 4]; 3],
            },
        )
        .unwrap());
}

#[test]
fn failed_portal_propagation_preserves_existing_coverage() {
    let cell = TraversalCellId {
        spatial_index: 12,
        cell_index: 3,
    };
    let incoming = PortalTileMask::full([0, 0, 1, 1]).unwrap();
    let mut visited = VisitedCellCoverage::default();
    visited.merge_incoming(cell, &incoming).unwrap();
    let before = visited.coverage(cell).unwrap().clone();
    let mut nonfinite = [[0.0; 4]; 4];
    nonfinite[0][0] = f32::NAN;
    assert!(visited
        .propagate_portal(
            cell,
            [0, 0, 4, 4],
            &incoming,
            PortalCoveragePath::Clipped {
                projected: nonfinite,
                face: [[0.0; 4]; 3]
            }
        )
        .is_err());
    assert_eq!(visited.coverage(cell), Some(&before));
    let wider = PortalTileMask::full([0, 0, 2, 1]).unwrap();
    assert!(visited
        .propagate_portal(cell, [0, 0, 4, 4], &wider, PortalCoveragePath::Unclipped)
        .is_err());
    assert_eq!(visited.coverage(cell), Some(&before));
    // The growth branch must also roll back if its later raster validation fails.
    assert!(visited
        .propagate_portal(
            cell,
            [0, 0, 16, 4],
            &wider,
            PortalCoveragePath::Clipped {
                projected: nonfinite,
                face: [[0.0; 4]; 3]
            }
        )
        .is_err());
    assert_eq!(visited.coverage(cell), Some(&before));
}

#[test]
fn unclipped_portal_does_not_consume_invalid_face_setup() {
    let incoming = PortalTileMask::full([0, 0, 1, 1]).unwrap();
    let projection = PortalQueryProjection::new([[f32::NAN; 4]; 4], false);
    let result = projected_link_portal_transition_prepared(
        &[],
        0,
        [0; 4],
        None,
        &projection,
        &[],
        [0, 0, 8, 4],
        &incoming,
    )
    .unwrap();
    assert_eq!(result, Some(([0, 0, 8, 4], PortalCoveragePath::Unclipped)));
}

#[test]
fn cell_traversal_state_matches_original_queued_and_processed_flags() {
    let queued = CellTraversalState::queued(0x123).unwrap();
    assert_eq!(queued.raw(), 0x8123);
    assert_eq!(queued.entry(), 0x123);
    assert!(queued.is_pending());

    let processed = CellTraversalState::processed(0x123).unwrap();
    assert_eq!(processed.raw(), 0x4123);
    assert_eq!(processed.entry(), 0x123);
    assert!(!processed.is_pending());
    assert!(!CellTraversalState::from_raw(0xc123).is_pending());
    assert!(CellTraversalState::queued(0x4000).is_err());
}

#[test]
fn maps_target_spatial_bit_row_to_query_cell_slots() {
    // FUN_140a84f10 starts the row at target_spatial_index * bit_count. For
    // index 1 and three bits, this reads source bits 3..5 (1,0,1).
    let mapping = map_spatial_cell_slots(1, 3, 1, &[0b0010_1000], &[0, 0, 0, 0]).unwrap();
    assert_eq!(mapping.slots, vec![1, 1, 0]);
    assert_eq!(mapping.next_slot, 0);

    // The executable advances each packed-word read by the descriptor's
    // word stride, so this reads words 0 and 2 rather than contiguous 0/1.
    let mapping = map_spatial_cell_slots(0, 33, 2, &[1, 0xffff_ffff, 0], &[0; 40]).unwrap();
    assert_eq!(mapping.slots.len(), 33);
    assert_eq!(mapping.slots.first(), Some(&1));
    assert_eq!(mapping.next_slot, 1);
    assert_eq!(mapping.slots.last(), Some(&1));
}

#[test]
fn target_spatial_slot_cache_reuses_final_slot_and_marks_only_intermediates() {
    let mut mapper = SpatialCellSlotMapper::default();
    let mut states = [0u16; 4];
    assert_eq!(mapper.resolve(1, 3, 1, &[0b0010_1000], &states).unwrap(), 0);
    assert_eq!(mapper.intermediate_slots(), &[1, 1]);

    // Same-cell cache hit returns the stored final slot without reading
    // the now deliberately unavailable bit row.
    assert_eq!(mapper.resolve(1, 3, 1, &[], &states).unwrap(), 0);
    mapper.mark_intermediate_processed(&mut states).unwrap();
    assert_eq!(states, [0, 0x4000, 0, 0]);

    // Requesting another target spatial record rebuilds the one-row cache.
    assert_eq!(mapper.resolve(0, 0, 1, &[], &states).unwrap(), 1);
    assert!(mapper.intermediate_slots().is_empty());
}

#[test]
fn target_spatial_slot_mapping_rejects_incomplete_tables() {
    assert!(map_spatial_cell_slots(0, 1, 0, &[0], &[0; 2]).is_err());
    assert!(map_spatial_cell_slots(1, 32, 1, &[0], &[0; 2]).is_err());
    assert!(map_spatial_cell_slots(0, 3, 1, &[1], &[0, 1]).is_err());
}

#[test]
fn parses_original_spatial_slot_rows_from_tome_header() {
    let mut bytes = vec![0; 0x104];
    bytes[0x90..0x94].copy_from_slice(&3u32.to_le_bytes());
    bytes[0x94..0x98].copy_from_slice(&3u32.to_le_bytes());
    bytes[0x98..0x9c].copy_from_slice(&0x100u32.to_le_bytes());
    bytes[0x100..0x104].copy_from_slice(&0b0010_1000u32.to_le_bytes());

    let rows = SpatialCellSlotRows::parse(&bytes).unwrap();
    assert_eq!(rows.spatial_record_count(), 3);
    assert_eq!(rows.bit_count(), 3);
    let mut mapper = SpatialCellSlotMapper::default();
    let states = [0u16; 4];
    assert_eq!(rows.resolve(&mut mapper, 1, &states).unwrap(), 0);
    assert_eq!(mapper.intermediate_slots(), &[1, 1]);
    assert!(rows.resolve(&mut mapper, 3, &states).is_err());
}

#[test]
fn tests_each_original_portal_pixel_bit_against_object_rectangle() {
    let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    for x in 0..8 {
        for y in 0..4 {
            let bit = if x < 4 {
                y * 4 + x
            } else {
                16 + y * 4 + (x - 4)
            };
            mask.bits[0] = 1 << bit;
            assert!(
                mask.intersects_pixels([x, y, x + 1, y + 1]).unwrap(),
                "bit {bit} at ({x}, {y})"
            );
            if x > 0 {
                assert!(!mask.intersects_pixels([0, y, x, y + 1]).unwrap());
            }
        }
    }
    mask.bits[0] = 0;
    assert!(!mask.intersects_pixels([0, 0, 8, 4]).unwrap());
    assert!(mask.intersects_pixels([0, 0, 9, 4]).is_err());
    let mut offset = PortalTileMask::new([1, 2, 3, 4]).unwrap();
    offset.bits[3] = 1 << (16 + 2 * 4 + 1);
    assert!(offset.intersects_pixels([21, 14, 22, 15]).unwrap());
    assert!(!offset.intersects_pixels([8, 8, 21, 14]).unwrap());
}

#[test]
fn rasterizes_original_small_and_medium_portal_tile_branches() {
    let mut destination = PortalTileMask::new([0, 0, 2, 3]).unwrap();
    let mut incoming = PortalTileMask::new([0, 0, 2, 3]).unwrap();
    incoming.bits.fill(u32::MAX);
    let projected = [[0.0; 4]; 4];
    let face = [[0.0; 4]; 3];
    assert!(destination
        .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
        .unwrap());
    assert_eq!(destination.bits[0], 0x0000_ffff);
    assert!(!destination
        .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
        .unwrap());
    destination.bits[0] = 0x0000_ff00;
    incoming.bits[0] = 0x5555_5555;
    assert!(destination
        .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
        .unwrap());
    assert_eq!(destination.bits[0], 0x0000_ff55);
    assert!(destination
        .rasterize_projected_box(&incoming, [0, 0, 16, 12], projected, face)
        .unwrap());
    incoming.bits[0] = u32::MAX;
    let mut outlined = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    let projected = [
        [-0.9, -0.9, 0.9, 0.9],
        [-0.9, 0.9, 0.9, -0.9],
        [1.0; 4],
        [0.0; 4],
    ];
    assert!(outlined
        .rasterize_projected_box(&incoming, [0, 0, 4, 4], projected, face)
        .unwrap());
    assert_eq!(outlined.bits[0], 0x0000_8000);
}

fn check_original_raster_samples(cases: &str, rect: [u8; 4]) {
    let word_count = (rect[2] - rect[0]) as usize * (rect[3] - rect[1]) as usize;
    for (case, row) in cases.lines().enumerate() {
        let values = row.split_whitespace().collect::<Vec<_>>();
        assert_eq!(values.len(), 33 + word_count, "case {case}");
        let pixels = std::array::from_fn(|axis| values[axis].parse::<i32>().unwrap());
        let projected = std::array::from_fn(|vector| {
            std::array::from_fn(|lane| values[4 + vector * 4 + lane].parse::<f32>().unwrap())
        });
        let face = std::array::from_fn(|vector| {
            std::array::from_fn(|lane| values[20 + vector * 4 + lane].parse::<f32>().unwrap())
        });
        let expected_changed = values[32].parse::<u8>().unwrap() != 0;
        let expected = (0..word_count)
            .map(|index| values[33 + index].parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        let mut destination = PortalTileMask::new(rect).unwrap();
        let mut incoming = PortalTileMask::new(rect).unwrap();
        incoming.bits.fill(u32::MAX);
        let changed = destination
            .rasterize_projected_box(&incoming, pixels, projected, face)
            .unwrap();
        assert_eq!(changed, expected_changed, "case {case} changed");
        assert_eq!(destination.bits, expected, "case {case} mask");
    }
}

#[test]
fn small_and_medium_portal_raster_matches_original_executable_samples() {
    check_original_raster_samples(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/portal_small_cases.txt"
        )),
        [0, 0, 3, 3],
    );
    check_original_raster_samples(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/portal_strip_cases.txt"
        )),
        [0, 0, 6, 11],
    );
    check_original_raster_samples(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/portal_medium_cases.txt"
        )),
        [0, 0, 4, 6],
    );
}

#[test]
fn large_portal_raster_matches_original_executable_samples() {
    check_original_raster_samples(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/portal_large_cases.txt"
        )),
        [0, 0, 7, 13],
    );
    let rect = [0, 0, 8, 16];
    let word_count = 8 * 16;
    for (case, row) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/portal_large_varied_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values = row.split_whitespace().collect::<Vec<_>>();
        assert_eq!(values.len(), 33 + word_count * 3, "case {case}");
        let pixels = std::array::from_fn(|axis| values[axis].parse::<i32>().unwrap());
        let projected = std::array::from_fn(|vector| {
            std::array::from_fn(|lane| values[4 + vector * 4 + lane].parse::<f32>().unwrap())
        });
        let face = std::array::from_fn(|vector| {
            std::array::from_fn(|lane| values[20 + vector * 4 + lane].parse::<f32>().unwrap())
        });
        let mut incoming = PortalTileMask::new(rect).unwrap();
        let mut destination = PortalTileMask::new(rect).unwrap();
        for index in 0..word_count {
            incoming.bits[index] = values[32 + index].parse().unwrap();
            destination.bits[index] = values[32 + word_count + index].parse().unwrap();
        }
        let changed = destination
            .rasterize_projected_box(&incoming, pixels, projected, face)
            .unwrap();
        let expected_changed = values[32 + word_count * 2].parse::<u8>().unwrap() != 0;
        assert_eq!(changed, expected_changed, "case {case} changed");
        let expected = (0..word_count)
            .map(|index| values[33 + word_count * 2 + index].parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(destination.bits, expected, "case {case} mask");
    }
}

#[test]
fn depth_grid_matches_original_executable_samples() {
    for (case, row) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/depth_grid_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values = row.split_whitespace().collect::<Vec<_>>();
        let x0 = values[0].parse::<i32>().unwrap();
        let y0 = values[1].parse::<i32>().unwrap();
        let x1 = values[2].parse::<i32>().unwrap();
        let y1 = values[3].parse::<i32>().unwrap();
        let threshold = values[4].parse::<f32>().unwrap();
        let count = values[5].parse::<usize>().unwrap();
        assert_eq!(values.len(), 7 + count * 3, "case {case}");
        let mut depths = vec![0.0f32; 64 * 64];
        for point in 0..count {
            let at = 6 + point * 3;
            let x = values[at].parse::<usize>().unwrap();
            let y = values[at + 1].parse::<usize>().unwrap();
            let tile = (y >> 2) * 16 + (x >> 2);
            depths[tile * 16 + ((y & 3) << 2) + (x & 3)] = values[at + 2].parse::<f32>().unwrap();
        }
        let expected = values[6 + count * 3].parse::<u8>().unwrap() != 0;
        let actual = depth_grid_intersects_pixels(&depths, [x0, y0], [x1, y1], threshold).unwrap();
        assert_eq!(actual, expected, "case {case}");
    }
}

#[test]
fn combined_mask_depth_matches_original_executable_samples() {
    for (case, row) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mask_depth_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values = row
            .split_whitespace()
            .map(|number| number.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        let pixels = [
            values[0] as i32,
            values[1] as i32,
            values[2] as i32,
            values[3] as i32,
        ];
        let threshold = f32::from_bits(values[4]);
        let mask_count = values[5] as usize;
        let mut mask = PortalTileMask::new([0, 0, 8, 16]).unwrap();
        for point in 0..mask_count {
            let x = values[6 + point * 2] as usize;
            let y = values[7 + point * 2] as usize;
            let index = (y / 4) * 8 + x / 8;
            let bit = ((x / 4) & 1) * 16 + (y & 3) * 4 + (x & 3);
            mask.bits[index] |= 1 << bit;
        }
        let depth_count_at = 6 + mask_count * 2;
        let depth_count = values[depth_count_at] as usize;
        let mut depths = vec![0.0f32; 64 * 64];
        for point in 0..depth_count {
            let at = depth_count_at + 1 + point * 3;
            let x = values[at] as usize;
            let y = values[at + 1] as usize;
            let slot = ((y / 4) * 16 + x / 4) * 16 + (y & 3) * 4 + (x & 3);
            depths[slot] = f32::from_bits(values[at + 2]);
        }
        let expected_at = depth_count_at + 1 + depth_count * 3;
        assert_eq!(values.len(), expected_at + 1, "case {case}");
        let actual = mask
            .intersects_depth_pixels(&depths, pixels, threshold)
            .unwrap();
        assert_eq!(actual, values[expected_at] != 0, "case {case}");
    }
}

#[test]
fn depth_mask_raster_matches_original_executable_samples() {
    for (case, row) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/depth_raster_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let numbers = row
            .split_whitespace()
            .map(|number| number.parse::<u64>().unwrap())
            .collect::<Vec<_>>();
        let rect = [
            numbers[0] as u8,
            numbers[1] as u8,
            numbers[2] as u8,
            numbers[3] as u8,
        ];
        let value = f32::from_bits(numbers[4] as u32);
        let previous = f32::from_bits(numbers[5] as u32);
        let count = numbers[6] as usize;
        let bits = numbers[7..7 + count]
            .iter()
            .map(|&word| word as u32)
            .collect::<Vec<_>>();
        let points = numbers[7 + count] as usize;
        assert_eq!(numbers.len(), 10 + count + points * 2, "case {case}");
        let mut depths = vec![0.0f32; 64 * 64];
        for point in 0..points {
            let at = 8 + count + point * 2;
            depths[numbers[at] as usize] = f32::from_bits(numbers[at + 1] as u32);
        }
        let maximum = rasterize_depth_mask(&mut depths, rect, &bits, value, previous).unwrap();
        assert_eq!(
            maximum.to_bits(),
            numbers[8 + count + points * 2] as u32,
            "case {case} maximum"
        );
        let digest = depths.iter().fold(0xcbf29ce484222325u64, |hash, depth| {
            (hash ^ depth.to_bits() as u64).wrapping_mul(0x100000001b3)
        });
        assert_eq!(digest, numbers[9 + count + points * 2], "case {case} grid");
    }
}

#[test]
fn decodes_original_grouped_tome_object_ids() {
    let mut data = vec![0u8; 0x90];
    data[0x40..0x44].copy_from_slice(&3u32.to_le_bytes());
    data[0x4c..0x50].copy_from_slice(&0x60u32.to_le_bytes());
    data[0x50..0x54].copy_from_slice(&0x70u32.to_le_bytes());
    for (i, offset) in [0u32, 2, 2, 3].iter().enumerate() {
        data[0x60 + i * 4..0x64 + i * 4].copy_from_slice(&offset.to_le_bytes());
    }
    for (i, id) in [11u32, 12, 99].iter().enumerate() {
        data[0x70 + i * 4..0x74 + i * 4].copy_from_slice(&id.to_le_bytes());
    }
    assert_eq!(object_ids_for_index(&data, 0).unwrap(), [11, 12]);
    assert!(object_ids_for_index(&data, 1).unwrap().is_empty());
    assert_eq!(object_ids_for_index(&data, 2).unwrap(), [99]);
    assert!(object_ids_for_index(&data, 3).is_err());
    data[0x6c..0x70].copy_from_slice(&1u32.to_le_bytes());
    assert!(object_ids_for_index(&data, 2).is_err());
}

#[test]
fn reconstructs_original_four_corner_order_from_query_planes() {
    let planes = [
        [0.0, 0.0, 1.0, -1.0],
        [0.0, 0.0, 1.0, -2.0],
        [1.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, -1.0],
    ];
    assert_eq!(
        first_depth_quad(planes).unwrap(),
        [
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, -1.0, 1.0],
        ]
    );
    let mut degenerate = planes;
    degenerate[4] = degenerate[2];
    assert!(first_depth_quad(degenerate).is_err());
}

#[test]
fn builds_original_render_view_reverse_depth_projection() {
    let finite = original_render_perspective_rows([-1.0, 1.0], [-1.0, 1.0], 1.0, 100.0).unwrap();
    assert_eq!(finite[0], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(finite[1], [0.0, 1.0, 0.0, 0.0]);
    assert_eq!(finite[3], [0.0, 0.0, -1.0, 0.0]);
    let project_depth = |z: f32| {
        let clip_z = finite[2][2] * z + finite[2][3];
        clip_z / -z
    };
    assert!((project_depth(-1.0) - 1.0).abs() < 1e-6);
    assert!(project_depth(-100.0).abs() < 1e-6);

    let infinite = original_render_perspective_rows([-1.0, 1.0], [-1.0, 1.0], 1.0, 0.0).unwrap();
    assert_eq!(infinite[2], [0.0, 0.0, -1.0, 0.0]);
    assert!(original_render_perspective_rows([0.0, 0.0], [-1.0, 1.0], 1.0, 100.0).is_err());
}

#[test]
fn world_visibility_query_replaces_depth_and_flips_portal_winding() {
    let pose = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let reverse = original_render_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
    let visibility =
        original_world_visibility_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
    assert_eq!(visibility[0], reverse[0]);
    assert_eq!(visibility[1], reverse[1]);
    assert_eq!(visibility[3], reverse[3]);
    assert!((visibility[2][2] + 7000.0 / 6999.99).abs() < 1e-6);
    assert!((visibility[2][3] + 70.0 / 6999.99).abs() < 1e-6);
    assert!(!query_matrix_winding_flip(reverse));
    assert!(query_matrix_winding_flip(visibility));
}

#[test]
fn derives_original_render_fov_from_reference_fov_and_pixel_aspect() {
    let (wide_horizontal, wide_vertical) =
        original_render_fov_radians(80.0, 1920, 1080, 1.0).unwrap();
    assert!((wide_horizontal.to_degrees() - 80.0).abs() < 1e-4);
    let expected_vertical = 2.0 * ((80.0_f32.to_radians() * 0.5).tan() * 0.5625).atan();
    assert!((wide_vertical - expected_vertical).abs() < 1e-6);

    let (narrow_horizontal, narrow_vertical) =
        original_render_fov_radians(80.0, 4, 3, 1.0).unwrap();
    assert!(narrow_horizontal < wide_horizontal);
    assert!((narrow_vertical - wide_vertical).abs() < 1e-6);

    assert!(original_render_fov_radians(80.0, 0, 1080, 1.0).is_err());
    assert!(original_render_fov_radians(80.0, 1920, 1080, 0.0).is_err());
    assert!(original_render_fov_radians(180.0, 1920, 1080, 1.0).is_err());
}

#[test]
fn assembles_query_planes_from_original_matrix_rows() {
    let rows = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, -1.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let planes = query_planes_from_rows(rows, rows);
    assert_eq!(
        planes,
        [
            [0.0, 0.0, 1.0, -1.0],
            [0.0, 0.0, -1.0, 2.0],
            [-1.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
        ]
    );
    assert_eq!(
        first_depth_quad(planes).unwrap(),
        [
            [1.0, 1.0, 1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ]
    );
}

#[test]
fn cell_depth_output_matches_full_original_cell_worker() {
    let mask = PortalTileMask::full([0, 0, 8, 16]).unwrap();
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cell_query_depth_cases.txt"
    ));
    for (case, line) in fixture.lines().enumerate() {
        let words: Vec<u64> = line
            .split_whitespace()
            .map(|word| word.parse().unwrap())
            .collect();
        let f = |index: usize| f32::from_bits(words[index] as u32);
        let root = std::array::from_fn(|side| std::array::from_fn(|axis| f(8 + side * 3 + axis)));
        let quantized = std::array::from_fn(|side| {
            std::array::from_fn(|axis| words[2 + side * 3 + axis] as u16)
        });
        let camera_imprecision = std::array::from_fn(|axis| f(15 + axis));
        let bounds =
            quantized_cell_query_bounds(root, quantized, f(14), camera_imprecision).unwrap();
        let render_rows =
            std::array::from_fn(|row| std::array::from_fn(|column| f(18 + row * 4 + column)));
        let state = RenderQueryState {
            render_rows,
            cropped_rows: crop_query_rows(
                render_rows,
                render_query_job_tile(words[0] as u32, words[1] as u32).unwrap(),
            )
            .unwrap(),
            camera_point: [0.0; 3],
            camera_imprecision,
            winding_flip: false,
            viewport: render_query_job_tile(words[0] as u32, words[1] as u32).unwrap(),
            maximum_depth: 0.0,
            depth_grid: None,
        };
        let count = words[66] as usize;
        assert_eq!(words.len(), 69 + count, "native cell depth case {case}");
        let active = words[67..67 + count]
            .iter()
            .map(|&word| word as usize)
            .collect::<Vec<_>>();
        let query = RenderAabbQuery::from_state(&state).unwrap();
        let native_planes: [[u32; 4]; 6] = std::array::from_fn(|plane| {
            std::array::from_fn(|axis| words[34 + plane * 4 + axis] as u32)
        });
        let native_bounds: [[u32; 4]; 2] = std::array::from_fn(|side| {
            std::array::from_fn(|axis| words[58 + side * 4 + axis] as u32)
        });
        assert_eq!(
            query.planes.map(|row| row.map(f32::to_bits)),
            native_planes,
            "native cell query planes case {case}"
        );
        assert_eq!(
            query.query_bounds.map(|row| row.map(f32::to_bits)),
            native_bounds,
            "native cell query bounds case {case}"
        );
        let value = query.cell_depth_value(bounds, &active).unwrap();
        let mut grid = [0.0f32; 4096];
        let maximum = rasterize_depth_mask(&mut grid, mask.rect, &mask.bits, value, 0.0).unwrap();
        assert_eq!(
            maximum.to_bits(),
            words[67 + count] as u32,
            "native cell depth maximum case {case}"
        );
        let digest = grid.iter().fold(0xcbf29ce484222325u64, |hash, depth| {
            (hash ^ u64::from(depth.to_bits())).wrapping_mul(0x100000001b3)
        });
        assert_eq!(
            digest,
            words[68 + count],
            "native cell depth grid case {case}"
        );
    }
    assert_eq!(fixture.lines().count(), 1440);
}

#[test]
fn depth_enabled_aabb_matches_installed_executable() {
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/depth_aabb_cases.txt"
    ));
    assert_eq!(fixture.lines().count(), 9000);
    for (line_number, line) in fixture.lines().enumerate() {
        let words: Vec<u32> = line
            .split_whitespace()
            .map(|word| word.parse().unwrap())
            .collect();
        assert_eq!(words.len(), 64);
        let viewport = std::array::from_fn(|axis| words[3 + axis] as i32);
        let render_rows = std::array::from_fn(|row| {
            std::array::from_fn(|column| f32::from_bits(words[10 + row * 4 + column]))
        });
        let grid = std::array::from_fn(|i| match words[0] {
            0 => 0.0,
            1 => f32::from_bits(0x3e4c_cccd),
            2 => 1.0,
            3 => (i * 7 % 101) as f32 / 100.0,
            4 => {
                if i % 67 == 0 {
                    1.0
                } else {
                    0.0
                }
            }
            _ => panic!("unknown native fixture grid pattern"),
        });
        let state = RenderQueryState {
            render_rows,
            cropped_rows: crop_query_rows(render_rows, viewport).unwrap(),
            camera_point: [0.0; 3],
            camera_imprecision: std::array::from_fn(|axis| f32::from_bits(words[7 + axis])),
            winding_flip: false,
            viewport,
            maximum_depth: 0.0,
            depth_grid: Some(Box::new(grid)),
        };
        let bounds = std::array::from_fn(|side| {
            std::array::from_fn(|axis| f32::from_bits(words[26 + side * 3 + axis]))
        });
        let query = RenderAabbQuery::from_state(&state).unwrap();
        let native_planes: [[f32; 4]; 6] = std::array::from_fn(|plane| {
            std::array::from_fn(|axis| f32::from_bits(words[32 + plane * 4 + axis]))
        });
        let native_bounds: [[f32; 4]; 2] = std::array::from_fn(|side| {
            std::array::from_fn(|axis| f32::from_bits(words[56 + side * 4 + axis]))
        });
        assert_eq!(
            query.planes, native_planes,
            "depth fixture {line_number}: query planes"
        );
        assert_eq!(
            query.query_bounds, native_bounds,
            "depth fixture {line_number}: query bounds"
        );
        let (visible, area) = query.visibility_and_area(bounds).unwrap();
        assert_eq!(
            visible,
            words[1] != 0,
            "depth fixture {line_number}: {bounds:?}"
        );
        assert_eq!(
            area.to_bits(),
            words[2],
            "depth fixture {line_number}: area {area}"
        );
        assert_eq!(query.visible(bounds).unwrap(), visible);
    }
}

#[test]
fn no_depth_light_aabb_area_matches_installed_instructions() {
    let mut cases = 0;
    for (line_number, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/light_aabb_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        assert_eq!(fields.len(), 77, "native case {}", line_number + 1);
        let f = |index: usize| fields[index].parse::<f32>().unwrap();
        let initialized = fields[3] == "1";
        let expected_visible = fields[11] == "1";
        let expected_area = fields[12].parse::<u32>().unwrap();
        if initialized {
            let padding = f(4);
            let expanded = [
                [f(5) - padding, f(6) - padding, f(7) - padding, 1.0],
                [f(8) + padding, f(9) + padding, f(10) + padding, 1.0],
            ];
            let query_bounds =
                std::array::from_fn(|side| std::array::from_fn(|axis| f(13 + side * 4 + axis)));
            let planes = (0..6)
                .map(|plane| std::array::from_fn(|axis| f(21 + plane * 4 + axis)))
                .collect::<Vec<_>>();
            let rows =
                std::array::from_fn(|row| std::array::from_fn(|axis| f(45 + row * 4 + axis)));
            let (visible, area) =
                query_aabb_visibility_without_depth(query_bounds, &planes, expanded, rows).unwrap();
            assert_eq!(
                visible,
                expected_visible,
                "native visibility case {}",
                line_number + 1
            );
            assert_eq!(
                area.to_bits(),
                expected_area,
                "native area case {}: {area}",
                line_number + 1
            );
            let render_rows =
                std::array::from_fn(|row| std::array::from_fn(|axis| f(61 + row * 4 + axis)));
            assert_eq!(
                rows,
                std::array::from_fn(|column| std::array::from_fn(|row| render_rows[row][column]))
            );
            let state = RenderQueryState {
                render_rows,
                cropped_rows: crop_query_rows(
                    render_rows,
                    render_query_job_tile(fields[1].parse().unwrap(), fields[2].parse().unwrap())
                        .unwrap(),
                )
                .unwrap(),
                camera_point: [0.0; 3],
                camera_imprecision: [padding; 3],
                winding_flip: false,
                viewport: [0, 0, 64, 64],
                maximum_depth: 0.0,
                depth_grid: None,
            };
            let bounds = [[f(5), f(6), f(7)], [f(8), f(9), f(10)]];
            let prepared = NoDepthAabbQuery::from_state(&state).unwrap();
            assert_eq!(prepared.visible(bounds).unwrap(), expected_visible);
            let (prepared_visible, prepared_area) = prepared.visibility_and_area(bounds).unwrap();
            assert_eq!(prepared_visible, expected_visible);
            assert_eq!(prepared_area.to_bits(), expected_area);
            let (visible, area) =
                render_query_aabb_visibility_without_depth(Some(&state), bounds).unwrap();
            assert_eq!(
                visible,
                expected_visible,
                "typed native visibility case {}",
                line_number + 1
            );
            assert_eq!(
                area.unwrap().to_bits(),
                expected_area,
                "typed native area case {}",
                line_number + 1
            );
            assert_eq!(
                render_query_aabb_visible_without_depth(Some(&state), bounds).unwrap(),
                expected_visible
            );
        } else {
            assert!(expected_visible);
            assert_eq!(expected_area, 123.0f32.to_bits());
            assert_eq!(
                render_query_aabb_visibility_without_depth(None, [[0.0; 3]; 2]).unwrap(),
                (true, None)
            );
        }
        cases += 1;
    }
    assert_eq!(cases, 4800);
}

#[test]
#[ignore = "manual CPU measurement of descriptor setup lifetime"]
fn prepared_spatial_query_cpu_measurement() {
    use std::{hint::black_box, time::Instant};
    let line = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/light_aabb_cases.txt"
    ))
    .lines()
    .find(|line| line.starts_with("1 0 1 1 0.0 "))
    .unwrap();
    let fields: Vec<_> = line.split_whitespace().collect();
    let f = |i: usize| fields[i].parse::<f32>().unwrap();
    let rows = std::array::from_fn(|r| std::array::from_fn(|a| f(61 + r * 4 + a)));
    let state = RenderQueryState {
        render_rows: rows,
        cropped_rows: rows,
        camera_point: [0.0; 3],
        camera_imprecision: [0.0; 3],
        winding_flip: false,
        viewport: [0, 0, 64, 64],
        maximum_depth: 0.0,
        depth_grid: None,
    };
    let bounds = [[f(5), f(6), f(7)], [f(8), f(9), f(10)]];
    let count = 100_000;
    let start = Instant::now();
    let mut repeated = 0;
    for _ in 0..count {
        repeated += usize::from(
            render_query_aabb_visible_without_depth(Some(black_box(&state)), black_box(bounds))
                .unwrap(),
        );
    }
    let repeated_time = start.elapsed();
    let start = Instant::now();
    let query = NoDepthAabbQuery::from_state(black_box(&state)).unwrap();
    let mut prepared = 0;
    for _ in 0..count {
        prepared += usize::from(query.visible(black_box(bounds)).unwrap());
    }
    let prepared_time = start.elapsed();
    assert_eq!(repeated, prepared);
    eprintln!("spatial CPU synthetic {count} boxes: per-object setup={repeated_time:?}, once-per-frame setup={prepared_time:?}; identical bools; not FPS");
}

#[test]
fn query_corners_and_bounds_match_installed_instructions() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/query_corner_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        let f = |i: usize| fields[i].parse::<f32>().unwrap();
        let finite = fields[1] == "1";
        assert_eq!(fields.len(), if finite { 58 } else { 34 });
        let planes = std::array::from_fn(|p| std::array::from_fn(|axis| f(2 + p * 4 + axis)));
        let expected: [[f32; 4]; 2] =
            std::array::from_fn(|side| std::array::from_fn(|axis| f(26 + side * 4 + axis)));
        let bounds = query_bounds_from_planes(&planes, [f(0); 3]).unwrap();
        assert_eq!(
            bounds.map(|row| row.map(f32::to_bits)),
            expected.map(|row| row.map(f32::to_bits)),
            "native bounds case {checked}"
        );
        if finite {
            for index in 0..8 {
                let expected: [f32; 3] = std::array::from_fn(|axis| f(34 + index * 3 + axis));
                assert_eq!(
                    query_frustum_corner(&planes, index)
                        .unwrap()
                        .map(f32::to_bits),
                    expected.map(f32::to_bits),
                    "native corner case {checked}/{index}"
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 60);
    assert!(query_frustum_corner(&[[0.0; 4]; 6], 8).is_err());
    assert!(query_bounds_from_planes(&[[0.0; 4]; 6], [f32::NAN; 3]).is_err());
}

#[test]
fn light_area_threshold_uses_the_original_64_numerator() {
    assert_eq!(
        minimum_light_projected_area_metric(1920, 1080)
            .unwrap()
            .to_bits(),
        (64.0f32 / 2_073_600.0).to_bits()
    );
    assert!(minimum_light_projected_area_metric(0, 1080).is_err());
    assert!(minimum_light_projected_area_metric(1920, i32::MAX).is_err());
}

#[test]
fn selects_active_query_planes_using_original_sign_masks() {
    let planes = [
        [1.0, 0.0, 0.0, -2.0],
        [-1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, -1.0],
        [0.0, 0.0, 1.0, -3.0],
    ];
    let bounds = [[0.0, 0.0, 0.0, 1.0], [1.0, 2.0, 4.0, 1.0]];
    assert_eq!(active_query_planes(&planes, bounds), [0, 1, 2, 3]);
}

#[test]
fn active_query_planes_match_original_executable_cases() {
    let mut cases = 0;
    for (line_number, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/active_plane_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values = line
            .split_whitespace()
            .map(|value| value.parse::<f32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 33, "fixture row {}", line_number + 1);
        let bounds = [
            values[0..4].try_into().unwrap(),
            values[4..8].try_into().unwrap(),
        ];
        let planes = values[8..32]
            .chunks_exact(4)
            .map(|plane| plane.try_into().unwrap())
            .collect::<Vec<[f32; 4]>>();
        let expected_mask = values[32] as u32;
        let actual_mask = active_query_planes(&planes, bounds)
            .into_iter()
            .fold(0, |mask, index| mask | (1 << index));
        assert_eq!(
            actual_mask,
            expected_mask,
            "fixture row {}",
            line_number + 1
        );
        cases += 1;
    }
    assert_eq!(cases, 256);
}

#[test]
fn crops_query_matrix_on_original_sixty_four_unit_grid() {
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    assert_eq!(crop_query_rows(identity, [0, 0, 64, 64]).unwrap(), identity);
    let left_half = crop_query_rows(identity, [0, 0, 32, 64]).unwrap();
    assert_eq!(
        left_half,
        [
            [2.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    );
    let planes = query_planes_from_rows(identity, left_half);
    assert_eq!(planes[2], [-2.0, 0.0, 0.0, 0.0]);
    assert_eq!(planes[3], [2.0, 0.0, 0.0, 2.0]);
    assert!(crop_query_rows(identity, [0, 0, 0, 64]).is_err());
}

#[test]
fn render_job_partition_and_crop_match_installed_executable() {
    let cases = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/query_job_cases.txt"
    ));
    let identity = std::array::from_fn(|row| {
        std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
    });
    let mut checked = 0;
    for (case, line) in cases.lines().enumerate() {
        let values: Vec<f32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(values.len(), 22);
        let tile = render_query_job_tile(values[1] as u32, values[0] as u32).unwrap();
        assert_eq!(
            tile,
            std::array::from_fn(|axis| values[2 + axis] as i32),
            "case {case}"
        );
        let cropped = crop_query_rows(identity, tile).unwrap();
        let planes = query_planes_from_rows(identity, cropped);
        for (actual, expected) in planes[2..].iter().flatten().zip(&values[6..]) {
            assert!(
                (actual - expected).abs() <= 1e-6,
                "case {case}: {actual} != {expected}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 136);
}

#[test]
fn render_query_state_merge_matches_installed_executable() {
    fn grid(source: bool) -> Box<[f32; 4096]> {
        Box::new(std::array::from_fn(|i| {
            if i % 29 == 0 {
                f32::from_bits(if source { 0x7fc01234 } else { 0x7fc05678 })
            } else if i % 17 == 0 {
                f32::from_bits(if source { 0 } else { 0x80000000 })
            } else {
                ((i * if source { 17 } else { 13 }) % 257) as f32 / 64.0 - 2.0
            }
        }))
    }
    fn hash_grid(grid: &[f32; 4096]) -> u64 {
        grid.iter().fold(0xcbf29ce484222325, |hash, value| {
            value
                .to_bits()
                .to_le_bytes()
                .iter()
                .fold(hash, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
                })
        })
    }
    let identity = std::array::from_fn(|row| {
        std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
    });
    let modes = [
        (false, true, false, false),
        (false, true, false, true),
        (true, false, true, true),
        (true, true, true, true),
        (true, true, true, false),
        (true, true, false, true),
        (true, true, true, true),
        (true, true, true, true),
        (false, false, false, false),
    ];
    let mut checked = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/query_state_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values: Vec<u64> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let count = values[0] as u32;
        let job = values[1] as u32;
        let mode = values[2] as usize;
        let (destination_valid, source_valid, destination_grid, source_grid) = modes[mode];
        let make = |source: bool, has_grid: bool| {
            let viewport =
                render_query_job_tile(if source { job } else { 0 }, if source { count } else { 4 })
                    .unwrap();
            let rows = if source && job % 2 != 0 {
                [
                    [2.0, 0.0, 0.0, 0.0],
                    [0.0, 3.0, 0.0, 0.0],
                    [0.0, 0.0, 4.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ]
            } else {
                identity
            };
            RenderQueryState {
                render_rows: rows,
                cropped_rows: crop_query_rows(rows, viewport).unwrap(),
                camera_point: if source {
                    [1.0, 2.0, 3.0]
                } else {
                    [-1.0, -2.0, -3.0]
                },
                camera_imprecision: [0.0; 3],
                winding_flip: false,
                viewport,
                maximum_depth: if mode == 6 && !source {
                    f32::from_bits(0x7fc12345)
                } else if mode == 7 {
                    f32::from_bits(if source { 0 } else { 0x80000000 })
                } else if source {
                    0.75
                } else {
                    0.25
                },
                depth_grid: has_grid.then(|| grid(source)),
            }
        };
        let mut destination = destination_valid.then(|| make(false, destination_grid));
        let source = source_valid.then(|| make(true, source_grid));
        merge_render_query_state(&mut destination, source.as_ref()).unwrap();
        assert_eq!(destination.is_some(), values[3] != 0, "case {case}");
        if let Some(destination) = destination {
            assert_eq!(
                destination.viewport,
                std::array::from_fn(|i| values[4 + i] as i32),
                "case {case}"
            );
            assert_eq!(
                u64::from(destination.maximum_depth.to_bits()),
                values[8],
                "case {case}"
            );
            assert_eq!(
                destination.camera_point.map(|v| u64::from(v.to_bits())),
                [values[9], values[10], values[11]],
                "case {case}"
            );
            assert_eq!(
                destination.depth_grid.is_some(),
                values[12] != 0,
                "case {case}"
            );
            if let Some(grid) = &destination.depth_grid {
                assert_eq!(hash_grid(grid), values[13], "entire grid case {case}");
                if !destination_valid {
                    assert_ne!(
                        grid.as_ptr(),
                        source
                            .as_ref()
                            .unwrap()
                            .depth_grid
                            .as_ref()
                            .unwrap()
                            .as_ptr()
                    );
                }
            }
            let planes = query_planes_from_rows(destination.render_rows, destination.cropped_rows);
            for (actual, expected) in planes[2..].iter().flatten().zip(&values[14..]) {
                assert!(
                    (actual - f32::from_bits(*expected as u32)).abs() <= 1e-6,
                    "plane case {case}"
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 342);
}

#[test]
fn builds_original_row_major_query_matrix_from_camera_pose() {
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let pose = [
        [0.0, -1.0, 0.0, 10.0],
        [1.0, 0.0, 0.0, 20.0],
        [0.0, 0.0, 1.0, 30.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    assert_eq!(
        render_query_base_rows(identity, pose),
        [
            [0.0, 1.0, 0.0, -20.0],
            [-1.0, 0.0, 0.0, 10.0],
            [0.0, 0.0, 1.0, -30.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    );
}

#[test]
fn composes_game_default_projection_with_original_camera_pose() {
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let matrix =
        original_render_query_matrix(80.0, 1920, 1080, 1.0, identity, 0.01, 7000.0).unwrap();
    assert!(matrix[0][0] > 0.0);
    assert!(matrix[1][1] > matrix[0][0]);
    assert_eq!(matrix[3], [0.0, 0.0, -1.0, 0.0]);
    assert!(original_render_query_matrix(80.0, 1920, 1080, 1.0, identity, 0.0, 7000.0).is_err());
}

#[test]
fn partitions_render_query_jobs_on_original_grid() {
    assert_eq!(render_query_job_tile(0, 1).unwrap(), [0, 0, 64, 64]);
    assert_eq!(render_query_job_tile(0, 2).unwrap(), [0, 0, 32, 64]);
    assert_eq!(render_query_job_tile(1, 2).unwrap(), [32, 0, 64, 64]);
    assert_eq!(render_query_job_tile(1, 3).unwrap(), [32, 0, 48, 64]);
    for count in 1..=16 {
        let mut covered = [[false; 64]; 64];
        for job in 0..count {
            let [left, top, right, bottom] = render_query_job_tile(job, count).unwrap();
            assert!(0 <= left && left < right && right <= 64);
            assert!(0 <= top && top < bottom && bottom <= 64);
            for y in top..bottom {
                for x in left..right {
                    let cell = &mut covered[y as usize][x as usize];
                    assert!(!*cell, "job tiles overlap for count {count}");
                    *cell = true;
                }
            }
        }
        assert!(
            covered.iter().flatten().all(|&cell| cell),
            "job tiles leave a gap for count {count}"
        );
    }
    assert!(render_query_job_tile(0, 0).is_err());
    assert!(render_query_job_tile(16, 16).is_err());
}

#[test]
fn original_render_job_settings_and_inputs() {
    assert_eq!(ORIGINAL_DEFAULT_QUERY_JOB_COUNT, 12);
    assert_eq!(ORIGINAL_DEFAULT_SCHEDULER_THREADING_MODE, 2);
    assert_eq!(
        original_world_query_job_count(ORIGINAL_DEFAULT_QUERY_JOB_COUNT, 2).unwrap(),
        12
    );
    assert_eq!(original_world_query_job_count(1, 2).unwrap(), 1);
    assert_eq!(original_world_query_job_count(32, 2).unwrap(), 16);
    assert_eq!(original_world_query_job_count(16, 0).unwrap(), 1);
    assert_eq!(original_world_query_job_count(4, 1).unwrap(), 4);
    assert!(original_world_query_job_count(0, 2).is_err());
    assert!(original_world_query_job_count(33, 2).is_err());
    assert!(original_world_query_job_count(1, 3).is_err());
    let rows = [
        [2.0, 0.0, 0.0, 3.0],
        [0.0, 4.0, 0.0, 5.0],
        [0.0, 0.0, 6.0, 7.0],
        [0.0, 0.0, -1.0, 0.0],
    ];
    let input = OrdinaryDirectQueryInput {
        camera_point: [1.0, 2.0, 3.0],
        render_rows: rows,
        cropped_rows: rows,
        initial_tile_rect: [0, 0, 8, 16],
        camera_imprecision: [0.6; 3],
        object_imprecision: 0.6,
        minimum_area_metric: 0.1,
        global_distance_threshold: ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
        object_distance_scale: 1.0,
        winding_flip: false,
        gate_states: None,
        output_capacity: ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
    };
    for count in 1..=16 {
        for job in 0..count {
            let tile = render_query_job_tile(job, count).unwrap();
            let split = input.for_render_job(job, count).unwrap();
            assert_eq!(split.render_rows, rows);
            assert_eq!(split.cropped_rows, crop_query_rows(rows, tile).unwrap());
            assert_eq!(split.initial_tile_rect, query_viewport_tile_bounds(tile));
            assert_eq!(split.minimum_area_metric, input.minimum_area_metric);
            assert_eq!(split.camera_point, input.camera_point);
            assert_eq!(split.output_capacity, 65536);
        }
    }
}

#[test]
fn render_job_merge_keeps_order_metrics_and_clears_failed_jobs() {
    let mut first = VisibleObjectIndexOutput::new(4, 4);
    first.push_unique_with_area_metric(1, Some(0.25)).unwrap();
    let mut second = VisibleObjectIndexOutput::new(4, 4);
    second.push_unique_with_area_metric(1, Some(0.5)).unwrap();
    second.push_unique_with_area_metric(2, Some(0.75)).unwrap();
    first.cells = Some(VisibleCellIndexOutput::new(4, 4));
    first.cells.as_mut().unwrap().push_unique(3).unwrap();
    second.cells = Some(VisibleCellIndexOutput::new(4, 4));
    second.cells.as_mut().unwrap().push_unique(3).unwrap();
    second.cells.as_mut().unwrap().push_unique(0).unwrap();
    let make_job =
        |job_index, result: Result<Option<VisibleObjectIndexOutput>, String>| RenderQueryJob {
            diagnostics: RenderQueryJobDiagnostics {
                job_index,
                viewport: None,
                elapsed_ns: 0,
                progress: OrdinaryQueryDiagnostics::default(),
                error: result.as_ref().err().cloned(),
            },
            state: None,
            result,
        };
    let mut jobs = vec![make_job(0, Ok(Some(first))), make_job(1, Ok(Some(second)))];
    let rows = std::array::from_fn(|row| {
        std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
    });
    for (index, job) in jobs.iter_mut().enumerate() {
        let viewport = render_query_job_tile(index as u32, 2).unwrap();
        job.state = Some(RenderQueryState {
            render_rows: rows,
            cropped_rows: crop_query_rows(rows, viewport).unwrap(),
            camera_point: [0.0; 3],
            camera_imprecision: [0.6; 3],
            winding_flip: false,
            viewport,
            maximum_depth: 0.0,
            depth_grid: None,
        });
    }
    let output = merge_render_query_outputs(&jobs).unwrap().unwrap();
    assert_eq!(output.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
    assert_eq!(output.state.as_ref().unwrap().cropped_rows, rows);
    assert_eq!(output.indices(), &[1, 1, 2]);
    assert_eq!(output.area_metrics(), &[Some(0.25), Some(0.5), Some(0.75)]);
    assert_eq!(output.cell_indices(), &[3, 3, 0]);
    jobs[1].result = Err("late object-pass failure".into());
    let output = merge_render_query_outputs(&jobs).unwrap().unwrap();
    assert_eq!(output.indices(), &[1]);
    assert_eq!(output.area_metrics(), &[Some(0.25)]);
    assert_eq!(output.cell_indices(), &[3]);
    assert_eq!(output.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
    jobs[0].result = Err("first worker also failed".into());
    assert!(merge_render_query_outputs(&jobs)
        .unwrap()
        .unwrap()
        .indices()
        .is_empty());
    jobs[0].result = Ok(None);
    jobs[1].result = Ok(None);
    let empty = merge_render_query_outputs(&jobs).unwrap().unwrap();
    assert!(empty.indices().is_empty());
    assert!(empty.area_metrics().is_empty());
    assert_eq!(empty.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
    jobs[0].state = None;
    jobs[1].state = None;
    assert!(merge_render_query_outputs(&jobs).unwrap().is_none());
    jobs.swap(0, 1);
    assert!(merge_render_query_outputs(&jobs).is_err());
}

#[test]
fn render_job_merge_grows_beyond_per_worker_query_capacity() {
    let capacity = ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY;
    let jobs: [_; 2] = std::array::from_fn(|job| {
        let mut output = VisibleObjectIndexOutput::new(capacity, capacity);
        for index in 0..capacity {
            output
                .push_unique_with_area_metric(index as u32, Some(job as f32 + 0.25))
                .unwrap();
        }
        RenderQueryJob {
            diagnostics: RenderQueryJobDiagnostics {
                job_index: job as u32,
                viewport: None,
                elapsed_ns: 0,
                progress: OrdinaryQueryDiagnostics::default(),
                error: None,
            },
            state: None,
            result: Ok(Some(output)),
        }
    });
    let output = merge_render_query_outputs(&jobs).unwrap().unwrap();
    assert_eq!(output.indices().len(), 2 * capacity);
    assert_eq!(output.area_metrics().len(), output.indices().len());
    assert_eq!(&output.indices()[..capacity], &output.indices()[capacity..]);
    for job in 0..2 {
        let entries = job * capacity..(job + 1) * capacity;
        assert!(output.area_metrics()[entries]
            .iter()
            .all(|&area| area == Some(job as f32 + 0.25)));
    }
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn cached_direct_query_matches_uncached_installed_tome() {
    let bytes =
        std::fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = SpatialTree::parse(&bytes).unwrap();
    let data = DirectTomeQueryData::parse(&bytes, &tree).unwrap();
    let gates = initial_gate_states(&bytes).unwrap();
    assert!(data.decoded_record_count() > 0);
    for point in [
        [-43.0, 138.0, 48.0],
        [-36.249935, 107.33754, 22.55],
        [100_000.0, 0.0, 0.0],
    ] {
        for yaw in [0.0f32, 0.5, 90.0, 180.0, 270.0] {
            let (sin, cos) = yaw.to_radians().sin_cos();
            let pose = [
                [cos, 0.0, sin, point[0]],
                [0.0, 1.0, 0.0, point[1]],
                [-sin, 0.0, cos, point[2]],
                [0.0, 0.0, 0.0, 1.0],
            ];
            let rows =
                original_world_visibility_query_matrix(80.0, 1920, 1080, 1.0, pose, 0.01, 7000.0)
                    .unwrap();
            let input = OrdinaryDirectQueryInput {
                camera_point: point,
                render_rows: rows,
                cropped_rows: rows,
                initial_tile_rect: [0, 0, 8, 16],
                camera_imprecision: [0.6; 3],
                object_imprecision: 0.6,
                minimum_area_metric: 25.0 / (1920.0 * 1080.0),
                global_distance_threshold: ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                object_distance_scale: 1.0,
                winding_flip: query_matrix_winding_flip(rows),
                gate_states: Some(&gates),
                output_capacity: ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
            };
            let mut reference_progress = OrdinaryQueryDiagnostics::default();
            let reference = ordinary_direct_tome_visible_objects_with_diagnostics(
                &tree,
                &bytes,
                &input,
                &mut reference_progress,
            );
            let cached = ordinary_direct_tome_render_job(&tree, &bytes, &data, &input, 0, 1);
            assert_eq!(cached.diagnostics.error, reference.as_ref().err().cloned());
            if point[0] == 100_000.0 {
                assert_eq!(
                    cached.diagnostics.error.as_deref(),
                    Some("Tome query input outside root bounds (error 5)")
                );
            }
            match (reference, cached.result) {
                (Ok(Some(reference)), Ok(Some(cached))) => {
                    assert_eq!(cached.indices(), reference.indices());
                    assert_eq!(cached.area_metrics(), reference.area_metrics());
                    assert_eq!(cached.count(), reference.count());
                }
                (Ok(None), Ok(None)) => {}
                (Err(reference), Err(cached)) => assert_eq!(reference, cached),
                _ => panic!("cached Tome result differs at {point:?}, yaw {yaw}"),
            }
            assert_eq!(
                cached.diagnostics.progress.cells_processed,
                reference_progress.cells_processed
            );
        }
    }
}

#[test]
fn tests_quad_edge_segments_against_spatial_boxes() {
    let bounds = [[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]];
    assert!(segment_intersects_aabb(
        bounds,
        [[-2.0, 0.0, 0.0], [2.0, 0.0, 0.0]]
    ));
    assert!(segment_intersects_aabb(
        bounds,
        [[-2.0, 1.0, 0.0], [2.0, 1.0, 0.0]]
    ));
    assert!(!segment_intersects_aabb(
        bounds,
        [[-2.0, 2.0, 0.0], [2.0, 2.0, 0.0]]
    ));
    assert!(!segment_intersects_aabb(
        bounds,
        [[-2.0, 2.0, 2.0], [2.0, 2.0, 2.0]]
    ));
}

#[test]
fn tests_original_four_corner_volume_against_spatial_box() {
    let bounds = [[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]];
    let quad = |extent: f32, z: f32| {
        [
            [-extent, -extent, z],
            [-extent, extent, z],
            [extent, extent, z],
            [extent, -extent, z],
        ]
    };
    assert!(quad_intersects_aabb(bounds, quad(0.5, 0.0)));
    assert!(quad_intersects_aabb(bounds, quad(2.0, 0.0)));
    assert!(quad_intersects_aabb(bounds, quad(2.0, 1.0)));
    assert!(quad_intersects_aabb(
        bounds,
        [
            [-2.0, -0.5, 0.0],
            [-2.0, 0.5, 0.0],
            [2.0, 0.5, 0.0],
            [2.0, -0.5, 0.0],
        ]
    ));
    assert!(!quad_intersects_aabb(bounds, quad(2.0, 2.0)));
    assert!(!quad_intersects_aabb(
        bounds,
        [
            [2.0, -2.0, 0.0],
            [2.0, 2.0, 0.0],
            [3.0, 2.0, 0.0],
            [3.0, -2.0, 0.0],
        ]
    ));
}

#[test]
fn reads_link_gate_indices_from_header_table() {
    assert_eq!(direct_spatial_link_target([17, 0, 0, 0]), Some(17));
    assert_eq!(
        direct_spatial_link_target([(1 << 27) | 17, 0, 0, 0]),
        Some(17)
    );
    assert_eq!(direct_spatial_link_target([(1 << 28) | 17, 0, 0, 0]), None);
    assert_eq!(
        packed_link_geometry([(2 << 30) | (1 << 29) | 17, 0x1234, 0x2000_8000, 0x4000_a000]),
        Some(PackedLinkGeometry {
            destination: LinkDestination::Spatial(17),
            axis: 2,
            positive_side: true,
            plane: 0x1234,
            spans: [[0x2000, 0x8000], [0x4000, 0xa000]],
        })
    );
    assert_eq!(
        packed_link_geometry([(1 << 28) | (2 << 29) | 17, 0x1234, 0x2000_8000, 0x4000_a000])
            .unwrap()
            .destination,
        LinkDestination::BoundaryFace(2)
    );
    assert_eq!(packed_link_geometry([(1 << 27) | 17, 0, 0, 0]), None);
    let facing = packed_link_geometry([(1 << 29) | 17, 50, 0, 0]).unwrap();
    assert!(packed_link_faces_volume(
        facing,
        [52, 0, 0, 0],
        [2, 0, 0, 0]
    ));
    assert!(!packed_link_faces_volume(
        facing,
        [53, 0, 0, 0],
        [2, 0, 0, 0]
    ));
    assert!(packed_link_faces_point(facing, [true, false, false, false]));
    assert!(!packed_link_faces_point(
        facing,
        [false, false, false, false]
    ));
    let opposite = packed_link_geometry([17, 50, 0, 0]).unwrap();
    assert!(packed_link_faces_volume(
        opposite,
        [48, 0, 0, 0],
        [2, 0, 0, 0]
    ));
    assert!(!packed_link_faces_volume(
        opposite,
        [47, 0, 0, 0],
        [2, 0, 0, 0]
    ));
    let mut data = vec![0u8; 0x120];
    data[0x68..0x6c].copy_from_slice(&4u32.to_le_bytes());
    data[0x30..0x34].copy_from_slice(&1.0f32.to_le_bytes());
    data[0x70..0x74].copy_from_slice(&0xc0u32.to_le_bytes());
    data[0x74..0x78].copy_from_slice(&3u32.to_le_bytes());
    data[0x78..0x7c].copy_from_slice(&0x100u32.to_le_bytes());
    for (index, value) in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0].iter().enumerate() {
        data[0xcc + index * 4..0xd0 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(
        gated_link_bounds(&data, [(1 << 27) | 17, 0, 0, 1 << 12]).unwrap(),
        Some([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]])
    );
    assert_eq!(
        gated_link_query_bounds(&data, 0, [(1 << 27) | 17, 0, 0, 1 << 12]).unwrap(),
        Some([[0.0, 1.0, 2.0], [5.0, 6.0, 7.0]])
    );
    assert_eq!(gated_link_bounds(&data, [17, 0, 0, 1 << 12]).unwrap(), None);
    for (index, gate) in [0u32, 2, 3].iter().enumerate() {
        data[0x100 + index * 4..0x104 + index * 4].copy_from_slice(&gate.to_le_bytes());
    }
    assert_eq!(
        gate_indices_for_link(&data, [1 << 27, 0, (1 << 12) | 2, 0]).unwrap(),
        vec![2, 3]
    );
    let link = [1 << 27, 0, (1 << 12) | 2, 0];
    let mut states = initial_gate_states(&data).unwrap();
    assert_eq!(states, vec![u32::MAX]);
    assert!(gate_link_allows(&data, link, Some(&states)).unwrap());
    set_gate_state(&data, &mut states, 3, false).unwrap();
    assert!(!gate_link_allows(&data, link, Some(&states)).unwrap());
    set_gate_state(&data, &mut states, 3, true).unwrap();
    assert!(gate_link_allows(&data, link, Some(&states)).unwrap());
    assert!(gate_link_allows(&data, link, None).unwrap());
    assert!(!gate_link_allows(&data, link, Some(&[1 << 2])).unwrap());
    assert!(gate_link_allows(&data, link, Some(&[(1 << 2) | (1 << 3)])).unwrap());
    assert!(gate_indices_for_link(&data, [1 << 27, 0, (2 << 12) | 2, 0]).is_err());
}

#[test]
fn traversal_link_precheck_selects_original_direction_and_gate_branches() {
    let packed = [(1 << 29) | 17, 50, 0, 0];
    assert_eq!(
        traversal_link_precheck(
            &[],
            packed,
            LinkQueryDirection::Volume {
                center: [52, 0, 0, 0],
                tolerance: [2, 0, 0, 0],
            },
            None,
        )
        .unwrap(),
        TraversalLinkPrecheck::PackedDirection { faces_query: true }
    );
    assert_eq!(
        traversal_link_precheck(
            &[],
            packed,
            LinkQueryDirection::Point {
                axis_sides: [true, false, false, false],
            },
            None,
        )
        .unwrap(),
        TraversalLinkPrecheck::PackedDirection { faces_query: true }
    );

    let mut bytes = vec![0u8; 0x104];
    bytes[0x68..0x6c].copy_from_slice(&1u32.to_le_bytes());
    bytes[0x74..0x78].copy_from_slice(&1u32.to_le_bytes());
    bytes[0x78..0x7c].copy_from_slice(&0x100u32.to_le_bytes());
    bytes[0x100..0x104].copy_from_slice(&0u32.to_le_bytes());
    let gated = [1 << 27, 0, 1, 0];
    assert_eq!(
        traversal_link_precheck(
            &bytes,
            gated,
            LinkQueryDirection::Volume {
                center: [0; 4],
                tolerance: [0; 4],
            },
            Some(&[0]),
        )
        .unwrap(),
        TraversalLinkPrecheck::Gates { open: false }
    );
    assert!(traversal_link_precheck(
        &bytes,
        gated,
        LinkQueryDirection::Point {
            axis_sides: [false; 4],
        },
        Some(&[1]),
    )
    .unwrap()
    .allows_traversal());
}

#[test]
fn ready_same_spatial_cell_target_matches_original_link_loop_condition() {
    let volume = LinkQueryDirection::Volume {
        center: [53, 0, 0, 0],
        tolerance: [2, 0, 0, 0],
    };
    let positive = [(1 << 29) | 17, 50 | (71 << 16), 0, 0];
    assert_eq!(
        ready_same_spatial_cell_target(positive, 17, volume, 0x8005),
        Some(ReadyCellTarget {
            cell_index: 71,
            worklist_entry: 5,
        })
    );
    assert_eq!(
        ready_same_spatial_cell_target(positive, 17, volume, 0xc005),
        None
    );
    assert_eq!(
        ready_same_spatial_cell_target(positive, 17, volume, 0x4005),
        None
    );
    assert_eq!(
        ready_same_spatial_cell_target(positive, 18, volume, 0x8005),
        None
    );

    let facing = LinkQueryDirection::Point {
        axis_sides: [true, false, false, false],
    };
    assert_eq!(
        ready_same_spatial_cell_target(positive, 17, facing, 0x8005),
        None
    );
    assert_eq!(
        ready_same_spatial_cell_target(
            positive,
            17,
            LinkQueryDirection::Point {
                axis_sides: [false, false, false, false],
            },
            0x8005
        ),
        Some(ReadyCellTarget {
            cell_index: 71,
            worklist_entry: 5,
        })
    );

    let gated = [(1 << 27) | (1 << 29) | 17, 0 | (71 << 16), 0, 0];
    assert_eq!(
        ready_same_spatial_cell_target(gated, 17, facing, 0x8005),
        Some(ReadyCellTarget {
            cell_index: 71,
            worklist_entry: 5,
        })
    );
    assert_eq!(
        ready_same_spatial_cell_target([(1 << 28) | 17, 71 << 16, 0, 0], 17, facing, 0x8005),
        None
    );
}

#[test]
fn quantizes_direct_tome_link_query_with_record_padding() {
    let mut bytes = vec![0u8; 0x80];
    for (index, value) in [0.0f32, 0.0, 0.0, 65535.0, 65535.0, 65535.0]
        .iter()
        .enumerate()
    {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x30..0x34].copy_from_slice(&1.0f32.to_le_bytes());
    let (center, tolerance) =
        direct_link_query_quantization(&bytes, 0, [32768.0, 100.0, 0.0]).unwrap();
    assert_eq!(center, [32767, 99, 0, -1]);
    assert_eq!(tolerance, [2, 2, 2, 1]);
}

#[test]
fn expands_spatial_record_query_bounds_by_authored_padding_and_context_offset() {
    let mut bytes = vec![0u8; 0x90];
    let record = 0x10usize;
    for (axis, value) in [0.0f32, 1.0, 2.0, 10.0, 11.0, 12.0].iter().enumerate() {
        let at = record + axis * 4;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[record + 0x30..record + 0x34].copy_from_slice(&0.5f32.to_le_bytes());
    assert_eq!(
        spatial_record_query_bounds(&bytes, record as u32, [0.25, 0.5, 0.0]).unwrap(),
        [[-0.75, 0.0, 1.5, 1.0], [10.75, 12.0, 12.5, 1.0]]
    );
    assert!(spatial_record_query_bounds(&bytes, record as u32, [f32::NAN, 0.0, 0.0]).is_err());
}

#[test]
fn assembles_record_local_query_bounds_quantization_and_active_planes() {
    let mut bytes = vec![0u8; 0x90];
    let record = 0x10usize;
    for (axis, value) in [0.0f32, 0.0, 0.0, 10.0, 10.0, 10.0].iter().enumerate() {
        let at = record + axis * 4;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[record + 0x30..record + 0x34].copy_from_slice(&0.5f32.to_le_bytes());
    let planes = [
        [1.0, 0.0, 0.0, -11.0],
        [-1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, -9.0],
    ];
    let setup = setup_spatial_record_query(
        &bytes,
        record as u32,
        [5.0, 6.0, 7.0],
        [0.25, 0.5, 0.0],
        &planes,
    )
    .unwrap();
    assert_eq!(
        setup.bounds,
        [[-0.75, -1.0, -0.5, 1.0], [10.75, 11.0, 10.5, 1.0]]
    );
    assert_eq!(setup.link_center, [32767, 39320, 45874, -1]);
    assert_eq!(setup.active_planes, [0, 1, 2]);
    assert_eq!(
        setup.link_tolerance,
        direct_link_query_quantization(&bytes, record as u32, [5.0, 6.0, 7.0])
            .unwrap()
            .1
    );
}

#[test]
fn expands_packed_portal_bounds_from_original_record() {
    let mut bytes = vec![0u8; 0x80];
    for (index, value) in [0.0f32, 0.0, 0.0, 65535.0, 65535.0, 65535.0]
        .iter()
        .enumerate()
    {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x30..0x34].copy_from_slice(&1.0f32.to_le_bytes());
    let geometry =
        packed_link_geometry([(2 << 30) | 17, 0x1234, 0x2000_8000, 0x4000_a000]).unwrap();
    assert_eq!(
        packed_link_query_bounds(&bytes, 0, geometry).unwrap(),
        [[8191.0, 16383.0, 4659.0], [32769.0, 40961.0, 4661.0]]
    );
}

#[test]
fn clips_box_against_original_query_plane_with_rcpps() {
    let query = [[-10.0, -10.0, -10.0, 1.0], [10.0, 10.0, 10.0, 1.0]];
    let input = [[0.0, 0.0, 0.0, 1.0], [2.0, 2.0, 2.0, 1.0]];
    let mut planes = [[0.0; 4]; 3];
    planes[2] = [1.0, 0.0, 0.0, -1.0];
    assert_eq!(
        clip_box_to_active_query_planes(query, input, &planes, &[2]).unwrap(),
        [[1.000244140625, 0.0, 0.0, 1.0], [2.0, 2.0, 2.0, 1.0]]
    );
    planes[2] = [-1.0, 0.0, 0.0, 1.0];
    assert_eq!(
        clip_box_to_active_query_planes(query, input, &planes, &[2]).unwrap(),
        [[0.0, 0.0, 0.0, 1.0], [0.999755859375, 2.0, 2.0, 1.0]]
    );
    planes[2] = [1.0, 0.0, 0.0, -3.0];
    assert_eq!(
        clip_box_to_active_query_planes(query, input, &planes, &[2]).unwrap(),
        input
    );
}

#[test]
fn box_clipping_matches_original_executable_cases() {
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/box_clip_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values = line
            .split_whitespace()
            .map(|part| part.parse::<f32>().unwrap())
            .collect::<Vec<_>>();
        let count = values[0] as usize;
        assert_eq!(values.len(), 25 + count * 4, "case {case}");
        let box_at = |start: usize| -> [[f32; 4]; 2] {
            [
                values[start..start + 4].try_into().unwrap(),
                values[start + 4..start + 8].try_into().unwrap(),
            ]
        };
        let mut planes = vec![[0.0; 4]; 2];
        for offset in (17..17 + count * 4).step_by(4) {
            planes.push(values[offset..offset + 4].try_into().unwrap());
        }
        let actual = clip_box_to_active_query_planes(
            box_at(1),
            box_at(9),
            &planes,
            &(2..2 + count).collect::<Vec<_>>(),
        )
        .unwrap();
        let expected = box_at(17 + count * 4);
        for side in 0..2 {
            for axis in 0..4 {
                assert!(
                    (actual[side][axis] - expected[side][axis]).abs() <= 0.005,
                    "case {case}, bound {side}, axis {axis}: {} != {}",
                    actual[side][axis],
                    expected[side][axis]
                );
            }
        }
    }
}

#[test]
fn std_floor_matches_installed_renderer_math() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/floor_cases.txt"
    ))
    .lines()
    {
        let words: Vec<_> = line
            .split_whitespace()
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect();
        assert_eq!(words.len(), 2);
        let value = f32::from_bits(words[0]);
        assert!(!value.is_nan());
        assert_eq!(value.floor().to_bits(), words[1], "input {:08x}", words[0]);
        checked += 1;
    }
    assert_eq!(checked, 9981);
}

#[test]
fn scaled_render_viewport_matches_installed_executable_cases() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/viewport_scale_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 10);
        let rect = std::array::from_fn(|i| fields[i].parse().unwrap());
        let scale = std::array::from_fn(|i| fields[4 + i].parse().unwrap());
        let expected: [i32; 4] = std::array::from_fn(|i| fields[6 + i].parse().unwrap());
        assert_eq!(scaled_render_viewport(rect, scale).unwrap(), expected);
        checked += 1;
    }
    assert_eq!(checked, 37);
    assert_eq!(
        render_viewport_dimensions([0, 0, 1919, 1079]).unwrap(),
        [1920, 1080]
    );
    assert!(scaled_render_viewport([0, 0, 9, 9], [f32::NAN, 1.0]).is_err());
    assert!(scaled_render_viewport([0, 0, 9, 9], [f32::MAX, 1.0]).is_err());
    assert!(scaled_render_viewport([i32::MIN, 0, i32::MAX, 9], [1.0; 2]).is_err());
    assert!(render_viewport_dimensions([0, 0, -1, 9]).is_err());
}

#[test]
fn caster_containment_matches_installed_executable_cases() {
    let mut checked = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/shadow_containment_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values: Vec<u32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let count = values[0] as usize;
        assert_eq!(values.len(), 34 + count);
        let bounds = std::array::from_fn(|side| {
            std::array::from_fn(|axis| f32::from_bits(values[2 + side * 4 + axis]))
        });
        let planes: [[f32; 4]; 6] = std::array::from_fn(|plane| {
            std::array::from_fn(|axis| f32::from_bits(values[10 + plane * 4 + axis]))
        });
        let active: Vec<usize> = values[34..].iter().map(|&v| v as usize).collect();
        assert_eq!(
            query_planes_contain_aabb(&planes, bounds, &active).unwrap(),
            values[1] != 0,
            "containment case {case}"
        );
        checked += 1;
    }
    assert_eq!(checked, 2048);
    assert!(query_planes_contain_aabb(&[], [[0.0; 4]; 2], &[0]).is_err());
}

#[test]
fn native_plane_initializer_matches_original_bytes() {
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/plane_initialization_cases.txt"
    ));
    let mut cases = 0;
    for (case, line) in fixture.lines().enumerate() {
        let mut fields = line.split_whitespace();
        let count = fields.next().unwrap().parse::<usize>().unwrap();
        let mut planes = [[0.0; 4]; 22];
        for plane in &mut planes[..count] {
            for coefficient in plane {
                *coefficient =
                    f32::from_bits(u32::from_str_radix(fields.next().unwrap(), 16).unwrap());
            }
        }
        let expected = fields.next().unwrap();
        assert_eq!(expected.len(), 0x590 * 2);
        assert!(fields.next().is_none());
        let expected: Vec<u8> = (0..0x590)
            .map(|index| u8::from_str_radix(&expected[index * 2..index * 2 + 2], 16).unwrap())
            .collect();
        let mut context = std::array::from_fn(|index| ((index * 37 + count * 17) & 255) as u8);
        initialize_native_plane_query(&mut context, &planes[..count]).unwrap();
        assert_eq!(
            context.as_slice(),
            expected.as_slice(),
            "initializer case {case}"
        );
        cases += 1;
    }
    assert_eq!(cases, 9);
}

#[test]
fn native_plane_initializer_rejects_oversize_before_writing() {
    let mut context = [0xa5; 0x590];
    let original = context;
    assert!(initialize_native_plane_query(&mut context, &[[1.0; 4]; 23]).is_err());
    assert_eq!(context, original);
}

#[test]
fn native_shadow_caster_query_returns_ordered_candidate_ids() {
    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
        put_u32(bytes, offset, value.to_bits());
    }
    fn box_planes(bounds: [[f32; 3]; 2]) -> [[f32; 4]; 6] {
        [
            [1.0, 0.0, 0.0, -bounds[0][0]],
            [-1.0, 0.0, 0.0, bounds[1][0]],
            [0.0, 1.0, 0.0, -bounds[0][1]],
            [0.0, -1.0, 0.0, bounds[1][1]],
            [0.0, 0.0, 1.0, -bounds[0][2]],
            [0.0, 0.0, -1.0, bounds[1][2]],
        ]
    }

    let count = 5usize;
    let bounds_at = 0x60;
    let ids_at = bounds_at + count * 0x18;
    let mut bytes = vec![0; ids_at + count * 4];
    put_u32(&mut bytes, 0x40, count as u32);
    put_u32(&mut bytes, 0x44, bounds_at as u32);
    put_u32(&mut bytes, 0x50, ids_at as u32);
    for (index, center_x) in [0.4, 0.5, 0.6, 0.0, 3.0].into_iter().enumerate() {
        let at = bounds_at + index * 0x18;
        for (axis, center) in [center_x, 0.0, 0.0].into_iter().enumerate() {
            put_f32(&mut bytes, at + axis * 4, center - 0.1);
            put_f32(&mut bytes, at + 12 + axis * 4, center + 0.1);
        }
        put_u32(&mut bytes, ids_at + index * 4, 100 + index as u32);
    }
    let tome = shadow_caster::ShadowCasterTome::parse(&bytes).unwrap();
    let narrow = [[-0.25; 3], [0.25; 3]];
    let broad = [[-1.0; 3], [1.0; 3]];
    let narrow_planes = box_planes(narrow);
    let broad_planes = box_planes(broad);
    let active = [0, 1, 2, 3, 4, 5];
    let receivers = [
        shadow_caster::VerifiedShadowReceiverState {
            bounds: narrow,
            planes: &narrow_planes,
            active_planes: &active,
        },
        shadow_caster::VerifiedShadowReceiverState {
            bounds: broad,
            planes: &broad_planes,
            active_planes: &active,
        },
    ];
    let flags = [true; 5];
    let parent_suppression = [
        NativeShadowCasterSuppression {
            parent_id: 101,
            suppressed_child_id: 100,
        },
        NativeShadowCasterSuppression {
            parent_id: 100,
            suppressed_child_id: 102,
        },
    ];
    let mut scratch = NativeShadowCasterAabbQueryScratch::new(&tome, receivers.len()).unwrap();
    let input = NativeShadowCasterAabbQueryInput {
        receivers: &receivers,
        caster_enabled: &flags,
        parent_suppression: &parent_suppression,
        distance_path: NativeShadowDistancePath::UnboundedNoAuxiliaryRecords,
    };
    assert_eq!(
        scratch.query_candidates(&tome, input).unwrap(),
        &[vec![103], vec![101, 102]]
    );
    let capacities: Vec<_> = scratch
        .output_by_receiver
        .iter()
        .map(Vec::capacity)
        .collect();
    let id_capacity = scratch.active_ids.capacity();
    assert_eq!(
        scratch.query_candidates(&tome, input).unwrap(),
        &[vec![103], vec![101, 102]]
    );
    assert_eq!(
        scratch
            .output_by_receiver
            .iter()
            .map(Vec::capacity)
            .collect::<Vec<_>>(),
        capacities
    );
    assert_eq!(scratch.active_ids.capacity(), id_capacity);
    assert_eq!(
        scratch
            .query_directional_candidates(&tome, &bytes, [1.0, 0.0, 0.0], input)
            .unwrap(),
        &[vec![103], vec![101, 102]]
    );

    // Dense table: direction [1,0,0] selects face 0, bin (8,8).
    // Only objects 0,1,3 may cast; suppression still removes object 0.
    bytes.resize(0x600, 0);
    put_u32(&mut bytes, 0x10, 2);
    put_u32(&mut bytes, 0x140, 0x180);
    for index in [0, 1, 3] {
        let bit = (8 * 16 + 8) * count + index;
        let at = 0x180 + (bit / 32) * 4;
        let word = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        put_u32(&mut bytes, at, word | (1 << (bit % 32)));
    }
    assert_eq!(
        scratch
            .query_directional_candidates(&tome, &bytes, [1.0, 0.0, 0.0], input)
            .unwrap(),
        &[vec![103], vec![101]]
    );

    // Palette table: nibble zero selects each object's palette entry.
    // Only objects 0 and 3 have the native accepting sentinel 0x7f7f.
    bytes[0x180..].fill(0);
    put_u32(&mut bytes, 0x10, 1);
    put_u32(&mut bytes, 0x13c, 0x180);
    put_u32(&mut bytes, 0x140, 0x400);
    put_u32(&mut bytes, 0x144, 0x500);
    put_u32(&mut bytes, 0x148, 1);
    for index in 0..count {
        put_u32(
            &mut bytes,
            0x180 + index * 0x40 + 4,
            if index == 0 || index == 3 { 0 } else { 1 },
        );
    }
    bytes[0x500..0x502].copy_from_slice(&0x7f7fu16.to_le_bytes());
    assert_eq!(
        scratch
            .query_directional_candidates(&tome, &bytes, [1.0, 0.0, 0.0], input)
            .unwrap(),
        &[vec![103], vec![100]]
    );
    assert_eq!(scratch.active_ids.capacity(), id_capacity);
    assert_eq!(
        scratch
            .output_by_receiver
            .iter()
            .map(Vec::capacity)
            .collect::<Vec<_>>(),
        capacities
    );
    assert!(scratch
        .query_directional_candidates(&tome, &bytes, [f32::NAN, 0.0, 0.0], input)
        .is_err());
    assert_eq!(scratch.candidate_ids(0), Some(&[][..]));
    assert_eq!(scratch.candidate_ids(1), Some(&[][..]));

    let unsupported = NativeShadowCasterAabbQueryInput {
        distance_path: NativeShadowDistancePath::Unknown,
        ..input
    };
    assert!(scratch.query_candidates(&tome, unsupported).is_err());
    assert_eq!(scratch.candidate_ids(0), Some(&[][..]));
    assert_eq!(scratch.candidate_ids(1), Some(&[][..]));
}

#[test]
fn spatial_query_and_bounds_match_installed_executable_cases() {
    let mut checked = [0; 2];
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/spatial_query_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let mut fields = line.split_whitespace();
        let kind = fields.next().unwrap();
        let values: Vec<u64> = fields.map(|v| v.parse().unwrap()).collect();
        let float = |i: usize| f32::from_bits(values[i] as u32);
        match kind {
            "spatial" => {
                assert_eq!(values.len(), 29);
                let rows = std::array::from_fn(|row| {
                    std::array::from_fn(|column| float(7 + row * 4 + column))
                });
                let viewport = render_query_job_tile(values[1] as u32, values[0] as u32).unwrap();
                let state = RenderQueryState {
                    render_rows: rows,
                    cropped_rows: crop_query_rows(rows, viewport).unwrap(),
                    camera_point: [0.0; 3],
                    camera_imprecision: std::array::from_fn(|i| float(4 + i)),
                    winding_flip: false,
                    viewport,
                    maximum_depth: 0.0,
                    depth_grid: None,
                };
                let bounds = std::array::from_fn(|side| {
                    std::array::from_fn(|axis| float(23 + side * 3 + axis))
                });
                assert_eq!(
                    render_query_aabb_visible_without_depth(
                        (values[2] != 0).then_some(&state),
                        bounds,
                    )
                    .unwrap(),
                    values[3] != 0,
                    "spatial case {case}"
                );
                checked[0] += 1;
            }
            "bounds" => {
                let count = values[0] as usize;
                assert_eq!(values.len(), 42 + count);
                let box_at = |first| {
                    std::array::from_fn(|side| {
                        std::array::from_fn(|axis| float(first + side * 4 + axis))
                    })
                };
                let planes: [[f32; 4]; 6] = std::array::from_fn(|plane| {
                    std::array::from_fn(|axis| float(18 + plane * 4 + axis))
                });
                let active: Vec<usize> = values[42..].iter().map(|&v| v as usize).collect();
                assert_eq!(
                    query_bounds_accepts_aabb(box_at(2), box_at(10), &planes, &active).unwrap(),
                    values[1] != 0,
                    "bounds case {case}"
                );
                checked[1] += 1;
            }
            _ => panic!("unknown original spatial fixture kind"),
        }
    }
    assert_eq!(checked, [576, 70]);
}

#[test]
fn spatial_query_uses_native_depth_and_rejects_bad_plane_indices() {
    let rows = std::array::from_fn(|row| {
        std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
    });
    let mut state = RenderQueryState {
        render_rows: rows,
        cropped_rows: rows,
        camera_point: [0.0; 3],
        camera_imprecision: [0.0; 3],
        winding_flip: false,
        viewport: [0, 0, 64, 64],
        maximum_depth: 0.0,
        depth_grid: None,
    };
    let bounds = [[0.0; 3], [1.0; 3]];
    assert!(render_query_aabb_visible_without_depth(Some(&state), bounds).unwrap());
    state.camera_imprecision = [0.6; 3];
    state.depth_grid = Some(Box::new([0.0; 4096]));
    // Native nearest-Z <= 0 accepts, even with an empty depth grid.
    assert!(render_query_aabb_visible_without_depth(Some(&state), bounds).unwrap());
    state.camera_imprecision = [0.0; 3];
    let in_front = [[-0.125, -0.125, 0.25], [0.125, 0.125, 0.75]];
    assert!(!render_query_aabb_visible_without_depth(Some(&state), in_front).unwrap());
    state.depth_grid = Some(Box::new([1.0; 4096]));
    assert!(render_query_aabb_visible_without_depth(Some(&state), in_front).unwrap());
    assert!(query_bounds_accepts_aabb([[0.0; 4]; 2], [[0.0; 4]; 2], &[], &[0]).is_err());
    assert!(render_query_aabb_visible_without_depth(None, [[f32::NAN; 3]; 2]).unwrap());
}

#[test]
fn projected_area_metric_uses_original_unit_square_clip() {
    assert_eq!(projected_object_area_metric([-1.0, -1.0, 1.0, 1.0]), 1.0);
    assert_eq!(projected_object_area_metric([0.0, 0.0, 1.0, 1.0]), 0.25);
    assert_eq!(projected_object_area_metric([-2.0, -2.0, 0.0, 0.0]), 0.25);
    assert_eq!(projected_object_area_metric([2.0, 2.0, 3.0, 3.0]), 0.0);
}

#[test]
fn projected_area_cutoff_is_25_viewport_pixels() {
    let cutoff = minimum_projected_area_metric(1920, 1080).unwrap();
    assert!((cutoff * 1920.0 * 1080.0 - 25.0).abs() < 0.0001);
    assert!(minimum_projected_area_metric(0, 1080).is_err());
    assert!(minimum_projected_area_metric(1920, i32::MAX).is_err());
}

#[test]
fn projected_visible_output_applies_cutoff_before_portal_and_deduplicates() {
    let mut portal = PortalTileMask::new([0, 0, 8, 16]).unwrap();
    portal.bits.fill(u32::MAX);
    let cutoff = minimum_projected_area_metric(64, 64).unwrap();
    let mut output = VisibleObjectIndexOutput::new(4, 4);

    let visible_metric = output
        .push_if_projected_object_visible(
            2,
            [0.0, 0.0, 0.2, 0.2],
            cutoff,
            [0, 0, 8, 16],
            &portal,
            None,
        )
        .unwrap()
        .unwrap();
    assert!((visible_metric - 0.01).abs() < 0.000_001);
    assert!(output.contains(2).unwrap());
    assert!(!output.contains(1).unwrap());
    assert_eq!(
        output
            .push_if_projected_object_visible(
                1,
                [0.0, 0.0, 0.05, 0.05],
                cutoff,
                [0, 0, 8, 16],
                &portal,
                None,
            )
            .unwrap(),
        None
    );
    assert_eq!(
        output
            .push_if_projected_object_visible(
                2,
                [f32::NAN; 4],
                cutoff,
                [0, 0, 8, 16],
                &portal,
                None,
            )
            .unwrap(),
        None
    );
    assert_eq!(output.indices(), &[2]);
    assert_eq!(output.area_metrics().len(), output.indices().len());
    assert!((output.area_metrics()[0].unwrap() - 0.01).abs() < 0.000_001);
    assert_eq!(output.count(), 1);
}

#[test]
fn cell_candidate_pipeline_skips_accepted_and_distance_rejected_bounds() {
    let mut bytes = vec![0u8; 0xa0];
    bytes[0x40..0x44].copy_from_slice(&1u32.to_le_bytes());
    bytes[0x44..0x48].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[0x54..0x58].copy_from_slice(&(1u32 | (1 << 5)).to_le_bytes());
    bytes[0x58..0x5c].copy_from_slice(&0x60u32.to_le_bytes());
    bytes[0x5c..0x60].copy_from_slice(&1u32.to_le_bytes());
    bytes[0x60] = 0b10; // candidate index 0, run length 1
    for (index, value) in [-0.5f32, -0.5, 0.0, 0.5, 0.5, 0.0].into_iter().enumerate() {
        bytes[0x80 + index * 4..0x84 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    let cells = CellTree {
        bounds: Vec::new(),
        children: Vec::new(),
        axes: Vec::new(),
        planes: Vec::new(),
        leaf_ranks: Vec::new(),
        cell_codes: Vec::new(),
        cell_ids: vec![1],
        cell_visibility_counts: vec![0],
        direct_cell_ids_available: true,
        cell_bounds: vec![[[-1.0; 3], [1.0; 3]]],
        cell_bounds_root: [[-1.0; 3], [1.0; 3]],
        cell_table_at: 0,
        cell_link_ranges: vec![(0, 0)],
        cell_candidate_ranges: vec![(0, 1)],
        links_at: 0,
        candidates_at: 0x60,
        candidate_index_bits: 1,
        candidate_run_bits: 1,
        candidate_record_count: 1,
        object_count: 1,
        code_width: 1,
        portal_nodes_at: None,
        portal_planes_at: None,
    };
    let mut portal = PortalTileMask::new([0, 0, 8, 16]).unwrap();
    portal.bits.fill(u32::MAX);
    let query = CellObjectVisibilityQuery {
        record_query_bounds: [[-1.0, -1.0, -1.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
        query_planes: &[],
        active_planes: &[],
        projection_rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        camera_translation_imprecision: 0.0,
        object_distance_point: [0.0; 3],
        object_distance_scale_squared: [1.0; 4],
        minimum_area_metric: 0.0,
        viewport_tile_bounds: [0, 0, 8, 16],
        portal: &portal,
        depth_grid: None,
        point_occlusion_near_offset: None,
    };
    let mut output = VisibleObjectIndexOutput::new(1, 1);
    cells
        .append_visible_object_candidates(&bytes, 0, &query, &mut output)
        .unwrap();
    assert_eq!(output.indices(), &[0]);
    //Native RCPPS(1) is0x3f7ff000, so this half-unit box's
    //projected area retains the executable's reciprocal error.
    assert_eq!(output.area_metrics()[0].unwrap().to_bits(), 0x3e7fe001);

    bytes[0x80..0x84].copy_from_slice(&f32::NAN.to_le_bytes());
    cells
        .append_visible_object_candidates(&bytes, 0, &query, &mut output)
        .unwrap();
    assert_eq!(output.count(), 1);

    // Distance rejection precedes the now-invalid geometry bounds, and
    // marks a tested bit without appending a visible object.
    bytes.resize(0xc0, 0);
    bytes[0x48..0x4c].copy_from_slice(&0xa0u32.to_le_bytes());
    let mut rejected = VisibleObjectIndexOutput::new(1, 1);
    cells
        .append_visible_object_candidates(&bytes, 0, &query, &mut rejected)
        .unwrap();
    assert!(rejected.indices().is_empty());
    assert!(rejected.area_metrics().is_empty());
    assert_eq!(rejected.count(), 0);
    assert!(rejected.contains(0).unwrap());
    assert_eq!(rejected.distance_rejected_objects, 1);

    // A later cell must skip even the distance-table read for this index.
    bytes[0x48..0x4c].copy_from_slice(&u32::MAX.to_le_bytes());
    cells
        .append_visible_object_candidates(&bytes, 0, &query, &mut rejected)
        .unwrap();
    assert_eq!(rejected.distance_rejected_objects, 1);
}

#[test]
fn object_distance_matches_installed_instruction_cases() {
    let mut count = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/object_distance_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let words: Vec<u32> = line
            .split_whitespace()
            .map(|word| word.parse().unwrap())
            .collect();
        assert_eq!(words.len(), 17, "distance fixture {case}");
        let point = std::array::from_fn(|axis| f32::from_bits(words[axis]));
        let record = std::array::from_fn(|row| {
            std::array::from_fn(|axis| f32::from_bits(words[4 + row * 4 + axis]))
        });
        let scale = std::array::from_fn(|axis| f32::from_bits(words[12 + axis]));
        assert_eq!(
            object_distance_rejection_mask(record, point, scale),
            words[16],
            "native distance case {case}"
        );
        count += 1;
    }
    assert_eq!(count, 822);
}

#[test]
fn optional_object_distance_table_is_indexed_and_preserves_nan() {
    let mut bytes = vec![0u8; 0xa0];
    bytes[0x40..0x44].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(object_distance_bounds(&bytes, 0).unwrap(), None);
    bytes[0x48..0x4c].copy_from_slice(&0x60u32.to_le_bytes());
    let records = [
        [-1.0f32, -2.0, -3.0, 4.0, 1.0, 2.0, 3.0, 9.0],
        [-5.0, -6.0, -7.0, 25.0, 5.0, 6.0, 7.0, f32::NAN],
    ];
    for (index, record) in records.into_iter().enumerate() {
        for (field, value) in record.into_iter().enumerate() {
            let at = 0x60 + index * 32 + field * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    assert_eq!(
        object_distance_bounds(&bytes, 0).unwrap(),
        Some([[-1.0, -2.0, -3.0, 4.0], [1.0, 2.0, 3.0, 9.0]])
    );
    let second = object_distance_bounds(&bytes, 1).unwrap().unwrap();
    assert_eq!(second[0], [-5.0, -6.0, -7.0, 25.0]);
    assert!(second[1][3].is_nan());
    assert!(object_distance_bounds(&bytes, 2)
        .unwrap_err()
        .contains("out of range"));
    bytes.truncate(0x9f);
    assert!(object_distance_bounds(&bytes, 1)
        .unwrap_err()
        .contains("truncated"));
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn authored_eotw_distance_tiers_match_installed_predicate() {
    let bytes =
        std::fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    for (point, expected) in [
        ([-43.0, 138.0, 48.0], [0, 15, 15]),
        ([-43.0, 138.0, 100.0], [15, 15, 0]),
    ] {
        let masks: Vec<u32> = (28684..=28686)
            .map(|index| {
                object_distance_rejection_mask(
                    object_distance_bounds(&bytes, index).unwrap().unwrap(),
                    point,
                    [1.0; 4],
                )
            })
            .collect();
        assert_eq!(masks, expected);
    }
    let record = object_distance_bounds(&bytes, 28684).unwrap().unwrap();
    assert_eq!(record[0][3], 0.0);
    assert_eq!(record[1][3], 225.0);
    let boundary = [-43.0, 138.0, record[1][2] + 15.0];
    let masks: Vec<u32> = (28684..=28686)
        .map(|index| {
            object_distance_rejection_mask(
                object_distance_bounds(&bytes, index).unwrap().unwrap(),
                boundary,
                [1.0; 4],
            )
        })
        .collect();
    assert_eq!(masks, [15, 0, 15]);
}

#[test]
fn viewport_bounds_pack_to_original_query_tile_bytes() {
    assert_eq!(query_viewport_tile_bounds([10, 5, 101, 51]), [1, 1, 13, 13]);
    assert_eq!(
        query_viewport_tile_bounds([0, 0, 1920, 1080]),
        [0, 0, 240, 14]
    );
}

#[test]
fn projected_object_rectangle_maps_to_pixels_and_intersects_query_tiles() {
    assert_eq!(
        projected_object_pixel_rect([-0.5, -0.5, 0.5, 0.5], [2, 3, 7, 11]).unwrap(),
        [16, 16, 48, 44]
    );
    assert_eq!(
        projected_object_pixel_rect([-2.0, -2.0, -1.5, -1.5], [0, 0, 8, 16]).unwrap(),
        [0, 0, 0, 0]
    );
    assert!(projected_object_pixel_rect([0.0, 0.0, 1.0, 1.0], [4, 8, 3, 9]).is_err());
}

#[test]
fn object_pixel_rectangles_match_installed_conversion_instructions() {
    let mut count = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/object_pixel_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 12);
        let projected =
            std::array::from_fn(|axis| f32::from_bits(fields[axis].parse::<u32>().unwrap()));
        let tiles = std::array::from_fn(|axis| fields[4 + axis].parse::<u8>().unwrap());
        let expected = std::array::from_fn(|axis| fields[8 + axis].parse::<i32>().unwrap());
        assert_eq!(
            projected_object_pixel_rect(projected, tiles).unwrap(),
            expected,
            "installed object pixel case {case}"
        );
        count += 1;
    }
    assert_eq!(count, 3000);
}

#[test]
fn thin_visible_objects_keep_pixel_coverage_under_small_screen_shifts() {
    let portal = PortalTileMask::full([0, 0, 8, 16]).unwrap();
    let cutoff = minimum_projected_area_metric(1280, 720).unwrap();
    for step in 0..120 {
        let x = -0.4 + step as f32 * 0.006;
        let projected = [x, -0.25, x + 0.0005, 0.25];
        let mut output = VisibleObjectIndexOutput::new(1, 1);
        assert!(
            output
                .push_if_projected_object_visible(
                    0,
                    projected,
                    cutoff,
                    [0, 0, 8, 16],
                    &portal,
                    None
                )
                .unwrap()
                .is_some(),
            "thin object lost pixel coverage at step {step}"
        );
    }
}

#[test]
fn projected_portal_rectangle_uses_authored_viewport_pixel_conversion() {
    assert_eq!(
        projected_portal_pixel_rect([-1.0, -1.0, 1.0, 1.0], [0, 0, 64, 64]).unwrap(),
        Some([0, 0, 64, 64])
    );
    assert_eq!(
        projected_portal_pixel_rect([-0.5, -0.5, 0.99, 0.99], [0, 0, 64, 64]).unwrap(),
        Some([16, 16, 64, 64])
    );
    assert_eq!(
        projected_portal_pixel_rect([-2.0, -2.0, -1.5, -1.5], [0, 0, 64, 64]).unwrap(),
        None
    );
    assert_eq!(
        projected_portal_pixel_rect([-2.0, -2.0, 2.0, 2.0], [8, 4, 40, 28]).unwrap(),
        Some([8, 4, 40, 28])
    );
    assert!(projected_portal_pixel_rect([f32::NAN, 0.0, 1.0, 1.0], [0, 0, 64, 64]).is_err());
    assert!(projected_portal_pixel_rect([0.0, 0.0, 1.0, 1.0], [4, 8, 3, 9]).is_err());
}

#[test]
fn projected_portal_box_fallback_reduces_both_homogeneous_faces() {
    let corners = [
        [-1.0, -0.5, 0.0, 2.0],
        [1.0, -0.5, 0.0, 2.0],
        [1.0, 0.5, 0.0, 2.0],
        [-1.0, 0.5, 0.0, 2.0],
        [-3.0, -2.0, 0.0, 4.0],
        [3.0, -2.0, 0.0, 4.0],
        [3.0, 2.0, 0.0, 4.0],
        [-3.0, 2.0, 0.0, 4.0],
    ];
    let bounds = projected_portal_box_fallback_bounds(corners).unwrap();
    for (actual, expected) in bounds.into_iter().zip([-0.75, -0.5, 0.75, 0.5]) {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }

    let zero_w = [
        [0.25, -0.25, 0.0, 0.0],
        [0.5, -0.5, 0.0, 1.0],
        [0.75, 0.5, 0.0, 1.0],
        [-0.5, 0.25, 0.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let bounds = projected_portal_box_fallback_bounds(zero_w).unwrap();
    for (actual, expected) in bounds.into_iter().zip([-0.5, -0.5, 0.75, 0.5]) {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }
    let invalid = [[f32::NAN, 0.0, 0.0, 1.0]; 8];
    assert!(projected_portal_box_fallback_bounds(invalid).is_err());
}

#[test]
fn projected_portal_box_uses_active_plane_crossing_clip_path() {
    let corners = [
        [-2.0, -0.5, 0.0, -1.0],
        [2.0, -0.5, 0.0, -1.0],
        [2.0, 0.5, 0.0, -1.0],
        [-2.0, 0.5, 0.0, -1.0],
        [-2.0, -0.5, 0.0, 1.0],
        [2.0, -0.5, 0.0, 1.0],
        [2.0, 0.5, 0.0, 1.0],
        [-2.0, 0.5, 0.0, 1.0],
    ];
    let (bounds, used_crossing_path) = projected_portal_box_bounds(corners, &[0]).unwrap().unwrap();
    assert!(used_crossing_path);
    for (actual, expected) in bounds.into_iter().zip([-1.0, -1.0, 1.0, 1.0]) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
    assert_eq!(
        projected_portal_box_pixel_rect(corners, &[0], [0, 0, 64, 64]).unwrap(),
        (Some([0, 0, 64, 64]), true)
    );

    let partially_visible = [
        [0.0, 0.0, 0.0, -1.0],
        [2.0, 0.0, 0.0, -1.0],
        [2.0, 0.5, 0.0, -1.0],
        [0.0, 0.5, 0.0, -1.0],
        [0.0, 0.0, 0.0, 1.0],
        [2.0, 0.0, 0.0, 1.0],
        [2.0, 0.5, 0.0, 1.0],
        [0.0, 0.5, 0.0, 1.0],
    ];
    let bounds = projected_portal_box_crossing_bounds(partially_visible)
        .unwrap()
        .unwrap();
    for (actual, expected) in bounds.into_iter().zip([-1.0, -1.0, 1.0, 1.0]) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    let outside = partially_visible.map(|mut corner| {
        corner[0] = 2.5;
        corner
    });
    assert_eq!(
        projected_portal_box_crossing_bounds(outside).unwrap(),
        Some([1.0, -1.0, 1.0, 1.0])
    );
    assert_eq!(
        projected_portal_box_pixel_rect(outside, &[0], [0, 0, 64, 64]).unwrap(),
        (None, true)
    );

    let (_, used_crossing_path) = projected_portal_box_bounds(corners, &[1, 0])
        .unwrap()
        .unwrap();
    assert!(!used_crossing_path);

    let in_front = corners.map(|mut corner| {
        corner[3] = 2.0;
        corner
    });
    let (_, used_crossing_path) = projected_portal_box_bounds(in_front, &[0])
        .unwrap()
        .unwrap();
    assert!(!used_crossing_path);
}

#[test]
fn portal_crossing_bounds_match_original_conservative_masks() {
    let mut count = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/portal_crossing_bounds_cases.txt"
    ))
    .lines()
    {
        let words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let corners = std::array::from_fn(|i| {
            let pair = i % 4;
            let offset = if i >= 4 { 12 } else { 0 };
            [
                f32::from_bits(words[offset + pair]),
                f32::from_bits(words[offset + 4 + pair]),
                0.,
                f32::from_bits(words[offset + 8 + pair]),
            ]
        });
        let actual = projected_portal_box_crossing_bounds(corners)
            .unwrap()
            .unwrap()
            .map(f32::to_bits);
        assert_eq!(actual, words[24..], "native portal crossing case{count}");
        count += 1;
    }
    assert_eq!(count, 512);
    assert!(projected_portal_box_crossing_bounds([[f32::NAN; 4]; 8]).is_err());
}

#[test]
fn object_bounds_expand_by_original_camera_translation_imprecision() {
    let bounds = [[-2.0, 3.0, 8.0], [5.0, 9.0, 14.0]];
    let expanded = expand_object_bounds_for_camera_imprecision(bounds, 0.6);
    for axis in 0..3 {
        assert!((expanded[0][axis] - (bounds[0][axis] - 0.6)).abs() < 1e-6);
        assert!((expanded[1][axis] - (bounds[1][axis] + 0.6)).abs() < 1e-6);
    }
    assert_eq!(
        expand_object_bounds_for_camera_imprecision(bounds, 0.0),
        bounds
    );
}

#[test]
fn visible_object_output_deduplicates_and_counts_beyond_buffer_capacity() {
    let mut output = VisibleObjectIndexOutput::new(70, 2);
    assert!(output.push_unique(31).unwrap());
    assert!(!output.push_unique(31).unwrap());
    assert!(output.push_unique(32).unwrap());
    assert!(output.push_unique(69).unwrap());
    assert!(!output.push_unique(69).unwrap());
    assert_eq!(output.indices(), [31, 32]);
    assert_eq!(output.count(), 3);
    assert!(output.push_unique(70).is_err());
}

#[test]
fn projected_rectangle_output_selects_original_portal_or_depth_branch() {
    let pixels = [0, 0, 8, 4];
    let mut portal = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    portal.bits[0] = 1;
    let mut output = VisibleObjectIndexOutput::new(4, 4);

    assert!(output
        .push_if_rectangle_visible(1, &portal, pixels, None)
        .unwrap());
    assert!(!output
        .push_if_rectangle_visible(1, &portal, pixels, None)
        .unwrap());

    let mut depths = vec![0.0; 64 * 64];
    depths[0] = 0.75;
    assert!(!output
        .push_if_rectangle_visible(2, &portal, pixels, Some((&depths, 0.8)))
        .unwrap());
    assert!(output
        .push_if_rectangle_visible(2, &portal, pixels, Some((&depths, 0.75)))
        .unwrap());
    assert_eq!(output.indices(), [1, 2]);
    assert_eq!(output.count(), 2);
}

fn visible_cell_test_tree(first: u32, count: u32) -> CellTree {
    CellTree {
        bounds: Vec::new(),
        children: Vec::new(),
        axes: Vec::new(),
        planes: Vec::new(),
        leaf_ranks: Vec::new(),
        cell_codes: Vec::new(),
        cell_ids: vec![first],
        cell_visibility_counts: vec![count],
        direct_cell_ids_available: true,
        cell_bounds: Vec::new(),
        cell_bounds_root: [[0.0; 3]; 2],
        cell_table_at: 0,
        cell_link_ranges: Vec::new(),
        cell_candidate_ranges: Vec::new(),
        links_at: 0,
        candidates_at: 0,
        candidate_index_bits: 0,
        candidate_run_bits: 0,
        candidate_record_count: 0,
        object_count: 0,
        code_width: 0,
        portal_nodes_at: None,
        portal_planes_at: None,
    }
}

#[test]
fn sphere_portal_geometry_matches_installed_instructions() {
    let mut checked = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/light_portal_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values: Vec<u32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let metrics = values[0];
        let link = values[7..11].try_into().unwrap();
        let sphere = std::array::from_fn(|i| f32::from_bits(values[11 + i]));
        let count = values[15] as usize;
        assert_eq!(values.len(), 16 + count + 3);
        let mut bytes = vec![0u8; 0x100 + count * 4];
        for (axis, &bits) in values[1..7].iter().enumerate() {
            bytes[0x14 + axis * 4..0x18 + axis * 4].copy_from_slice(&bits.to_le_bytes());
        }
        bytes[0x70..0x74].copy_from_slice(&0x100u32.to_le_bytes());
        bytes[0x74..0x78].copy_from_slice(&((count / 3) as u32).to_le_bytes());
        for (index, &bits) in values[16..16 + count].iter().enumerate() {
            bytes[0x100 + index * 4..0x104 + index * 4].copy_from_slice(&bits.to_le_bytes());
        }
        let mut inside = 1.25f32;
        let mut outside = 2.5f32;
        let accepted = sphere_intersects_portal(
            &bytes,
            link,
            sphere,
            (metrics & 1 != 0).then_some(&mut inside),
            (metrics & 2 != 0).then_some(&mut outside),
        )
        .unwrap();
        assert_eq!(
            [u32::from(accepted), inside.to_bits(), outside.to_bits()],
            <[u32; 3]>::try_from(&values[16 + count..]).unwrap(),
            "sphere portal case {case}: link={link:?}, sphere={sphere:?}, metrics={metrics}"
        );
        checked += 1;
    }
    assert_eq!(checked, 16704);
}

#[test]
fn triangle_distance_matches_installed_instructions() {
    let mut checked = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/light_triangle_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values: Vec<u32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(values.len(), 13);
        let point = std::array::from_fn(|axis| f32::from_bits(values[axis]));
        let triangle = std::array::from_fn(|vertex| {
            std::array::from_fn(|axis| f32::from_bits(values[3 + vertex * 3 + axis]))
        });
        assert_eq!(
            point_triangle_distance_squared(point, triangle).to_bits(),
            values[12],
            "triangle case {case}: point={point:?}, triangle={triangle:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 4728);
}

#[test]
fn authored_id_search_matches_installed_instructions() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/authored_id_search_cases.txt"
    ))
    .lines()
    {
        let mut fields = line.split_whitespace();
        let target: u32 = fields.next().unwrap().parse().unwrap();
        let present = fields.next().unwrap() == "1";
        let count: usize = fields.next().unwrap().parse().unwrap();
        let ids: Vec<u32> = fields
            .by_ref()
            .take(count)
            .map(|v| v.parse().unwrap())
            .collect();
        let expected: i32 = fields.next().unwrap().parse().unwrap();
        assert!(fields.next().is_none());
        let input = if present { ids.as_slice() } else { &[] };
        assert_eq!(
            sorted_authored_id_index(input, target)
                .map(|v| v as i32)
                .unwrap_or(-1),
            expected,
            "target {target}, present={present}, ids={ids:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 334);
}

#[test]
fn visible_cell_output_matches_installed_instructions() {
    let mut checked = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/visible_cell_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let values: Vec<u32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let [index_bits, run_bits, domain, capacity, first, count, remap, base, requested, seed_count]: [u32; 10] = values[..10].try_into().unwrap();
        let mut at = 10;
        let seeds = &values[at..at + seed_count as usize];
        at += seed_count as usize;
        let rows = values[at] as usize;
        at += 1;
        let pairs = &values[at..at + rows * 2];
        at += rows * 2;
        let expected_count = values[at] as usize;
        let expected_length = values[at + 1] as usize;
        at += 2;
        let expected_indices = &values[at..at + expected_length];
        at += expected_length;
        let mask_length = values[at] as usize;
        at += 1;
        let expected_mask = &values[at..at + mask_length];
        assert_eq!(at + mask_length, values.len(), "case {case}");
        let width = (index_bits + run_bits) as usize;
        let mut bytes = vec![0u8; 0x100 + (rows * width).div_ceil(32) * 4 + 4];
        bytes[0x54..0x58].copy_from_slice(&(index_bits << 10 | run_bits << 15).to_le_bytes());
        bytes[0x60..0x64].copy_from_slice(&0x100u32.to_le_bytes());
        for (row, pair) in pairs.chunks_exact(2).enumerate() {
            let packed = pair[0] as u64 | ((pair[1] as u64) << index_bits);
            for bit in 0..width {
                let target = row * width + bit;
                bytes[0x100 + target / 8] |= (((packed >> bit) & 1) as u8) << (target % 8);
            }
        }
        let mut cells = visible_cell_test_tree(first, count);
        let mut output = VisibleCellIndexOutput::new(domain as usize, capacity as usize);
        let remap = (remap != 0).then_some(base);
        for &seed in seeds {
            cells.cell_ids[0] = seed;
            cells.cell_visibility_counts[0] = 0;
            cells
                .append_visible_cell_indices(
                    &bytes,
                    0,
                    remap,
                    (requested != 0).then_some(&mut output),
                )
                .unwrap();
        }
        cells.cell_ids[0] = first;
        cells.cell_visibility_counts[0] = count;
        cells
            .append_visible_cell_indices(&bytes, 0, remap, (requested != 0).then_some(&mut output))
            .unwrap();
        assert_eq!(
            output.count(),
            expected_count,
            "native cell count case {case}"
        );
        assert_eq!(
            output.indices(),
            expected_indices,
            "native cell indices case {case}"
        );
        assert_eq!(output.seen, expected_mask, "native cell bitset case {case}");
        checked += 1;
    }
    assert_eq!(checked, 756);
}

#[test]
fn point_cell_ids_match_installed_instructions() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/visible_cell_cases.point-id.txt"
    ))
    .lines()
    {
        let values: Vec<u32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let mut cells = visible_cell_test_tree(0, 0);
        cells.direct_cell_ids_available = values[0] >> 16 != 0;
        cells.cell_ids = values[2..4].to_vec();
        assert_eq!(cells.point_cell_id(values[1] as usize).unwrap(), values[4]);
        checked += 1;
    }
    assert_eq!(checked, 13);
}

#[test]
fn visible_cell_mask_and_invalid_stream_boundaries() {
    assert_eq!(
        cell_visibility_mask(65, &[0, 31, 32, 64, 31]).unwrap(),
        [0x80000001, 1, 1]
    );
    assert!(cell_visibility_mask(0, &[]).unwrap().is_empty());
    assert!(cell_visibility_mask(65, &[65]).is_err());
    assert!(cell_visibility_mask(usize::MAX, &[]).is_err());
    let cells = visible_cell_test_tree(u32::MAX, 100);
    cells
        .append_visible_cell_indices(&[], usize::MAX, None, None)
        .unwrap();
    let mut output = VisibleCellIndexOutput::new(1, 0);
    assert!(cells
        .append_visible_cell_indices(&[], 0, None, Some(&mut output))
        .is_err());
    assert!(output.push_unique(1).is_err());
    output.push_unique(0).unwrap();
    assert_eq!(output.count(), 1);
    assert!(output.indices().is_empty());
    assert_eq!(cell_visibility_mask(1, output.indices()).unwrap(), [0]);
    let cells = visible_cell_test_tree(0, 1);
    let mut bytes = vec![0; 0x64];
    assert!(cells
        .append_visible_cell_indices(&bytes, 0, None, Some(&mut output))
        .unwrap_err()
        .contains("zero-width"));
    bytes[0x54..0x58].copy_from_slice(&(1u32 << 15).to_le_bytes());
    assert!(cells
        .append_visible_cell_indices(&bytes, 0, None, Some(&mut output))
        .unwrap_err()
        .contains("zero-length"));
}

#[test]
fn object_candidate_batches_match_original_32_entry_limit_across_runs() {
    let first_run = (31u64 << 6) | 0;
    let second_run = (4u64 << 6) | 31;
    let packed = first_run | (second_run << 11);
    let bytes = packed.to_le_bytes();
    let cells = CellTree {
        bounds: Vec::new(),
        children: Vec::new(),
        axes: Vec::new(),
        planes: Vec::new(),
        leaf_ranks: Vec::new(),
        cell_codes: Vec::new(),
        cell_ids: Vec::new(),
        cell_visibility_counts: Vec::new(),
        direct_cell_ids_available: true,
        cell_bounds: Vec::new(),
        cell_bounds_root: [[0.0; 3]; 2],
        cell_table_at: 0,
        cell_link_ranges: Vec::new(),
        cell_candidate_ranges: vec![(0, 35)],
        links_at: 0,
        candidates_at: 0,
        candidate_index_bits: 6,
        candidate_run_bits: 5,
        candidate_record_count: 2,
        object_count: 64,
        code_width: 0,
        portal_nodes_at: None,
        portal_planes_at: None,
    };

    let mut batches = Vec::new();
    cells
        .for_each_object_candidate_batch(&bytes, 0, |batch| {
            batches.push(batch.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [32, 3]);
    assert_eq!(
        batches.into_iter().flatten().collect::<Vec<_>>(),
        (0..35).collect::<Vec<_>>()
    );
}

#[test]
fn resolves_direct_link_to_internal_record_cell_table() {
    let mut bytes = vec![0u8; 0x140];
    bytes[0x134..0x138].copy_from_slice(&3u32.to_le_bytes());
    let tree = SpatialTree {
        bounds: Vec::new(),
        children: Vec::new(),
        axes: Vec::new(),
        planes: Vec::new(),
        offsets: vec![0, 0x100],
        global_distance_scales: None,
    };
    let link = [1, 2 << 16, 0, 0];
    assert_eq!(
        tree.direct_link_destination(&bytes, link).unwrap(),
        Some(DirectLinkDestination {
            spatial_index: 1,
            record_offset: 0x100,
            cell_index: 2
        })
    );
    assert!(tree
        .direct_link_destination(&bytes, [1, 3 << 16, 0, 0])
        .is_err());
    assert_eq!(
        tree.direct_link_destination(&bytes, [1 << 28, 0, 0, 0])
            .unwrap(),
        None
    );
}

#[test]
fn split_branch_stack_matches_original_child_insertions_and_order() {
    let tree = SpatialTree {
        bounds: Vec::new(),
        children: vec![Some([1, 2]), None, None],
        axes: vec![0, 3, 3],
        planes: vec![0.0; 3],
        offsets: vec![0; 3],
        global_distance_scales: None,
    };
    let mut bytes = [0x55; 0x28];
    bytes[0x0c..0x10].copy_from_slice(&0u32.to_le_bytes());
    bytes[0x1c] = 0;
    bytes[0x1d] = 0; // parent split axis x
    bytes[0x1e] = 0xff;
    let root = SpatialQueryBranchEntry::from_bytes(bytes);

    let mut high_first = SpatialQueryBranchStack::new(root);
    high_first
        .push_split(&tree, 3, 0x3f80_0000, false, 0x1111, 0x2222)
        .unwrap();
    assert_eq!(high_first.top(), 1);
    assert_eq!(high_first.entry(0).unwrap().spatial_node_index(), 1);
    assert_eq!(high_first.entry(0).unwrap().branch_mode(), 1);
    assert_eq!(high_first.entry(0).unwrap().query_state(), 0x1111);
    assert_eq!(high_first.entry(0).unwrap().bytes()[0x1e], 0xfd);
    assert_eq!(
        high_first.entry(0).unwrap().bytes()[0x10..0x14],
        0x3f80_0000u32.to_le_bytes()
    );
    assert_eq!(high_first.current().spatial_node_index(), 2);
    assert_eq!(high_first.current().branch_mode(), 2);
    assert_eq!(high_first.current().query_state(), 0x2222);
    assert_eq!(high_first.current().bytes()[0x1e], 0xfe);
    assert_eq!(high_first.current().bytes()[4..8], [0x55; 4]);

    let mut low_first = SpatialQueryBranchStack::new(root);
    low_first
        .push_split(&tree, 3, 0x3f80_0000, true, 0x1111, 0x2222)
        .unwrap();
    assert_eq!(low_first.current().spatial_node_index(), 1);
    assert_eq!(low_first.entry(0).unwrap().spatial_node_index(), 2);

    let mut one_child = SpatialQueryBranchStack::new(root);
    one_child
        .push_split(&tree, 1, 9, false, 0x3333, 0x4444)
        .unwrap();
    assert_eq!(one_child.top(), 0);
    assert_eq!(one_child.current().spatial_node_index(), 1);
    assert_eq!(one_child.current().query_state(), 0x3333);
}

#[test]
fn spatial_branch_scheduler_selects_or_retires_branches_from_cell_states() {
    let tree = SpatialTree {
        bounds: Vec::new(),
        children: vec![Some([1, 2]), None, None],
        axes: vec![0, 3, 3],
        planes: vec![1.0; 3],
        offsets: vec![0; 3],
        global_distance_scales: None,
    };
    let mut bytes = [0u8; 0x28];
    bytes[0x0c..0x10].copy_from_slice(&0u32.to_le_bytes());
    bytes[0x1d] = 0;
    bytes[0x24..0x28].copy_from_slice(&0u32.to_le_bytes());

    let mut queued = SpatialQueryBranchStack::new(SpatialQueryBranchEntry::from_bytes(bytes));
    let candidate = queued
        .next_leaf(
            &tree,
            &[0x8001, 0x2001, 0x2002],
            &mut [0b111],
            &[1.0],
            0,
            1,
            SpatialBranchDirection::AxisValues([0.0; 3]),
        )
        .unwrap();
    assert_eq!(
        candidate,
        Some(SpatialQueryBranchCandidate {
            spatial_node_index: 1,
            cell_state_index: 1,
        })
    );

    let mut worklists = SpatialCellWorklists::new(3);
    let initial_cell = CellWorkItem {
        entry_id: 2,
        cell_index: 0,
    };
    worklists.prepend_pending_cell(1, initial_cell).unwrap();
    let activation = worklists
        .activate_spatial_candidate(candidate.unwrap())
        .unwrap();
    assert_eq!(
        activation,
        SpatialRecordActivation {
            spatial_node_index: 1,
            pending_slot: 1,
            activated_cells: 1,
        }
    );
    assert_eq!(
        worklists.active_worklist().entries().collect::<Vec<_>>(),
        [initial_cell]
    );
    assert_eq!(worklists.cell_states()[0], 0x8002);

    let mut empty = SpatialQueryBranchStack::new(SpatialQueryBranchEntry::from_bytes(bytes));
    let mut active_nodes = [0b111];
    assert_eq!(
        empty
            .next_leaf(
                &tree,
                &[0x8001, 0, 0],
                &mut active_nodes,
                &[1.0],
                0,
                1,
                SpatialBranchDirection::AxisSides([true; 3]),
            )
            .unwrap(),
        None
    );
    assert!(!empty.is_active());
    assert_eq!(active_nodes, [0b001]);
}

#[test]
fn ordinary_renderer_inverted_sentinel_selects_camera_point_volume() {
    let point = [0.0, 0.0, 0.0];
    let sentinel = [
        f32::MAX,
        f32::MAX,
        f32::MAX,
        -f32::MAX,
        -f32::MAX,
        -f32::MAX,
    ];
    assert_eq!(
        ordinary_spatial_query_volume(sentinel, point).unwrap(),
        OrdinarySpatialQueryVolume::CameraPoint(point)
    );
    assert_eq!(
        ordinary_spatial_query_volume([-2.0, -1.0, -1.0, 2.0, 1.0, 1.0], point).unwrap(),
        OrdinarySpatialQueryVolume::Bounds([[-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]])
    );
    assert!(ordinary_spatial_query_volume(sentinel, [f32::NAN, 0.0, 0.0]).is_err());

    let tree = SpatialTree {
        bounds: vec![
            [[-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]],
            [[-2.0, -1.0, -1.0], [0.0, 1.0, 1.0]],
            [[0.0, -1.0, -1.0], [2.0, 1.0, 1.0]],
        ],
        children: vec![Some([1, 2]), None, None],
        axes: vec![0, 3, 3],
        planes: vec![0.0; 3],
        offsets: vec![1, 1, 1],
        global_distance_scales: None,
    };
    let planes = [[0.0; 4]; 6];
    assert_eq!(
        tree.record_nodes_for_ordinary_query_prefilter(sentinel, point, &planes, [0.0; 3])
            .unwrap(),
        [0, 1, 2]
    );
    assert_eq!(
        tree.record_nodes_for_ordinary_query_prefilter(
            [-1.8, 0.0, 0.0, -0.2, 0.5, 0.5],
            point,
            &planes,
            [0.0; 3]
        )
        .unwrap(),
        Vec::<usize>::new()
    );
    assert_eq!(
        tree.record_nodes_for_ordinary_query_prefilter(
            [-3.0, -2.0, -2.0, 3.0, 2.0, 2.0],
            point,
            &planes,
            [0.0; 3]
        )
        .unwrap(),
        [0, 1, 2]
    );
}

#[test]
fn original_query_root_gate_matches_all_three_volume_modes() {
    let tree = SpatialTree {
        bounds: vec![[[-2.0, -1.0, -1.0], [2.0, 1.0, 1.0]]],
        children: vec![None],
        axes: vec![3],
        planes: vec![0.0],
        offsets: vec![1],
        global_distance_scales: None,
    };
    assert!(original_query_accepts_tome_root(
        &tree,
        OriginalSpatialQueryVolume::Bounds([[-3.0; 3], [3.0; 3]])
    )
    .unwrap());
    assert!(!original_query_accepts_tome_root(
        &tree,
        OriginalSpatialQueryVolume::Bounds([[-1.0; 3], [1.0; 3]])
    )
    .unwrap());
    assert!(original_query_accepts_tome_root(
        &tree,
        OriginalSpatialQueryVolume::FourCorners([
            [0.0, 0.0, 0.0],
            [-1.0, 0.5, 0.0],
            [0.5, -0.5, 0.5],
            [1.0, 0.0, -1.0],
        ])
    )
    .unwrap());
    assert!(!original_query_accepts_tome_root(
        &tree,
        OriginalSpatialQueryVolume::FourCorners([
            [0.0, 0.0, 0.0],
            [-1.0, 0.5, 0.0],
            [0.5, -0.5, 0.5],
            [3.0, 0.0, 0.0],
        ])
    )
    .unwrap());
    assert!(original_query_accepts_tome_root(
        &tree,
        OriginalSpatialQueryVolume::Point([-2.0, 0.0, 1.0])
    )
    .unwrap());
    assert!(!original_query_accepts_tome_root(
        &tree,
        OriginalSpatialQueryVolume::Point([2.01, 0.0, 0.0])
    )
    .unwrap());
}

#[test]
fn newly_reached_active_cell_is_prepended_and_marked_queued() {
    let mut worklists = SpatialCellWorklists::new(3);
    let item = CellWorkItem {
        entry_id: 2,
        cell_index: 1,
    };
    worklists.prepend_active_cell(item).unwrap();
    assert_eq!(
        worklists.active_worklist().entries().collect::<Vec<_>>(),
        [item]
    );
    assert_eq!(worklists.cell_states(), &[0, 0x8002, 0]);
    assert!(worklists.prepend_active_cell(item).is_err());
    assert!(worklists
        .prepend_active_cell(CellWorkItem {
            entry_id: 3,
            cell_index: 1,
        })
        .is_err());
}

#[test]
fn work_node_pool_allocates_fresh_ids_then_flushes_recycled_head() {
    let mut pool = CellWorkNodePool::new(3).unwrap();
    let mut states = vec![0u16; 4];
    let mut coverage = VisitedCellCoverage::default();
    let first_owner = TraversalCellId {
        spatial_index: 5,
        cell_index: 0,
    };
    let second_owner = TraversalCellId {
        spatial_index: 8,
        cell_index: 1,
    };
    let first = pool
        .allocate(first_owner, &mut states, &mut coverage, |_, _, _| Ok(()))
        .unwrap()
        .unwrap();
    let second = pool
        .allocate(second_owner, &mut states, &mut coverage, |_, _, _| Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!((first.entry_id, second.entry_id), (1, 2));
    assert_eq!(pool.free_count(), 0);
    assert!(pool
        .allocate(
            TraversalCellId {
                spatial_index: 9,
                cell_index: 2,
            },
            &mut states,
            &mut coverage,
            |_, _, _| Ok(())
        )
        .unwrap()
        .is_none());

    let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    mask.bits[0] = 1;
    coverage.merge_incoming(first_owner, &mask).unwrap();
    coverage.merge_incoming(second_owner, &mask).unwrap();

    pool.release_processed(first, &mut states).unwrap();
    pool.release_processed(second, &mut states).unwrap();
    assert_eq!(pool.recycled_count(), 2);
    assert_eq!(states, [0x4001, 0x4002, 0, 0]);

    let mut flushed = Vec::new();
    let reused = pool
        .allocate(
            TraversalCellId {
                spatial_index: 9,
                cell_index: 2,
            },
            &mut states,
            &mut coverage,
            |old, owner, masks| {
                assert_eq!(owner, second_owner);
                assert!(masks.coverage(owner).is_some());
                flushed.push(old);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(flushed, [second]);
    assert_eq!(
        reused,
        CellWorkItem {
            entry_id: 2,
            cell_index: 2
        }
    );
    assert_eq!(pool.recycled_count(), 1);
    assert_eq!(states, [0x4001, 2, 0, 0]);
    assert!(coverage.coverage(second_owner).is_none());
    assert!(coverage.coverage(first_owner).is_some());
}

#[test]
fn failed_recycled_cell_flush_keeps_pool_head_and_state() {
    let mut pool = CellWorkNodePool::new(2).unwrap();
    let mut states = vec![0u16; 2];
    let mut coverage = VisitedCellCoverage::default();
    let owner = TraversalCellId {
        spatial_index: 7,
        cell_index: 1,
    };
    let item = pool
        .allocate(owner, &mut states, &mut coverage, |_, _, _| Ok(()))
        .unwrap()
        .unwrap();
    pool.release_processed(item, &mut states).unwrap();
    let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    mask.bits[0] = 1;
    coverage.merge_incoming(owner, &mask).unwrap();
    assert!(pool
        .allocate(
            TraversalCellId {
                spatial_index: 7,
                cell_index: 0,
            },
            &mut states,
            &mut coverage,
            |_, _, _| Err("candidate query failed".into())
        )
        .is_err());
    assert_eq!(pool.recycled_count(), 1);
    assert_eq!(states[1], 0x4001);
    assert!(coverage.coverage(owner).is_some());
}

#[test]
fn record_end_flush_queries_recycled_cells_then_returns_them_to_free_head() {
    let mut pool = CellWorkNodePool::new(3).unwrap();
    let mut states = vec![0u16; 3];
    let mut coverage = VisitedCellCoverage::default();
    let owners = [
        TraversalCellId {
            spatial_index: 4,
            cell_index: 1,
        },
        TraversalCellId {
            spatial_index: 6,
            cell_index: 2,
        },
    ];
    let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    mask.bits[0] = 1;
    let first = pool
        .allocate(owners[0], &mut states, &mut coverage, |_, _, _| Ok(()))
        .unwrap()
        .unwrap();
    let second = pool
        .allocate(owners[1], &mut states, &mut coverage, |_, _, _| Ok(()))
        .unwrap()
        .unwrap();
    coverage.merge_incoming(owners[0], &mask).unwrap();
    coverage.merge_incoming(owners[1], &mask).unwrap();
    pool.release_processed(first, &mut states).unwrap();
    pool.release_processed(second, &mut states).unwrap();

    let mut queried = Vec::new();
    assert_eq!(
        pool.flush_recycled(&mut states, &mut coverage, |item, owner, masks| {
            assert!(masks.coverage(owner).is_some());
            queried.push((item, owner));
            Ok(())
        })
        .unwrap(),
        2
    );
    assert_eq!(queried, [(second, owners[1]), (first, owners[0])]);
    assert_eq!(states, [0, 1, 2]);
    assert_eq!(pool.free_count(), 2);
    assert_eq!(pool.recycled_count(), 0);
    assert!(owners
        .iter()
        .all(|owner| coverage.coverage(*owner).is_none()));

    let reused = pool
        .allocate(
            TraversalCellId {
                spatial_index: 9,
                cell_index: 0,
            },
            &mut states,
            &mut coverage,
            |_, _, _| Ok(()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(reused.entry_id, 1);
}

#[test]
fn new_portal_coverage_schedules_unvisited_cells_by_spatial_record() {
    let mut worklists = SpatialCellWorklists::new(3);
    let mut coverage = VisitedCellCoverage::default();
    let mut incoming = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    incoming.bits[0] = 1;
    let active = CellWorkItem {
        entry_id: 2,
        cell_index: 1,
    };
    let active_id = TraversalCellId {
        spatial_index: 4,
        cell_index: 1,
    };

    assert!(worklists
        .propagate_and_schedule_unvisited_cell(
            &mut coverage,
            active_id,
            3,
            true,
            active,
            [0, 0, 4, 4],
            &incoming,
            PortalCoveragePath::Unclipped,
        )
        .unwrap());
    assert_eq!(
        worklists.active_worklist().entries().collect::<Vec<_>>(),
        [active]
    );
    assert_eq!(worklists.cell_states()[1], 0x8002);

    let pending = CellWorkItem {
        entry_id: 3,
        cell_index: 2,
    };
    let pending_id = TraversalCellId {
        spatial_index: 9,
        cell_index: 2,
    };
    assert!(worklists
        .propagate_and_schedule_unvisited_cell(
            &mut coverage,
            pending_id,
            7,
            false,
            pending,
            [0, 0, 4, 4],
            &incoming,
            PortalCoveragePath::Unclipped,
        )
        .unwrap());
    assert_eq!(worklists.pending_cell(7, 2), Some(pending));

    // New coverage for an already pending cell updates its mask without
    // allocating a second pending node.
    incoming.bits[0] = 2;
    assert!(worklists
        .propagate_and_schedule_unvisited_cell(
            &mut coverage,
            pending_id,
            7,
            false,
            pending,
            [0, 0, 4, 4],
            &incoming,
            PortalCoveragePath::Unclipped,
        )
        .unwrap());
    assert_eq!(worklists.pending_cell(7, 2), Some(pending));
}

#[test]
fn pooled_portal_transition_returns_unused_node_before_active_enqueue() {
    let mut worklists = SpatialCellWorklists::new(2);
    let mut pool = CellWorkNodePool::new(2).unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let destination = TraversalCellId {
        spatial_index: 12,
        cell_index: 1,
    };
    let mut no_bits = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    assert!(!worklists
        .propagate_and_schedule_unvisited_cell_from_pool(
            &mut pool,
            &mut coverage,
            destination,
            0,
            true,
            [0, 0, 4, 4],
            &no_bits,
            PortalCoveragePath::Unclipped,
            |_, _, _| Ok(()),
        )
        .unwrap());
    assert_eq!(pool.free_count(), 1);
    assert!(worklists.active_worklist().is_empty());
    assert_eq!(worklists.cell_states()[1], 0);
    assert!(coverage.coverage(destination).is_none());

    no_bits.bits[0] = 1;
    assert!(worklists
        .propagate_and_schedule_unvisited_cell_from_pool(
            &mut pool,
            &mut coverage,
            destination,
            0,
            true,
            [0, 0, 4, 4],
            &no_bits,
            PortalCoveragePath::Unclipped,
            |_, _, _| Ok(()),
        )
        .unwrap());
    assert_eq!(
        worklists.active_worklist().entries().collect::<Vec<_>>(),
        [CellWorkItem {
            entry_id: 1,
            cell_index: 1
        }]
    );
    assert_eq!(worklists.cell_states()[1], 0x8001);
}

#[test]
fn pooled_cross_spatial_transition_reuses_existing_pending_node() {
    let mut worklists = SpatialCellWorklists::new(3);
    let mut pool = CellWorkNodePool::new(3).unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let destination = TraversalCellId {
        spatial_index: 23,
        cell_index: 2,
    };
    // Numeric index 2 in the current record is a different local cell.
    worklists.cell_states_mut()[2] = 0x8002;
    let mut mask = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    mask.bits[0] = 1;
    assert!(worklists
        .propagate_and_schedule_unvisited_cell_from_pool(
            &mut pool,
            &mut coverage,
            destination,
            7,
            false,
            [0, 0, 4, 4],
            &mask,
            PortalCoveragePath::Unclipped,
            |_, _, _| Ok(()),
        )
        .unwrap());
    let first = worklists.pending_cell(7, 2).unwrap();
    assert_eq!(first.entry_id, 1);
    assert_eq!(pool.free_count(), 1);
    assert_eq!(worklists.cell_states()[2], 0x8002);

    mask.bits[0] = 2;
    assert!(worklists
        .propagate_and_schedule_unvisited_cell_from_pool(
            &mut pool,
            &mut coverage,
            destination,
            7,
            false,
            [0, 0, 4, 4],
            &mask,
            PortalCoveragePath::Unclipped,
            |_, _, _| Ok(()),
        )
        .unwrap());
    assert_eq!(worklists.pending_cell(7, 2), Some(first));
    assert_eq!(pool.free_count(), 1);
}

#[test]
fn cross_spatial_portal_scheduling_resolves_original_pending_slot() {
    let mut worklists = SpatialCellWorklists::new(3);
    let mut pool = CellWorkNodePool::new(3).unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let mut mapper = SpatialCellSlotMapper::default();
    let mut tome_bytes = vec![0; 0x104];
    tome_bytes[0x90..0x94].copy_from_slice(&9u32.to_le_bytes());
    tome_bytes[0x94..0x98].copy_from_slice(&2u32.to_le_bytes());
    tome_bytes[0x98..0x9c].copy_from_slice(&0x100u32.to_le_bytes());
    tome_bytes[0x100..0x104].copy_from_slice(&(1u32 << 2).to_le_bytes());
    let rows = SpatialCellSlotRows::parse(&tome_bytes).unwrap();
    let destination = TraversalCellId {
        spatial_index: 1,
        cell_index: 2,
    };
    let mut global_branch_states = vec![0u16; ORIGINAL_QUERY_CELL_STATE_COUNT];
    let mut incoming = PortalTileMask::new([0, 0, 1, 1]).unwrap();
    incoming.bits[0] = 1;

    // Target spatial record 1 selects bits 2..3. Its packed row has a
    // set first bit and then a clear bit; the scan resolves the pending
    // list head to slot 1 from the global branch states.
    assert!(worklists
        .propagate_and_schedule_cross_spatial_cell_from_pool(
            &mut mapper,
            &mut pool,
            &mut coverage,
            &rows,
            destination,
            &mut global_branch_states,
            [0, 0, 4, 4],
            &incoming,
            PortalCoveragePath::Unclipped,
            |_, _, _| Ok(()),
        )
        .unwrap());

    let pending = worklists.pending_cell(1, 2).unwrap();
    assert_eq!(worklists.cell_states()[2], 0);
    assert_eq!(worklists.activate_pending_spatial(1).unwrap(), 1);
    assert_eq!(
        worklists.active_worklist().entries().collect::<Vec<_>>(),
        [pending]
    );
    assert_eq!(worklists.cell_states()[2], pending.entry_id | 0x8000);
    assert_eq!(mapper.intermediate_slots(), &[1]);
    assert_eq!(
        global_branch_states[1],
        pending.entry_id | CellTraversalState::PROCESSED
    );
}

#[test]
fn pending_cross_record_cell_uses_selected_record_on_activation() {
    let mut worklists = SpatialCellWorklists::new(4);
    let mut pool = CellWorkNodePool::new(4).unwrap();
    let mut coverage = VisitedCellCoverage::default();
    let target = TraversalCellId {
        spatial_index: 610,
        cell_index: 2,
    };
    let mask = PortalTileMask::full([0, 0, 1, 1]).unwrap();
    let node = worklists
        .seed_initial_camera_cell(&mut pool, &mut coverage, target, 5, &mask)
        .unwrap()
        .unwrap();
    assert_eq!(worklists.activate_pending_spatial(5).unwrap(), 1);
    worklists
        .bind_active_cells_to_spatial_record(4, &mut pool, &mut coverage)
        .unwrap();
    let selected = TraversalCellId {
        spatial_index: 4,
        cell_index: 2,
    };
    assert_eq!(pool.owners[node.entry_id as usize], Some(selected));
    assert!(coverage.coverage(target).is_none());
    assert_eq!(coverage.coverage(selected), Some(&mask));
}

#[test]
fn selects_children_from_packed_codes_and_planes() {
    let mut data = vec![0u8; 0x120];
    data[0..4].copy_from_slice(&0xd600_0012u32.to_le_bytes());
    let size = data.len() as u32;
    data[8..12].copy_from_slice(&size.to_le_bytes());
    for (index, value) in [-2.0f32, -1.0, -1.0, 2.0, 1.0, 1.0].iter().enumerate() {
        data[0x14 + index * 4..0x18 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    data[0x2c..0x30].copy_from_slice(&96u32.to_le_bytes());
    data[0x30..0x34].copy_from_slice(&0x60u32.to_le_bytes());
    data[0x38..0x3c].copy_from_slice(&3u32.to_le_bytes());
    data[0x3c..0x40].copy_from_slice(&0x70u32.to_le_bytes());
    data[0xa0..0xa4].copy_from_slice(&0x80u32.to_le_bytes());
    data[0x60] = 0b0011_1100; // x split, then two leaves
    data[0x70..0x74].copy_from_slice(&0.0f32.to_le_bytes());
    let mut tree = SpatialTree::parse(&data).unwrap();
    let root_entry = tree.initial_query_branch_entry().unwrap();
    assert_eq!(root_entry.spatial_node_index(), 0);
    assert_eq!(root_entry.branch_mode(), 0);
    assert_eq!(root_entry.query_state(), 0);
    assert_eq!(
        &root_entry.bytes()[0..12],
        &[-2.0f32, -1.0, -1.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(&root_entry.bytes()[12..16], &[0; 4]);
    assert_eq!(
        &root_entry.bytes()[16..28],
        &[2.0f32, 1.0, 1.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(root_entry.bytes()[0x1c], 0); // root path length
    assert_eq!(root_entry.bytes()[0x1d], data[0x60] & 3);
    assert_eq!(&root_entry.bytes()[0x1e..0x20], &0x003fu16.to_le_bytes());
    let root_stack = SpatialQueryBranchStack::from_original_root(&tree).unwrap();
    assert_eq!(root_stack.top(), 0);
    assert_eq!(root_stack.current(), &root_entry);
    assert_eq!(tree.leaf_for_point([-1.0, 0.0, 0.0]), Some(1));
    assert_eq!(tree.leaf_for_point([0.0, 0.0, 0.0]), Some(2));
    assert_eq!(tree.leaf_for_point([3.0, 0.0, 0.0]), None);
    tree.offsets[1] = 1;
    tree.offsets[2] = 1;
    tree.offsets[0] = 1;
    assert_eq!(
        tree.record_leaves_intersecting_aabb([[-2.0, -1.0, -1.0], [-0.1, 1.0, 1.0]]),
        vec![1]
    );
    assert_eq!(
        tree.record_leaves_intersecting_aabb([[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]),
        vec![1, 2]
    );
    assert_eq!(
        tree.record_leaves_containing_point([-1.0, 0.0, 0.0]),
        vec![1]
    );
    assert_eq!(
        tree.record_leaves_containing_point([0.0, 0.0, 0.0]),
        vec![1, 2]
    );
    assert!(tree
        .record_leaves_containing_point([f32::NAN, 0.0, 0.0])
        .is_empty());
    assert!(tree
        .record_leaves_intersecting_aabb([[3.0, 0.0, 0.0], [4.0, 1.0, 1.0]])
        .is_empty());
    let camera_planes = [
        [1.0, 0.0, 0.0, 2.0],
        [-1.0, 0.0, 0.0, -0.1],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, -1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, -1.0, 1.0],
    ];
    assert_eq!(
        tree.record_nodes_for_camera_query_prefilter([4.0, 0.0, 0.0], &camera_planes, [0.0; 3])
            .unwrap(),
        vec![0, 1]
    );
    assert_eq!(
        tree.record_nodes_for_camera_query_prefilter(
            [4.0, 0.0, 0.0],
            &camera_planes,
            [0.1, 0.0, 0.0]
        )
        .unwrap(),
        vec![0, 1, 2]
    );
    assert_eq!(
        tree.record_nodes_for_camera_query_prefilter([-1.0, 0.0, 0.0], &camera_planes, [0.0; 3])
            .unwrap(),
        vec![0, 1]
    );
    assert!(tree
        .record_nodes_for_camera_query_prefilter([4.0, 0.0, 0.0], &camera_planes[..5], [0.0; 3])
        .is_err());
    let left_quad = [
        [-1.0, -0.5, -0.5],
        [-1.0, 0.5, -0.5],
        [-1.0, 0.5, 0.5],
        [-1.0, -0.5, 0.5],
    ];
    assert_eq!(tree.record_leaves_intersecting_quad(left_quad), vec![1]);
    let base = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    assert_eq!(
        tree.record_leaves_for_quad_diagnostic(base, 0, 1).unwrap(),
        vec![1, 2]
    );
}

#[test]
fn initial_camera_cell_seed_copies_full_mask_and_marks_pending_state() {
    let mut worklists = SpatialCellWorklists::new_for_original_query();
    let mut pool = CellWorkNodePool::new_for_original_query().unwrap();
    let mut coverage = VisitedCellCoverage::default();
    assert_eq!(
        worklists.cell_states().len(),
        ORIGINAL_RECORD_CELL_STATE_COUNT
    );
    // 140a85090 reserves exterior node1 before this camera-cell seed.
    assert_eq!(pool.free_count(), ORIGINAL_QUERY_WORK_NODE_COUNT - 2);
    let destination = TraversalCellId {
        spatial_index: 17,
        cell_index: 2,
    };
    let initial_mask = PortalTileMask::full([0, 0, 2, 1]).unwrap();
    let item = worklists
        .seed_initial_camera_cell(&mut pool, &mut coverage, destination, 1, &initial_mask)
        .unwrap()
        .unwrap();

    assert_eq!(
        item,
        CellWorkItem {
            entry_id: 2,
            cell_index: 2
        }
    );
    assert_eq!(worklists.pending_cell(1, 2), Some(item));
    assert_eq!(worklists.cell_states()[2], 0x8002);
    assert_eq!(coverage.coverage(destination).unwrap().bits, [u32::MAX; 2]);
    assert_eq!(
        worklists
            .seed_initial_camera_cell(&mut pool, &mut coverage, destination, 1, &initial_mask)
            .unwrap(),
        None
    );
    assert_eq!(pool.free_count(), ORIGINAL_QUERY_WORK_NODE_COUNT - 3);
}

#[test]
fn parses_spatial_record_without_local_split_tree() {
    let mut data = vec![0u8; 0x180];
    data[0x54..0x58].copy_from_slice(&(1u32 | (1 << 5)).to_le_bytes());
    data[0x58..0x5c].copy_from_slice(&0x80u32.to_le_bytes());
    let base = 0x100;
    for (index, value) in [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0].iter().enumerate() {
        data[base + index * 4..base + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    data[base + 0x34..base + 0x38].copy_from_slice(&1u32.to_le_bytes());
    data[base + 0x38..base + 0x3c].copy_from_slice(&0x50u32.to_le_bytes());
    data[base + 0x3c..base + 0x40].copy_from_slice(&0x74u32.to_le_bytes());
    for (offset, packed) in [(0x18, 0u32), (0x1c, 0x0000_ffff), (0x20, 0xffff_ffff)] {
        data[base + 0x50 + offset..base + 0x54 + offset].copy_from_slice(&packed.to_le_bytes());
    }
    let cells = CellTree::parse(&data, base as u32).unwrap();
    assert_eq!(cells.node_count(), 0);
    assert_eq!(cells.cell_count(), 1);
    assert_eq!(cells.code_counts(), (0, 0, 0));
    assert!(cells
        .cell_for_point(&data, [1.0, 1.0, 1.0])
        .unwrap()
        .is_none());
    assert_eq!(
        cells.traversal_records(&data, 0).unwrap(),
        Vec::<[u32; 4]>::new()
    );
}

#[test]
fn selects_direct_cell_code_from_local_tree() {
    let mut data = vec![0u8; 0x300];
    data[0x40..0x44].copy_from_slice(&16u32.to_le_bytes());
    data[0x54..0x58].copy_from_slice(&(4u32 | (3 << 5)).to_le_bytes());
    data[0x58..0x5c].copy_from_slice(&0x80u32.to_le_bytes());
    data[0x5c..0x60].copy_from_slice(&1u32.to_le_bytes());
    data[0x80] = 2 | (3 << 4); // object indices 2, 3, 4
    let base = 0x100;
    for (index, value) in [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0].iter().enumerate() {
        data[base + index * 4..base + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    data[base + 0x18..base + 0x1c].copy_from_slice(&(3u32 << 5 | 2).to_le_bytes());
    data[base + 0x1c..base + 0x20].copy_from_slice(&0x80u32.to_le_bytes());
    data[base + 0x20..base + 0x24].copy_from_slice(&0x90u32.to_le_bytes());
    data[base + 0x24..base + 0x28].copy_from_slice(&1u32.to_le_bytes());
    data[base + 0x28..base + 0x2c].copy_from_slice(&0xa0u32.to_le_bytes());
    data[base + 0x34..base + 0x38].copy_from_slice(&(2u32 | (1 << 16)).to_le_bytes());
    data[base + 0x38..base + 0x3c].copy_from_slice(&0xb0u32.to_le_bytes());
    data[base + 0x3c..base + 0x40].copy_from_slice(&0x120u32.to_le_bytes());
    data[base + 0x80] = 0b0011_1100;
    data[base + 0x90] = 0b0000_0100; // leaf codes 0, 1
    data[base + 0xa0..base + 0xa4].copy_from_slice(&1.0f32.to_le_bytes());
    data[base + 0xc0..base + 0xc4].copy_from_slice(&7u32.to_le_bytes());
    data[base + 0xe4..base + 0xe8].copy_from_slice(&9u32.to_le_bytes());
    data[base + 0xb4..base + 0xb8].copy_from_slice(&1u32.to_le_bytes());
    data[base + 0xbc..base + 0xc0].copy_from_slice(&3u32.to_le_bytes());
    for (offset, packed) in [(0x18, 0u32), (0x1c, 0x0000_ffff), (0x20, 0xffff_8000)] {
        data[base + 0xb0 + offset..base + 0xb4 + offset].copy_from_slice(&packed.to_le_bytes());
    }
    for (index, value) in [2u32, 3, 4, 5].iter().enumerate() {
        data[base + 0x120 + index * 4..base + 0x124 + index * 4]
            .copy_from_slice(&value.to_le_bytes());
    }
    let cells = CellTree::parse(&data, base as u32).unwrap();
    let low = cells
        .cell_for_point(&data, [0.5, 1.0, 1.0])
        .unwrap()
        .unwrap();
    let high = cells
        .cell_for_point(&data, [1.0, 1.0, 1.0])
        .unwrap()
        .unwrap();
    assert_eq!(
        (low.tree_node, low.leaf_rank, low.cell_index, low.cell_id),
        (1, 0, 0, 7)
    );
    let mut seeded_worklists = SpatialCellWorklists::new(2);
    let mut seed_pool = CellWorkNodePool::new(4).unwrap();
    let mut seed_coverage = VisitedCellCoverage::default();
    let initial_mask = PortalTileMask::full([0, 0, 2, 1]).unwrap();
    let selected_tree = SpatialTree {
        bounds: vec![[[0.0; 3], [2.0; 3]]; 2],
        children: vec![None; 2],
        axes: vec![3; 2],
        planes: vec![0.0; 2],
        offsets: vec![base as u32; 2],
        global_distance_scales: None,
    };
    let seeded = selected_tree
        .seed_initial_camera_cell_for_candidate(
            &data,
            [0.5, 1.0, 1.0],
            SpatialQueryBranchCandidate {
                spatial_node_index: 0,
                cell_state_index: 17,
            },
            &mut seeded_worklists,
            &mut seed_pool,
            &mut seed_coverage,
            &initial_mask,
        )
        .unwrap()
        .unwrap();
    assert_eq!(seeded.spatial_node_index, 0);
    assert_eq!(seeded.pending_slot, 17);
    assert_eq!(
        (seeded.selection.cell_index, seeded.selection.cell_id),
        (0, 7)
    );
    assert_eq!(
        seeded.work_item,
        CellWorkItem {
            entry_id: 1,
            cell_index: 0
        }
    );
    assert_eq!(seeded_worklists.cell_states()[0], 0x8001);
    assert_eq!(seeded_worklists.pending_cell(17, 0), Some(seeded.work_item));
    let second_seed = selected_tree
        .seed_initial_camera_cell_for_candidate(
            &data,
            [0.5, 1.0, 1.0],
            SpatialQueryBranchCandidate {
                spatial_node_index: 1,
                cell_state_index: 18,
            },
            &mut seeded_worklists,
            &mut seed_pool,
            &mut seed_coverage,
            &initial_mask,
        )
        .unwrap()
        .unwrap();
    assert_eq!(seeded_worklists.cell_states()[0], 0x8002);
    assert_eq!(seeded_worklists.pending_cell(17, 0), Some(seeded.work_item));
    assert_eq!(
        seeded_worklists.pending_cell(18, 0),
        Some(second_seed.work_item)
    );
    assert_eq!(
        (
            high.tree_node,
            high.leaf_rank,
            high.cell_index,
            high.cell_id
        ),
        (2, 1, 1, 9)
    );
    assert_eq!(low.link_count, 1);
    let decoded = cells.cell_bounds(0).unwrap();
    assert_eq!(decoded[0], [0.0, 0.0, 0.0]);
    assert_eq!(decoded[1][0], 2.0);
    assert_eq!(decoded[1][1], 2.0);
    assert!((decoded[1][2] - 1.0000153).abs() < 0.000001);
    data[base + 0x124..base + 0x128].copy_from_slice(&((1u32 << 16) | 3).to_le_bytes());
    assert_eq!(
        cells.traversal_records(&data, 0).unwrap(),
        vec![[2, (1 << 16) | 3, 4, 5]]
    );
    assert_eq!(
        cells
            .first_ready_same_spatial_cell_target(
                &data,
                2,
                0,
                LinkQueryDirection::Volume {
                    center: [0; 4],
                    tolerance: [0; 4],
                },
                &[0, 0x8003],
            )
            .unwrap(),
        Some(ReadyCellTarget {
            cell_index: 1,
            worklist_entry: 3,
        })
    );
    let mut spatial_lists = SpatialCellWorklists::new(2);
    spatial_lists
        .prepend_pending_cell(
            17,
            CellWorkItem {
                entry_id: 2,
                cell_index: 1,
            },
        )
        .unwrap();
    spatial_lists
        .prepend_pending_cell(
            17,
            CellWorkItem {
                entry_id: 1,
                cell_index: 0,
            },
        )
        .unwrap();
    assert_eq!(
        spatial_lists.pending_cell(17, 1),
        Some(CellWorkItem {
            entry_id: 2,
            cell_index: 1,
        })
    );
    assert_eq!(spatial_lists.activate_pending_spatial(17).unwrap(), 2);
    assert_eq!(
        spatial_lists
            .active_worklist()
            .entries()
            .collect::<Vec<_>>(),
        vec![
            CellWorkItem {
                entry_id: 1,
                cell_index: 0,
            },
            CellWorkItem {
                entry_id: 2,
                cell_index: 1,
            },
        ]
    );
    assert_eq!(spatial_lists.cell_states(), &[0x8001, 0x8002]);
    assert_eq!(
        spatial_lists
            .select_active_cell(
                &cells,
                &data,
                2,
                LinkQueryDirection::Volume {
                    center: [0; 4],
                    tolerance: [0; 4],
                },
            )
            .unwrap(),
        Some(CellWorkItem {
            entry_id: 2,
            cell_index: 1,
        })
    );
    assert_eq!(
        spatial_lists
            .active_worklist()
            .entries()
            .collect::<Vec<_>>(),
        vec![CellWorkItem {
            entry_id: 1,
            cell_index: 0,
        }]
    );
    assert_eq!(spatial_lists.cell_states(), &[0x8001, 0x8002]);
    assert_eq!(cells.object_candidates(&data, 0).unwrap(), vec![2, 3, 4]);

    // The original scan ends on its first link to another spatial record,
    // even when a later link points back to this one.
    let mut ordered = data.clone();
    ordered[base + 0xb4..base + 0xb8].copy_from_slice(&2u32.to_le_bytes());
    ordered[base + 0x120..base + 0x124].copy_from_slice(&3u32.to_le_bytes());
    ordered[base + 0x130..base + 0x134].copy_from_slice(&2u32.to_le_bytes());
    ordered[base + 0x134..base + 0x138].copy_from_slice(&((1u32 << 16) | 3).to_le_bytes());
    let ordered_cells = CellTree::parse(&ordered, base as u32).unwrap();
    assert_eq!(
        ordered_cells
            .first_ready_same_spatial_cell_target(
                &ordered,
                2,
                0,
                LinkQueryDirection::Volume {
                    center: [0; 4],
                    tolerance: [0; 4],
                },
                &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0x8003],
            )
            .unwrap(),
        None
    );
}

#[test]
fn resolves_portal_code_with_authored_plane_and_terminal_children() {
    let mut data = vec![0u8; 0x400];
    data[0x54..0x58].copy_from_slice(&1u32.to_le_bytes());
    let base = 0x100;
    for (index, value) in [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0].iter().enumerate() {
        data[base + index * 4..base + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    data[base + 0x18..base + 0x1c].copy_from_slice(&(1u32 << 5 | 2).to_le_bytes());
    for (field, offset) in [
        (0x1c, 0x80u32),
        (0x20, 0x90),
        (0x38, 0xb0),
        (0x40, 0x100),
        (0x48, 0x110),
    ] {
        data[base + field..base + field + 4].copy_from_slice(&offset.to_le_bytes());
    }
    data[base + 0x34..base + 0x38].copy_from_slice(&(2u32 | (1 << 16)).to_le_bytes());
    data[base + 0x80] = 3; // one local leaf
    data[base + 0x90] = 2; // high-bit portal code, node zero
    data[base + 0xc0..base + 0xc4].copy_from_slice(&7u32.to_le_bytes());
    data[base + 0xe4..base + 0xe8].copy_from_slice(&9u32.to_le_bytes());
    data[base + 0x100..base + 0x104].copy_from_slice(&0xc000_0000u32.to_le_bytes());
    data[base + 0x104..base + 0x108].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    for (index, value) in [1.0f32, 0.0, 0.0, -1.0].iter().enumerate() {
        data[base + 0x110 + index * 4..base + 0x114 + index * 4]
            .copy_from_slice(&value.to_le_bytes());
    }
    let cells = CellTree::parse(&data, base as u32).unwrap();
    assert_eq!(
        cells
            .cell_for_point(&data, [0.5, 0.5, 0.5])
            .unwrap()
            .unwrap()
            .cell_id,
        7
    );
    assert_eq!(
        cells
            .cell_for_point(&data, [1.5, 0.5, 0.5])
            .unwrap()
            .unwrap()
            .cell_id,
        9
    );
}
