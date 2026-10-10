//! Read-only coverage check for compiled MD6 structural decoding.
#[path = "../archive.rs"]
mod archive;
#[path = "../bmd6.rs"]
mod bmd6;

use std::{collections::BTreeMap, env, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = env::args()
        .nth(1)
        .unwrap_or_else(|| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
    let filter = env::args().nth(2).unwrap_or_default().to_ascii_lowercase();
    let catalog = archive::Catalog::open(Path::new(&base))?;
    let paths = catalog.paths_with_suffix(".bmd6model");
    let mut total = 0usize;
    let mut decoded = 0usize;
    let mut reasons = BTreeMap::<String, usize>::new();
    let mut declarations = BTreeMap::<(u32, u32), usize>::new();
    let mut trailing_bytes = BTreeMap::<&'static str, usize>::new();
    let mut surfaces_total = 0usize;
    let mut geometry_decoded = 0usize;
    let mut uv_nonfinite = 0usize;
    let mut uv_extreme = 0usize;
    let mut failures_shown = 0usize;
    for path in paths.into_iter().filter(|path| path.contains(&filter)) {
        total += 1;
        let bytes = catalog
            .read(&path)?
            .ok_or_else(|| format!("indexed MD6 resource missing: {path}"))?;
        let result = bmd6::inspect(&bytes);
        match result {
            Ok(info) => {
                decoded += 1;
                if info.has_decoded_geometry {
                    geometry_decoded += 1;
                }
                let trailing_group = match info.bytes_after_surfaces {
                    0 => "none",
                    1..=64 => "1-64",
                    65..=1024 => "65-1024",
                    _ => "over 1024",
                };
                *trailing_bytes.entry(trailing_group).or_default() += 1;
                surfaces_total += info.surfaces.len();
                for surface in &info.surfaces {
                    *declarations
                        .entry((
                            surface.cpu_vertex_declaration,
                            surface.gpu_vertex_declaration,
                        ))
                        .or_default() += 1;
                    if surface.uvs.iter().flatten().any(|v| !v.is_finite()) {
                        uv_nonfinite += 1;
                    }
                    if surface.uvs.iter().flatten().any(|v| v.abs() > 1_000.0) {
                        uv_extreme += 1;
                    }
                }
                if !filter.is_empty() {
                    println!(
                        "{path}: {} surfaces, {} trailing bytes, geometry={}",
                        info.surfaces.len(),
                        info.bytes_after_surfaces,
                        info.has_decoded_geometry
                    );
                    for surface in info.surfaces {
                        let mut low = [f32::INFINITY; 3];
                        let mut high = [f32::NEG_INFINITY; 3];
                        for point in &surface.positions {
                            for axis in 0..3 {
                                low[axis] = low[axis].min(point[axis]);
                                high[axis] = high[axis].max(point[axis]);
                            }
                        }
                        let mut uv_low = [f32::INFINITY; 2];
                        let mut uv_high = [f32::NEG_INFINITY; 2];
                        for uv in &surface.uvs {
                            for axis in 0..2 {
                                uv_low[axis] = uv_low[axis].min(uv[axis]);
                                uv_high[axis] = uv_high[axis].max(uv[axis]);
                            }
                        }
                        println!(
                            "  {} | {} | CPU={} GPU={} vertices={} indices={} attachments={}",
                            surface.name,
                            surface.material_path,
                            surface.cpu_vertex_declaration,
                            surface.gpu_vertex_declaration,
                            surface.vertex_count,
                            surface.index_count,
                            surface.attachment_count
                        );
                        println!("    decoded bounds={low:?}..{high:?} uv={uv_low:?}..{uv_high:?}");
                    }
                }
            }
            Err(error) => {
                let category = if error.starts_with("invalid MD6 surface count:") {
                    "invalid MD6 surface count".to_string()
                } else if error.contains("CPU vertex declaration") {
                    "unsupported CPU vertex declaration".to_string()
                } else {
                    error.clone()
                };
                *reasons.entry(category).or_default() += 1;
                if !filter.is_empty() || failures_shown < 12 {
                    println!("{path}: {error}");
                    if failures_shown < 3 {
                        println!("  prefix={:02x?}", &bytes[..bytes.len().min(128)]);
                    }
                    failures_shown += 1;
                }
            }
        }
    }
    println!("MD6 structures: {decoded}/{total} decoded");
    println!("MD6 geometry: {geometry_decoded}/{decoded} finite primary streams");
    println!("UV surfaces: {uv_nonfinite} nonfinite; {uv_extreme} with magnitude over 1000");
    println!("Surfaces: {surfaces_total}");
    for ((cpu, gpu), count) in declarations {
        println!("{count:>6} surfaces with CPU declaration {cpu}, GPU declaration {gpu}");
    }
    for (group, count) in trailing_bytes {
        println!("{count:>6} models with trailing bytes: {group}");
    }
    for (reason, count) in reasons {
        println!("{count:>6} {reason}");
    }
    Ok(())
}
