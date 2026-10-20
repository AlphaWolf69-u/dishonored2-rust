use dishonored2_rust::archive;
use dishonored2_rust::bprobe;
use dishonored2_rust::pnetwork;
use dishonored2_rust::probe_lighting;

use std::{collections::BTreeMap, env, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("Usage: probe_audit <extracted .bprobe> | --catalog <fragment> | --paths <suffix> [fragment] | --search <path fragment> | --show <resource>")?;
    if path == "--list-networks" {
        let catalog =
            archive::Catalog::open(&dishonored2_rust::assets::installation::game_base()?)?;
        for path in catalog.paths_with_suffix(".probesnetwork") {
            let bytes = catalog
                .read_prefix(&path, 1024 * 1024)?
                .ok_or("missing network")?;
            let (_, names, _, _) = pnetwork::ordered_probe_names(&bytes)?;
            println!(
                "{} {path} {}",
                names.len(),
                names.first().map(String::as_str).unwrap_or("")
            );
        }
        return Ok(());
    }
    if path == "--show" {
        let resource = env::args()
            .nth(2)
            .ok_or("--show requires a resource path")?;
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let Some(bytes) = catalog.read(&resource)? else {
            return Err(format!("resource not found: {resource}").into());
        };
        print!("{}", String::from_utf8_lossy(&bytes));
        return Ok(());
    }
    if path == "--search" {
        let fragment = env::args()
            .nth(2)
            .ok_or("--search requires a path fragment")?;
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = archive::Catalog::open(Path::new(&base))?;
        for resource in catalog
            .paths_containing(&fragment)
            .into_iter()
            .filter(|resource| !resource.ends_with(".bprobe"))
        {
            println!("{resource}");
        }
        return Ok(());
    }
    if path == "--network" {
        let resource = env::args()
            .nth(2)
            .ok_or("--network requires a catalog resource path")?;
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let bytes = catalog
            .read_prefix(&resource, 1024 * 1024)?
            .ok_or("resource not found")?;
        println!("{resource}: {} bytes", bytes.len());
        let (version, names, secondary, end) =
            pnetwork::ordered_probe_names(&bytes).map_err(std::io::Error::other)?;
        let present = names
            .iter()
            .filter(|name| catalog.contains(&format!("{name}.bprobe")))
            .count();
        println!("PNW version {version}: {} ordered probe resources; {} resolve to installed .bprobe files; {} secondary resources; parsed lists end at {end:#x}", names.len(), present, secondary.len());
        for (index, name) in names.iter().take(8).enumerate() {
            let position = catalog
                .read(&format!("{name}.bprobe"))?
                .and_then(|payload| bprobe::parse(&payload).ok())
                .map(|probe| probe.position);
            println!("  {index}: {name}.bprobe position={position:?}");
        }
        if names.len() > 8 {
            println!("  … {} more", names.len() - 8);
        }
        for (index, name) in secondary.iter().take(4).enumerate() {
            println!("  secondary {index}: {name}");
        }
        let spatial_offset = u32::from_le_bytes(bytes[end..end + 4].try_into()?);
        let cell_count = u32::from_le_bytes(bytes[end + 5..end + 9].try_into()?);
        println!("Voronoi section version {spatial_offset}, cell count {cell_count}");
        for (offset, chunk) in bytes[end..end + 160].chunks(16).enumerate() {
            print!("{:08x}: ", end + offset * 16);
            for byte in chunk {
                print!("{byte:02x} ");
            }
            println!();
        }
        let query = (env::args().nth(3), env::args().nth(4), env::args().nth(5));
        if let (Some(x), Some(y), Some(z)) = query {
            let query = [x.parse::<f32>()?, y.parse::<f32>()?, z.parse::<f32>()?];
            let mut nearest = Vec::with_capacity(names.len());
            for name in &names {
                let Some(payload) = catalog.read(&format!("{name}.bprobe"))? else {
                    continue;
                };
                let Ok(probe) = bprobe::parse(&payload) else {
                    continue;
                };
                let distance = (0..3)
                    .map(|axis| (probe.position[axis] - query[axis]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                nearest.push((distance, name, probe.position));
            }
            nearest.sort_by(|left, right| left.0.total_cmp(&right.0));
            println!("Nearest parsed probes to {query:?}:");
            for (distance, name, position) in nearest.iter().take(8) {
                println!("  {distance:.3} m | {name}.bprobe | {position:?}");
            }
        }
        return Ok(());
    }
    if path == "--resolve" {
        let stem = env::args()
            .nth(2)
            .ok_or("--resolve requires a world-file stem")?;
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let started = std::time::Instant::now();
        let Some(set) =
            probe_lighting::load_for_world(&catalog, &stem).map_err(std::io::Error::other)?
        else {
            println!("No unique PNW network matches {stem:?}");
            return Ok(());
        };
        let probe_index_counts = set
            .spatial
            .tetrahedra
            .iter()
            .flat_map(|tetra| tetra.probe_indices)
            .fold((0usize, 0u16), |(invalid, maximum), index| {
                (
                    invalid + usize::from(index as usize >= set.probes.len() && index != u16::MAX),
                    maximum.max(index),
                )
            });
        println!(
            "Loaded {}: {} probes parsed, {} secondary resources, {} rejected; Voronoi v{} cells {}, tetrahedra {}, tetra vertices {}; probe index max {}, out-of-range references {}; {:.2}s",
            set.network_resource,
            set.probes.len(),
            set.secondary_resources.len(),
            set.rejected_resources.len(),
            set.spatial.voronoi_version,
            set.spatial.cell_count,
            set.spatial.tetrahedra.len(),
            set.spatial.vertices.len(),
            probe_index_counts.1,
            probe_index_counts.0,
            started.elapsed().as_secs_f32()
        );
        if let (Some(x), Some(y), Some(z)) =
            (env::args().nth(3), env::args().nth(4), env::args().nth(5))
        {
            let query = [x.parse::<f32>()?, y.parse::<f32>()?, z.parse::<f32>()?];
            let mut nearest: Vec<_> = set
                .probes
                .iter()
                .map(|probe| {
                    let distance = (0..3)
                        .map(|axis| (probe.position[axis] - query[axis]).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    (distance, probe)
                })
                .collect();
            nearest.sort_by(|left, right| left.0.total_cmp(&right.0));
            for (distance, probe) in nearest.into_iter().take(8) {
                println!(
                    "{distance:.3} m | {} | {:?} | static SH L0 {:?} | cubemap {:?}",
                    probe.resource, probe.position, probe.static_lighting_sh[0], probe.cubemap_name,
                );
            }
            match set.interpolate_static_sh(query) {
                Some(interpolated) => {
                    println!(
                        "Exact PNW tetrahedron {}: barycentrics {:?}; probes {:?}; authored weights {:?}; probe static-light illuminance {:?}, sky factor {}, sun factor {}, illuminance estimate {:?}; interpolated static/sun/filtered-sun SH L0 {:?}/{:?}/{:?}",
                        interpolated.tetrahedron_index,
                        interpolated.barycentrics,
                        interpolated.probe_indices,
                        interpolated.probe_weights,
                        interpolated.static_lights_illuminance,
                        interpolated.sky_occlusion_factor,
                        interpolated.sun_irradiance_factor,
                        interpolated.illuminance_estimate,
                        interpolated.static_lighting_sh[0],
                        interpolated.sun_factor_sh[0],
                        interpolated.filtered_sun_factor_sh[0],
                    );
                    for slot in 0..4 {
                        if let Some(probe) =
                            set.probes.get(interpolated.probe_indices[slot] as usize)
                        {
                            println!(
                                "  slot {slot}: weight {:.6}, {} at {:?}",
                                interpolated.probe_weights[slot], probe.resource, probe.position
                            );
                        }
                    }
                    let tetrahedron = set.spatial.tetrahedra[interpolated.tetrahedron_index];
                    println!(
                        "  ordered probe 610 position: {:?}",
                        set.probes.get(610).map(|probe| probe.position)
                    );
                    for (vertex_slot, &vertex_index) in
                        tetrahedron.vertex_indices.iter().enumerate()
                    {
                        let vertex = set.spatial.vertices[vertex_index as usize];
                        println!(
                            "  tetra vertex {vertex_slot}: index {vertex_index}, position {:?}, packed weights {:?}",
                            vertex.position, vertex.probe_weights
                        );
                    }
                }
                None => println!("No PNW tetrahedron contains {query:?}"),
            }
        }
        return Ok(());
    }
    if path == "--paths" {
        let suffix = env::args()
            .nth(2)
            .ok_or("--paths requires a resource suffix")?;
        let fragment = env::args().nth(3).unwrap_or_default().to_ascii_lowercase();
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let paths = catalog
            .paths_with_suffix(&suffix)
            .into_iter()
            .filter(|resource| resource.contains(&fragment));
        for resource in paths {
            println!("{resource}");
        }
        return Ok(());
    }
    if path == "--catalog" {
        let fragment = env::args().nth(2).unwrap_or_default();
        let base = dishonored2_rust::assets::installation::game_base()?;
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let mut versions = BTreeMap::<(u32, u32), usize>::new();
        let mut total = 0usize;
        let mut errors = 0usize;
        let mut additional = 0usize;
        let mut examples = Vec::new();
        for resource in catalog
            .paths_with_suffix(".bprobe")
            .into_iter()
            .filter(|resource| fragment.is_empty() || resource.contains(&fragment))
        {
            if examples.len() < 8 {
                examples.push(resource.clone());
            }
            let Some(bytes) = catalog.read(&resource)? else {
                continue;
            };
            total += 1;
            match bprobe::parse(&bytes) {
                Ok(probe) => {
                    *versions
                        .entry((probe.version, probe.generation))
                        .or_default() += 1;
                    additional += usize::from(probe.has_additional_results);
                }
                Err(error) => {
                    errors += 1;
                    eprintln!("{resource}: {error}");
                }
            }
        }
        println!(
            "Parsed {total} installed .bprobe resources matching {fragment:?}; errors {errors}; additional-results flags {additional}"
        );
        for ((version, generation), count) in versions {
            println!("  loader version {version}, generation {generation}: {count}");
        }
        for resource in examples {
            println!("  {resource}");
        }
        return Ok(());
    }
    let bytes = fs::read(&path)?;
    let probe = bprobe::parse(&bytes).map_err(std::io::Error::other)?;
    let finite_sh = probe
        .static_lighting_sh
        .iter()
        .flatten()
        .filter(|value| value.is_finite())
        .count();
    let sample_bounds = probe.samples.iter().fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(mut low, mut high), sample| {
            for axis in 0..3 {
                low[axis] = low[axis].min(sample.position[axis]);
                high[axis] = high[axis].max(sample.position[axis]);
            }
            (low, high)
        },
    );
    println!(
        "{}: marker {}, version {}, generation {}, position {:?}, {} bytes consumed / {}, {} samples, static SH finite {}/{}, sample bounds {:?} .. {:?}, cubemap {:?}, additional results {}",
        Path::new(&path).display(),
        probe.marker,
        probe.version,
        probe.generation,
        probe.position,
        probe.bytes_consumed,
        bytes.len(),
        probe.samples.len(),
        finite_sh,
        probe.static_lighting_sh.len() * 3,
        sample_bounds.0,
        sample_bounds.1,
        probe.cubemap_name,
        probe.has_additional_results,
    );
    println!(
        "SH means: static {:?}; sun {:?}; filtered sun {:?}; sky {:?}; filtered sky {:?}; first sample direction {:?}, values {:?}, flags {:?}",
        probe
            .static_lighting_sh
            .iter()
            .flatten()
            .sum::<f32>() / 27.0,
        probe.sun_factor_sh.iter().flatten().sum::<f32>() / 27.0,
        probe
            .filtered_sun_factor_sh
            .iter()
            .flatten()
            .sum::<f32>() / 27.0,
        probe.sky_occlusion_sh.iter().sum::<f32>() / 9.0,
        probe.filtered_sky_occlusion_sh.iter().sum::<f32>() / 9.0,
        probe.samples.first().map(|sample| sample.direction),
        probe.samples.first().map(|sample| sample.values),
        probe.samples.first().map(|sample| sample.flags),
    );
    Ok(())
}
