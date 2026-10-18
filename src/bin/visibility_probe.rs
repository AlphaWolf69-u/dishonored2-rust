//! Replay the live client's Tome query without creating a window or GPU device.
//! Multiple jobs run serially here: timings are query CPU work, not GPU/frame time.
use dishonored2_rust::tome;
use std::{env, fs, time::Instant};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let path = args.first().ok_or(
        "usage: visibility-probe <map.tome> --pose <16 row-major floats> [--viewport width height] [--sweep] [--jobs 1..32] [--repeat N] [--uncached] [--timing-only] [--sphere x y z radius]",
    )?;
    let parse = |flag: &str, count: usize| -> Result<Vec<f32>, String> {
        let at = args
            .iter()
            .position(|arg| arg == flag)
            .ok_or_else(|| format!("missing {flag}"))?;
        (1..=count)
            .map(|i| {
                args.get(at + i)
                    .ok_or_else(|| format!("{flag} needs {count} values"))?
                    .parse::<f32>()
                    .map_err(|error| format!("{flag}: {error}"))
            })
            .collect()
    };
    let integer = |flag: &str, default: u32| -> Result<u32, String> {
        match args.iter().position(|arg| arg == flag) {
            None => Ok(default),
            Some(at) => args
                .get(at + 1)
                .ok_or_else(|| format!("missing {flag} value"))?
                .parse::<u32>()
                .map_err(|error| format!("{flag}: {error}")),
        }
    };
    let values = parse("--pose", 16)?;
    let pose: [[f32; 4]; 4] =
        std::array::from_fn(|row| std::array::from_fn(|col| values[row * 4 + col]));
    let viewport = if args.iter().any(|arg| arg == "--viewport") {
        let values = parse("--viewport", 2)?;
        if values
            .iter()
            .any(|v| !v.is_finite() || *v < 1.0 || *v > 16384.0 || v.fract() != 0.0)
        {
            return Err("invalid viewport dimensions".into());
        }
        [values[0] as u32, values[1] as u32]
    } else {
        [1920, 1080]
    };
    let job_count = tome::original_world_query_job_count(
        integer("--jobs", tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT)?,
        tome::ORIGINAL_DEFAULT_SCHEDULER_THREADING_MODE,
    )?;
    let repeat = integer("--repeat", 1)?;
    if !(1..=10000).contains(&repeat) {
        return Err("--repeat must be in 1..10000".into());
    }
    let uncached = args.iter().any(|arg| arg == "--uncached");
    let timing_only = args.iter().any(|arg| arg == "--timing-only");
    let sphere = args
        .iter()
        .any(|arg| arg == "--sphere")
        .then(|| parse("--sphere", 4))
        .transpose()?
        .map(|values| <[f32; 4]>::try_from(values).unwrap());
    if sphere.is_some_and(|sphere| sphere.iter().any(|value| !value.is_finite())) {
        return Err("nonfinite sphere input".into());
    }
    if sphere.is_some() && uncached {
        return Err("--sphere requires cached point-cell views".into());
    }
    let load_started = Instant::now();
    let bytes = fs::read(path)?;
    let sphere_graph = sphere
        .map(|_| tome::SphereCellGraph::parse(&bytes))
        .transpose()?;
    let mut sphere_worker = tome::SphereCellWorker::default();
    let tree = tome::SpatialTree::parse(&bytes)?;
    let gates = tome::initial_gate_states(&bytes)?;
    let data = if uncached {
        None
    } else {
        Some(tome::DirectTomeQueryData::parse(&bytes, &tree)?)
    };
    println!(
        "LOAD cached={} decoded_records={} elapsed_ms={:.3}",
        !uncached,
        data.as_ref().map_or(0, |data| data.decoded_record_count()),
        load_started.elapsed().as_secs_f64() * 1000.0
    );
    let sweep = args.iter().any(|arg| arg == "--sweep");
    let angles: Vec<f32> = if sweep {
        (0..720).map(|i| i as f32 * 0.5).collect()
    } else {
        vec![0.0]
    };
    let mut errors = 0;
    let mut empty = 0;
    let mut min_count = usize::MAX;
    let mut max_count = 0;
    let mut total_ns = 0;
    let mut min_ns = u128::MAX;
    let mut max_ns = 0;
    let mut samples = 0;
    for angle in angles {
        let (sin, cos) = angle.to_radians().sin_cos();
        let mut rotated = pose;
        if angle != 0.0 {
            // Yaw about game-space Z, leaving the camera origin and pitch
            // intact. Rotating local right/forward would also roll a pitched view.
            for column in 0..3 {
                rotated[0][column] = pose[0][column] * cos - pose[1][column] * sin;
                rotated[1][column] = pose[0][column] * sin + pose[1][column] * cos;
            }
        }
        let rows = tome::original_world_visibility_query_matrix(
            80.0,
            viewport[0],
            viewport[1],
            1.0,
            rotated,
            0.01,
            7000.0,
        )?;
        let input = tome::OrdinaryDirectQueryInput {
            camera_point: [rotated[0][3], rotated[1][3], rotated[2][3]],
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (viewport[0] as f32 * viewport[1] as f32),
            global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_distance_scale: 1.0,
            winding_flip: tome::query_matrix_winding_flip(rows),
            gate_states: Some(&gates),
            output_capacity: tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
        };
        for sample in 0..repeat {
            let started = Instant::now();
            let jobs: Vec<_> = (0..job_count)
                .map(|job| {
                    if let Some(data) = &data {
                        tome::ordinary_direct_tome_render_job(
                            &tree, &bytes, data, &input, job, job_count,
                        )
                    } else {
                        tome::ordinary_direct_tome_render_job_uncached(
                            &tree, &bytes, &input, job, job_count,
                        )
                    }
                })
                .collect();
            let result = tome::merge_render_query_outputs(&jobs);
            let ns = started.elapsed().as_nanos();
            total_ns += ns;
            min_ns = min_ns.min(ns);
            max_ns = max_ns.max(ns);
            samples += 1;
            let failed_jobs = jobs.iter().filter(|job| job.result.is_err()).count();
            let (status, count) = match &result {
                Ok(Some(output)) => {
                    if failed_jobs > 0 {
                        errors += 1;
                    }
                    (
                        if failed_jobs == 0 {
                            "ok".to_owned()
                        } else {
                            format!("ok_with_job_errors: {failed_jobs}")
                        },
                        output.indices().len(),
                    )
                }
                Ok(None) => ("outside_tome_root".to_owned(), 0),
                Err(error) => {
                    errors += 1;
                    (format!("error: {error}"), 0)
                }
            };
            empty += usize::from(count == 0);
            min_count = min_count.min(count);
            max_count = max_count.max(count);
            if !timing_only || result.is_err() || failed_jobs > 0 {
                let cell_entries = result
                    .as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .map_or(0, |output| output.cell_indices().len());
                println!("yaw_delta={angle} sample={sample} jobs={job_count} status={status:?} objects={count} visible_cell_entries={cell_entries} query_ms={:.3}", ns as f64 / 1e6);
                for job in &jobs {
                    println!("{:?}", job.diagnostics);
                }
                if !sweep && repeat == 1 {
                    println!("pose={rotated:?}\nrender_rows={rows:?}");
                    if let Ok(Some(output)) = &result {
                        println!("indices={:?}", output.indices());
                        println!("cell_indices={:?}", output.cell_indices());
                        if let (Some(sphere), Some(graph), Some(data)) =
                            (sphere, &sphere_graph, &data)
                        {
                            let point_cell = data.point_cell_id(
                                &bytes,
                                &tree,
                                [sphere[0], sphere[1], sphere[2]],
                            )?;
                            let mask = tome::cell_visibility_mask(
                                data.cell_count(),
                                output.cell_indices(),
                            )?;
                            let result = graph.query_spheres(
                                &bytes,
                                &mut sphere_worker,
                                &[sphere],
                                &[point_cell],
                                &mask,
                                Some(&gates),
                                None,
                                1,
                            )?;
                            println!("SPHERE point_cell={point_cell:?} accepted={:?} count={} error_code={}",
                                result.indices, result.count, result.error_code);
                        }
                        println!("merged_state={:?}", output.state);
                    }
                }
            }
        }
    }
    println!("SUMMARY cached={} jobs={job_count} samples={samples} errors={errors} empty={empty} min={min_count} max={max_count} query_ms[min,mean,max]=[{:.3},{:.3},{:.3}]",
        !uncached, min_ns as f64 / 1e6, total_ns as f64 / samples as f64 / 1e6, max_ns as f64 / 1e6);
    if errors > 0 {
        return Err(format!("{errors} visibility queries failed").into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Visibility probe: {error}");
        std::process::exit(1);
    }
}
