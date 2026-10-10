//! Inspect map-local pickup references in an extracted level definition.
#[path = "../archive.rs"]
mod archive;
#[path = "../bmodel.rs"]
mod bmodel;
#[path = "../entities.rs"]
mod entities;
#[path = "../runtime.rs"]
mod runtime;
#[path = "../tome.rs"]
mod tome;

use std::{collections::BTreeMap, env, fs, path::Path};

fn read_environment_lighting(
    catalog: &archive::Catalog,
    environment: &str,
) -> Result<Option<entities::EnvironmentLighting>, Box<dyn std::error::Error>> {
    let resource = format!("generated/decls/env/{environment}.decl");
    let Some(bytes) = catalog.read(&resource)? else {
        return Ok(None);
    };
    let source = String::from_utf8_lossy(&bytes);
    let lighting = entities::environment_lighting_with_inheritance(&source, |parent| {
        let parent_resource = format!("generated/decls/env/{parent}.decl");
        catalog
            .read(&parent_resource)
            .map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
            .map_err(|error| error.to_string())
    })
    .map_err(std::io::Error::other)?;
    Ok(Some(lighting))
}

fn point_inside_triangle_mesh(point: [f32; 3], triangles: &[[[f32; 3]; 3]]) -> bool {
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    let directions = [
        [1.0, 0.371, 0.217],
        [0.213, 1.0, 0.517],
        [0.413, 0.239, 1.0],
    ];
    let inside_votes = directions
        .iter()
        .filter(|&&direction| {
            let intersections = triangles
                .iter()
                .filter(|triangle| {
                    let edge1 = sub(triangle[1], triangle[0]);
                    let edge2 = sub(triangle[2], triangle[0]);
                    let h = cross(direction, edge2);
                    let determinant = dot(edge1, h);
                    if determinant.abs() < 1e-7 {
                        return false;
                    }
                    let inverse = determinant.recip();
                    let from_vertex = sub(point, triangle[0]);
                    let u = dot(from_vertex, h) * inverse;
                    if !(-1e-6..=1.000001).contains(&u) {
                        return false;
                    }
                    let q = cross(from_vertex, edge1);
                    let v = dot(direction, q) * inverse;
                    if v < -1e-6 || u + v > 1.000001 {
                        return false;
                    }
                    dot(edge2, q) * inverse > 1e-5
                })
                .count();
            intersections % 2 == 1
        })
        .count();
    inside_votes >= 2
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("Usage: entity-audit <level.entities>")?;
    if path == "--light-bindings" {
        let level = env::args()
            .nth(2)
            .ok_or("Usage: entity-audit --light-bindings <level.entities> <level.tome>")?;
        let visibility = env::args().nth(3).ok_or("missing level.tome")?;
        let source = fs::read_to_string(level)?;
        let document = entities::parse(&source)?;
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let lights = entities::placed_lights_with_inheritance(&document, |resource| {
            Ok(catalog
                .read(resource)
                .map_err(|error| error.to_string())?
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
        })?;
        let bytes = fs::read(visibility)?;
        let count = u32::from_le_bytes(
            bytes
                .get(0x40..0x44)
                .ok_or("truncated Tome header")?
                .try_into()?,
        );
        let mut ids = Vec::new();
        for index in 0..count {
            ids.extend(tome::object_ids_for_index(&bytes, index)?);
        }
        ids.sort_unstable();
        let mut totals = [0usize; 2];
        let mut matches = [0usize; 2];
        for light in &lights {
            let class = usize::from(light.culling == entities::LightCulling::DynamicPosition);
            totals[class] += 1;
            matches[class] += usize::from(
                tome::sorted_authored_id_index(&ids, light.definition_name_hash).is_some(),
            );
        }
        println!("LIGHT_BINDINGS source_class[static,dynamic]={totals:?} parsed_name_hash_in_all_tome_ids={matches:?} binding_unverified=true");
        for light in lights
            .iter()
            .filter(|light| {
                tome::sorted_authored_id_index(&ids, light.definition_name_hash).is_none()
            })
            .take(8)
        {
            println!(
                "  unmatched parsed name {:?}: hash={} class={:?}",
                light.id, light.definition_name_hash, light.culling
            );
        }
        return Ok(());
    }
    if path == "--probe-components" {
        let resource = env::args()
            .nth(2)
            .ok_or("Usage: entity-audit --probe-components <level.entities resource>")?;
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let bytes = catalog
            .read(&resource)?
            .ok_or("entity resource not found")?;
        let document = entities::parse(&String::from_utf8_lossy(&bytes))?;
        for definition in document
            .definitions
            .iter()
            .filter(|item| item.class.to_ascii_lowercase().contains("probe"))
        {
            println!("{} {} {}", definition.kind, definition.class, definition.id);
        }
        return Ok(());
    }
    if path == "--probe-versions" {
        let fragment = env::args().nth(2).unwrap_or_default();
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let mut versions = BTreeMap::<(u8, Option<u32>, Option<u32>), usize>::new();
        let mut invalid = 0usize;
        let mut total = 0usize;
        for resource in catalog
            .paths_with_suffix(".bprobe")
            .into_iter()
            .filter(|resource| fragment.is_empty() || resource.contains(&fragment))
        {
            let Some(bytes) = catalog.read(&resource)? else {
                continue;
            };
            total += 1;
            if bytes.get(0..3) == Some(b"PRB") {
                let marker = bytes[3];
                // FUN_1402b99a0 treats byte 3 as a legacy marker. For marker
                // >= 11 it then reads the actual resource version and
                // generation from the next two little-endian u32 fields.
                let (version, generation) = if marker >= 11 && bytes.len() >= 12 {
                    (
                        Some(u32::from_le_bytes(bytes[4..8].try_into().unwrap())),
                        Some(u32::from_le_bytes(bytes[8..12].try_into().unwrap())),
                    )
                } else {
                    (None, None)
                };
                *versions.entry((marker, version, generation)).or_default() += 1;
            } else {
                invalid += 1;
            }
        }
        println!(".bprobe resources matching {fragment:?}: {total}; invalid magic: {invalid}");
        for ((marker, version, generation), count) in versions {
            println!("  PRB marker {marker}; loader version {version:?}; generation {generation:?}: {count}");
        }
        return Ok(());
    }
    if path == "--light-counts" {
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let mut largest_area_set = (String::new(), 0usize, 0usize);
        for resource in catalog
            .paths_with_suffix(".entities")
            .into_iter()
            .filter(|resource| resource.starts_with("maps/"))
        {
            let Some(bytes) = catalog.read(&resource)? else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            let document = entities::parse(&text)?;
            let lights = entities::placed_lights_with_inheritance(&document, |decl| {
                let declaration = format!("generated/decls/entitydef/{decl}.decl");
                catalog
                    .read(&declaration)
                    .map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
                    .map_err(|error| error.to_string())
            })?;
            let area_count = lights
                .iter()
                .filter(|light| light.kind == entities::LightType::Area)
                .count();
            println!(
                "{resource}: {} authored lights, {area_count} area",
                lights.len()
            );
            if area_count > largest_area_set.1 {
                largest_area_set = (resource, area_count, lights.len());
            }
        }
        println!(
            "Largest area-light set: {} areas in {} ({} total lights); renderer capacity is 128",
            largest_area_set.1, largest_area_set.0, largest_area_set.2
        );
        return Ok(());
    }
    if path == "--resources" {
        let fragment = env::args().nth(2).unwrap_or_default();
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let paths = if fragment.is_empty() {
            catalog.paths_with_suffix(".decl")
        } else {
            catalog.paths_containing(&fragment)
        };
        for resource in &paths {
            println!("{resource}");
        }
        eprintln!("{} matching resources", paths.len());
        return Ok(());
    }
    if path == "--environment-lighting" {
        let environment = env::args()
            .nth(2)
            .ok_or("Usage: entity-audit --environment-lighting <environment-name>")?;
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let lighting = read_environment_lighting(&catalog, &environment)?
            .ok_or_else(|| format!("environment declaration not found: {environment}"))?;
        println!(
            "Resolved {environment}: ambient high={:?}, low={:?}; scalar high/low={:?}/{:?}; indirect ambient={:?}; bounce static/sun/horizon/zenith/dynamic={:?}/{:?}/{:?}/{:?}/{:?}; sun color/theta/phi={:?}/{:?}/{:?}; exposure EV={:?}",
            lighting.ambient_high,
            lighting.ambient_low,
            lighting.ambient_high_scalar,
            lighting.ambient_low_scalar,
            lighting.indirect_ambient,
            lighting.bounce_static,
            lighting.bounce_sun,
            lighting.bounce_sky_horizon,
            lighting.bounce_sky_zenith,
            lighting.bounce_dynamic,
            lighting.sun_color,
            lighting.sun_theta_degrees,
            lighting.sun_phi_degrees,
            lighting.auto_exposure_ev,
        );
        return Ok(());
    }
    if path == "--lights-near" {
        let level = env::args()
            .nth(2)
            .ok_or("Usage: entity-audit --lights-near <level.entities> <x> <y> <z> [radius]")?;
        let coordinates = env::args()
            .skip(3)
            .take(3)
            .map(|value| value.parse::<f32>())
            .collect::<Result<Vec<_>, _>>()?;
        if coordinates.len() != 3 {
            return Err("--lights-near requires x y z coordinates".into());
        }
        let radius = env::args()
            .nth(6)
            .map(|value| value.parse::<f32>())
            .transpose()?
            .unwrap_or(40.0);
        let bytes = fs::read(&level)?;
        let document = entities::parse(&String::from_utf8_lossy(&bytes))?;
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let lights = entities::placed_lights_with_inheritance(&document, |decl| {
            catalog
                .read(decl)
                .map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
                .map_err(|error| error.to_string())
        })?;
        let mut nearby = lights
            .iter()
            .filter_map(|light| {
                let distance = (0..3)
                    .map(|axis| (light.position[axis] - coordinates[axis]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                (distance <= radius).then_some((distance, light))
            })
            .collect::<Vec<_>>();
        nearby.sort_by(|a, b| a.0.total_cmp(&b.0));
        let in_authored_range = nearby
            .iter()
            .filter(|(distance, light)| {
                light.visible
                    && matches!(
                        light.kind,
                        entities::LightType::Point
                            | entities::LightType::HalfPoint
                            | entities::LightType::Spot
                            | entities::LightType::Area
                    )
                    && light.distance_cutoff > *distance
            })
            .count();
        let in_spot_cone = nearby
            .iter()
            .filter(|(distance, light)| {
                if !light.visible
                    || light.kind != entities::LightType::Spot
                    || light.angular_cutoff <= 0.0
                    || light.distance_cutoff <= *distance
                {
                    return false;
                }
                let toward_query = std::array::from_fn::<_, 3, _>(|axis| {
                    (coordinates[axis] - light.position[axis]) / distance.max(0.0001)
                });
                let aim_dot = (0..3)
                    .map(|axis| light.rotation[0][axis] * toward_query[axis])
                    .sum::<f32>()
                    .clamp(-1.0, 1.0);
                aim_dot >= (light.angular_cutoff * 0.5).to_radians().cos()
            })
            .count();
        let in_halfpoint_cone = nearby
            .iter()
            .filter(|(distance, light)| {
                if !light.visible
                    || light.kind != entities::LightType::HalfPoint
                    || light.distance_cutoff <= *distance
                {
                    return false;
                }
                if light.angular_cutoff <= 0.0 {
                    return true;
                }
                let toward_query = std::array::from_fn::<_, 3, _>(|axis| {
                    (coordinates[axis] - light.position[axis]) / distance.max(0.0001)
                });
                let aim_dot = (0..3)
                    .map(|axis| light.rotation[0][axis] * toward_query[axis])
                    .sum::<f32>()
                    .clamp(-1.0, 1.0);
                aim_dot >= (light.angular_cutoff * 0.5).to_radians().cos()
            })
            .count();
        println!(
            "{} authored lights within {radius:.1} m of {:?}; {in_authored_range} visible point/half-point/spot/area sources inside authored cutoff; {in_spot_cone} spots and {in_halfpoint_cone} half-points inside the viewer-mapped cone",
            nearby.len(), coordinates,
        );
        for (distance, light) in nearby.iter().take(40) {
            let toward_query = std::array::from_fn::<_, 3, _>(|axis| {
                (coordinates[axis] - light.position[axis]) / distance.max(0.0001)
            });
            let aim_dot = (0..3)
                .map(|axis| light.rotation[0][axis] * toward_query[axis])
                .sum::<f32>()
                .clamp(-1.0, 1.0);
            let cone_candidate = matches!(
                light.kind,
                entities::LightType::Spot | entities::LightType::HalfPoint
            ) && light.angular_cutoff > 0.0
                && aim_dot >= (light.angular_cutoff * 0.5).to_radians().cos();
            println!(
                "{distance:6.2} m | {:?} | {} | visible={} | cutoff/falloff={:.2}/{:.2} | angles={:.1}/{:.1} | axis0={:?} authored={} | mapped-cone-candidate={} | {:?}",
                light.kind,
                light.id,
                light.visible,
                light.distance_cutoff,
                light.distance_falloff,
                light.angular_cutoff,
                light.angular_falloff,
                light.rotation[0],
                light.orientation_authored,
                cone_candidate,
                light.preset
            );
        }
        return Ok(());
    }
    if path == "--volumes-near" {
        let level = env::args()
            .nth(2)
            .ok_or("Usage: entity-audit --volumes-near <level.entities> <x> <y> <z> [radius]")?;
        let coordinates = env::args()
            .skip(3)
            .take(3)
            .map(|value| value.parse::<f32>())
            .collect::<Result<Vec<_>, _>>()?;
        if coordinates.len() != 3 {
            return Err("--volumes-near requires x y z coordinates".into());
        }
        let radius = env::args()
            .nth(6)
            .map(|value| value.parse::<f32>())
            .transpose()?
            .unwrap_or(100.0);
        let bytes = fs::read(&level)?;
        let document = entities::parse(&String::from_utf8_lossy(&bytes))?;
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let mut nearby = entities::placed_environment_volumes(&document)
            .into_iter()
            .filter_map(|volume| {
                let distance = (0..3)
                    .map(|axis| (volume.position[axis] - coordinates[axis]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                (distance <= radius).then_some((distance, volume))
            })
            .collect::<Vec<_>>();
        nearby.sort_by(|a, b| a.0.total_cmp(&b.0));
        println!(
            "{} authored arkAreaVolume centers within {radius:.1} m of {:?}",
            nearby.len(),
            coordinates
        );
        for (distance, volume) in nearby.iter().take(40) {
            let mut contains_aabb = None;
            let mut contains_mesh = None;
            let mut bounds = None;
            if let Some(model_path) = volume.model.as_deref() {
                let resource = format!("{model_path}.bmodel");
                if let Some(bytes) = catalog.read(&resource)? {
                    match bmodel::parse(&bytes) {
                        Ok(model) => {
                            let mut low = [f32::INFINITY; 3];
                            let mut high = [f32::NEG_INFINITY; 3];
                            let mut triangles = Vec::new();
                            for surface in &model.surfaces {
                                let points: Vec<[f32; 3]> = surface
                                    .positions
                                    .iter()
                                    .map(|vertex| {
                                        std::array::from_fn(|row| {
                                            volume.position[row]
                                                + (0..3)
                                                    .map(|axis| {
                                                        volume.rotation[axis][row] * vertex[axis]
                                                    })
                                                    .sum::<f32>()
                                        })
                                    })
                                    .collect();
                                for &point in &points {
                                    for axis in 0..3 {
                                        low[axis] = low[axis].min(point[axis]);
                                        high[axis] = high[axis].max(point[axis]);
                                    }
                                }
                                triangles.extend(surface.indices.chunks_exact(3).map(|face| {
                                    [
                                        points[face[0] as usize],
                                        points[face[1] as usize],
                                        points[face[2] as usize],
                                    ]
                                }));
                            }
                            if low.iter().all(|value| value.is_finite()) {
                                contains_aabb = Some((0..3).all(|axis| {
                                    coordinates[axis] >= low[axis]
                                        && coordinates[axis] <= high[axis]
                                }));
                                contains_mesh = Some(point_inside_triangle_mesh(
                                    [coordinates[0], coordinates[1], coordinates[2]],
                                    &triangles,
                                ));
                                bounds = Some((low, high));
                            }
                        }
                        Err(error) => {
                            let header = bytes
                                .iter()
                                .take(128)
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ");
                            eprintln!(
                                "{resource}: {error}; {} bytes; header={header}",
                                bytes.len()
                            );
                        }
                    }
                } else {
                    eprintln!("missing render model resource {resource}");
                }
            }
            println!(
                "{distance:6.2} m | {} | render-model AABB contains point={contains_aabb:?}, mesh ray candidate={contains_mesh:?}, bounds={bounds:?} | model={:?} | ambient RGB high/low={:?}/{:?}; scalar high/low={:?}/{:?} | sun={:?}",
                volume.id,
                volume.model,
                volume.lighting_override.ambient_high,
                volume.lighting_override.ambient_low,
                volume.lighting_override.ambient_high_scalar,
                volume.lighting_override.ambient_low_scalar,
                volume.lighting_override.sun_color
            );
        }
        return Ok(());
    }
    if path == "--resource" {
        let resource = env::args()
            .nth(2)
            .ok_or("Usage: entity-audit --resource <archive-path>")?;
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let bytes = catalog
            .read(&resource)?
            .ok_or_else(|| format!("resource not found: {resource}"))?;
        print!("{}", String::from_utf8_lossy(&bytes));
        return Ok(());
    }
    if path == "--sun-angle-audit" {
        let base = env::var("DH2_GAME_BASE")
            .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
        let catalog = archive::Catalog::open(Path::new(&base))?;
        let mut compared = 0usize;
        for resource in catalog
            .paths_with_suffix(".entities")
            .into_iter()
            .filter(|resource| resource.contains("maps/campaign/"))
        {
            let Some(bytes) = catalog.read(&resource)? else {
                continue;
            };
            let document = entities::parse(&String::from_utf8_lossy(&bytes))?;
            let Some(environment) = entities::world_environment(&document) else {
                continue;
            };
            let environment_path = format!("generated/decls/env/{environment}.decl");
            let Some(environment_bytes) = catalog.read(&environment_path)? else {
                continue;
            };
            let lighting =
                entities::environment_lighting(&String::from_utf8_lossy(&environment_bytes));
            let Some(phi) = lighting.sun_phi_degrees else {
                continue;
            };
            let lights = entities::placed_lights_with_inheritance(&document, |decl| {
                catalog
                    .read(decl)
                    .map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
                    .map_err(|error| error.to_string())
            })?;
            for light in lights.iter().filter(|light| {
                light.kind == entities::LightType::Parallel
                    && light.preset.as_deref() == Some("LIGHT_PRESET_SUN")
            }) {
                let axis = light.rotation[0];
                let azimuth = (-axis[1]).atan2(axis[0]).to_degrees().rem_euclid(360.0);
                let difference = (azimuth - phi + 180.0).rem_euclid(360.0) - 180.0;
                println!(
                    "{resource}: environment phi={phi:.3}, SUN-light axis azimuth={azimuth:.3}, delta={difference:.3}, orientation_authored={} ",
                    light.orientation_authored
                );
                compared += 1;
            }
        }
        println!("Compared {compared} campaign SUN parallel lights against environment phi.");
        return Ok(());
    }
    let bytes = fs::read(&path)?;
    let text = String::from_utf8_lossy(&bytes);
    let document = entities::parse(&text)?;
    let start = entities::player_start(&document).ok_or("No player start in level")?;
    let base = env::var("DH2_GAME_BASE")
        .unwrap_or_else(|_| r"D:\Programs\Steam\steamapps\common\Dishonored2\base".into());
    let catalog = archive::Catalog::open(std::path::Path::new(&base))?;
    let direct_models = entities::placed_models(&document);
    let resolved_models = entities::placed_models_with_inheritance(&document, |path| {
        catalog
            .read(path)
            .map(|resource| resource.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
            .map_err(|error| error.to_string())
    })?;
    println!(
        "Explicit model placements: {}; with inherited entityDefs: {}",
        direct_models.len(),
        resolved_models.len()
    );
    let lights = entities::placed_lights_with_inheritance(&document, |path| {
        catalog
            .read(path)
            .map(|resource| resource.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
            .map_err(|error| error.to_string())
    })?;
    if let Some(environment) = entities::world_environment(&document) {
        let resource = format!("generated/decls/env/{environment}.decl");
        if let Some(bytes) = catalog.read(&resource)? {
            let lighting = entities::environment_lighting_with_inheritance(
                &String::from_utf8_lossy(&bytes),
                |parent| {
                    let parent_resource = format!("generated/decls/env/{parent}.decl");
                    catalog
                        .read(&parent_resource)
                        .map(|bytes| {
                            bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                        })
                        .map_err(|error| error.to_string())
                },
            )
            .map_err(std::io::Error::other)?;
            println!(
                "World environment: {resource}; ambient high={:?}, low={:?}; indirect ambient={:?}; bounce static={:?}, sun={:?}, horizon={:?}, zenith={:?}, dynamic={:?}; parallel-light extent={:?}; sun color={:?}, theta={:?}, phi={:?}; auto-exposure EV={:?}, luminance min/max={:?}/{:?}, adapt up/down={:?}/{:?}",
                lighting.ambient_high,
                lighting.ambient_low,
                lighting.indirect_ambient,
                lighting.bounce_static,
                lighting.bounce_sun,
                lighting.bounce_sky_horizon,
                lighting.bounce_sky_zenith,
                lighting.bounce_dynamic,
                lighting.parallel_light_extend,
                lighting.sun_color,
                lighting.sun_theta_degrees,
                lighting.sun_phi_degrees,
                lighting.auto_exposure_ev,
                lighting.auto_exposure_min_luminance,
                lighting.auto_exposure_max_luminance,
                lighting.auto_exposure_adapt_speed_up,
                lighting.auto_exposure_adapt_speed_down,
            );
        }
    }
    println!(
        "Resolved lights: {} point, {} half-point, {} spot, {} area, {} parallel",
        lights
            .iter()
            .filter(|light| light.kind == entities::LightType::Point)
            .count(),
        lights
            .iter()
            .filter(|light| light.kind == entities::LightType::HalfPoint)
            .count(),
        lights
            .iter()
            .filter(|light| light.kind == entities::LightType::Spot)
            .count(),
        lights
            .iter()
            .filter(|light| light.kind == entities::LightType::Area)
            .count(),
        lights
            .iter()
            .filter(|light| light.kind == entities::LightType::Parallel)
            .count(),
    );
    let mut presets = BTreeMap::<String, usize>::new();
    let mut reflector_presets = BTreeMap::<String, usize>::new();
    let mut spot_shadow_qualities = BTreeMap::<String, usize>::new();
    let mut light_decay_values = BTreeMap::<String, usize>::new();
    let mut half_point_angles = BTreeMap::<(i32, i32), usize>::new();
    let mut spot_angles = BTreeMap::<(i32, i32), usize>::new();
    for light in &lights {
        *presets
            .entry(light.preset.clone().unwrap_or_else(|| "<unset>".into()))
            .or_default() += 1;
        *reflector_presets
            .entry(
                light
                    .reflector_preset
                    .clone()
                    .unwrap_or_else(|| "<unset>".into()),
            )
            .or_default() += 1;
        *light_decay_values
            .entry(format!("{:.3}", light.decay))
            .or_default() += 1;
        if light.kind == entities::LightType::Spot && !light.no_shadows {
            *spot_shadow_qualities
                .entry(
                    light
                        .spot_shadow_quality
                        .clone()
                        .unwrap_or_else(|| "<unset>".into()),
                )
                .or_default() += 1;
        }
        if light.kind == entities::LightType::HalfPoint {
            *half_point_angles
                .entry((
                    (light.angular_cutoff * 10.0).round() as i32,
                    (light.angular_falloff * 10.0).round() as i32,
                ))
                .or_default() += 1;
        }
        if light.kind == entities::LightType::Spot {
            *spot_angles
                .entry((
                    (light.angular_cutoff * 10.0).round() as i32,
                    (light.angular_falloff * 10.0).round() as i32,
                ))
                .or_default() += 1;
        }
    }
    println!("Authored preset counts: {presets:?}");
    println!("Effective engine m_decay inputs: {light_decay_values:?}");
    println!("Authored reflector-preset counts: {reflector_presets:?}");
    println!("Shadow-casting spot quality inputs: {spot_shadow_qualities:?}");
    println!(
        "Light back-channel inputs: {} explicit indirect-light boosts ({} above 1x); {} nonzero back-reflection values",
        lights
            .iter()
            .filter(|light| light.indirect_lighting_boost.is_some())
            .count(),
        lights.iter().filter(|light| light.indirect_lighting_boost.is_some_and(|boost| boost.iter().any(|channel| *channel > 1.0))).count(),
        lights
            .iter()
            .filter(|light| light.back_reflection.is_some_and(|value| value != 0.0))
            .count(),
    );
    println!(
        "Half-point authored (cutoff, falloff) angle bins in 0.1 degrees: {half_point_angles:?}"
    );
    println!("Spot authored (cutoff, falloff) angle bins in 0.1 degrees: {spot_angles:?}");
    println!(
        "Direct-light inputs: {} point/spot with positive cutoff; {} spot orientations with two nonparallel axes",
        lights.iter().filter(|light| matches!(light.kind, entities::LightType::Point | entities::LightType::Spot) && light.distance_cutoff.is_finite() && light.distance_cutoff > 0.0).count(),
        lights.iter().filter(|light| light.kind == entities::LightType::Spot && {
            let a = light.rotation[0];
            let b = light.rotation[1];
            let cross = [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];
            cross.iter().map(|x| x*x).sum::<f32>() > 0.0001
        }).count(),
    );
    println!(
        "Shadow flags: {} explicitly disable shadows, {} have no disable flag",
        lights.iter().filter(|light| light.no_shadows).count(),
        lights.iter().filter(|light| !light.no_shadows).count(),
    );
    let area_lights = lights
        .iter()
        .filter(|light| light.kind == entities::LightType::Area)
        .collect::<Vec<_>>();
    println!(
        "Area-light metadata: {} non-unit renderModelInfo sizes, {} textures, {} nonzero diffusion, {} nonzero projection direction, {} nonzero authored shadow controls, {} energy-conservative",
        area_lights
            .iter()
            .filter(|light| light.area_size != [1.0; 2])
            .count(),
        area_lights
            .iter()
            .filter(|light| light.area_texture.as_deref().is_some_and(|path| path != "NULL"))
            .count(),
        area_lights.iter().filter(|light| light.area_diffusion != 0.0).count(),
        area_lights
            .iter()
            .filter(|light| light.area_projection_direction != [0.0; 2])
            .count(),
        area_lights.iter().filter(|light| {
            light.area_shadow_source_offset != [0.0; 2]
                || light.area_shadow_blur != 0.0
                || light.area_shadow_hardening != 0.0
                || light.area_shadow_range != 0.0
        }).count(),
        area_lights
            .iter()
            .filter(|light| light.area_energy_conservative)
            .count(),
    );
    let bindings = entities::component_bindings(&document);
    let bodies = entities::rigid_body_settings(&document);
    let links = entities::constraint_links(&document);
    println!(
        "Physics bindings: {} rigid bodies, {} constraints ({} map-local declarations)",
        bindings
            .iter()
            .filter(|binding| binding.class == "cpntRigidBody")
            .count(),
        bindings
            .iter()
            .filter(|binding| binding.class == "cpntConstraint")
            .count(),
        links.len()
    );
    println!(
        "Constraint endpoints: {} A, {} B, {} without either",
        links
            .iter()
            .filter(|link| link.entity_a_id.is_some())
            .count(),
        links
            .iter()
            .filter(|link| link.entity_b_id.is_some())
            .count(),
        links
            .iter()
            .filter(|link| link.entity_a_id.is_none() && link.entity_b_id.is_none())
            .count()
    );
    println!("Constraint settings: {} joint A, {} joint B, {} linear types, {} angular types, {} explicit collision flags",
        links.iter().filter(|link| link.joint_a_id.is_some()).count(),
        links.iter().filter(|link| link.joint_b_id.is_some()).count(),
        links.iter().filter(|link| link.linear_type.is_some()).count(),
        links.iter().filter(|link| link.angular_type.is_some()).count(),
        links.iter().filter(|link| link.disable_collision.is_some()).count());
    println!("Rigid-body settings: {} HKT shapes, {} mass overrides, {} density overrides, {} rotation locks",
        bodies.iter().filter(|body| body.shape_type.as_deref() == Some("PHYSICS_SHAPE_HKT")).count(),
        bodies.iter().filter(|body| body.mass_override.is_some()).count(),
        bodies.iter().filter(|body| body.density_override.is_some()).count(),
        bodies.iter().filter(|body| body.lock_rotation.iter().any(|&locked| locked)).count());
    let entity_ids: std::collections::HashSet<&str> = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "entity")
        .map(|definition| definition.id.as_str())
        .collect();
    let body_ids: std::collections::HashSet<&str> =
        bodies.iter().map(|body| body.entity_id.as_str()).collect();
    let endpoints: Vec<&str> = links
        .iter()
        .flat_map(|link| [link.entity_a_id.as_deref(), link.entity_b_id.as_deref()])
        .flatten()
        .collect();
    println!(
        "Constraint endpoint resolution: {} total, {} named entities, {} rigid-body entities",
        endpoints.len(),
        endpoints
            .iter()
            .filter(|id| entity_ids.contains(**id))
            .count(),
        endpoints
            .iter()
            .filter(|id| body_ids.contains(**id))
            .count()
    );
    for endpoint in endpoints.iter().filter(|id| !entity_ids.contains(**id)) {
        println!("Unresolved constraint endpoint: {endpoint}");
    }
    let classes: std::collections::HashMap<&str, &str> = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "entity")
        .filter_map(|definition| {
            definition.fields.iter().find_map(|(key, value)| {
                if key == "class" {
                    if let entities::Value::Atom(name) = value {
                        return Some((definition.id.as_str(), name.as_str()));
                    }
                }
                None
            })
        })
        .collect();
    let point = if let Some(x) = env::args().nth(2) {
        [
            x.parse::<f32>()?,
            env::args().nth(3).ok_or("missing y")?.parse::<f32>()?,
            env::args().nth(4).ok_or("missing z")?.parse::<f32>()?,
        ]
    } else {
        start.position
    };
    let body_by_id: std::collections::HashMap<_, _> = bodies
        .iter()
        .map(|body| (body.entity_id.as_str(), body))
        .collect();
    let mut nearby_bodies = Vec::new();
    for definition in document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "entity")
    {
        let Some(body) = body_by_id.get(definition.id.as_str()) else {
            continue;
        };
        let Some(edit) = nested(&definition.fields, "edit") else {
            continue;
        };
        let Some(position) = nested(edit, "spawnPosition") else {
            continue;
        };
        let xyz = ["x", "y", "z"].map(|axis| {
            atom(position, axis)
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.0)
        });
        let distance = distance_squared(xyz, point).sqrt();
        if distance < 35.0 {
            nearby_bodies.push((
                distance,
                definition.id.as_str(),
                xyz,
                body.shape_type.as_deref(),
            ));
        }
    }
    nearby_bodies.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!(
        "Nearest 30 rigid-body entity origins ({} within 35m):",
        nearby_bodies.len()
    );
    for (distance, id, xyz, shape) in nearby_bodies.iter().take(30) {
        println!("{distance:.2} m | {id} | {xyz:?} | {shape:?}");
    }
    let mut nearby = direct_models.clone();
    nearby.sort_by(|a, b| {
        distance_squared(a.position, point).total_cmp(&distance_squared(b.position, point))
    });
    println!("Nearest 20 explicitly modeled entities:");
    for model in nearby.iter().take(20) {
        println!(
            "{:.1} m | {} | {} | {} | {:?} | rotation={:?}",
            distance_squared(model.position, point).sqrt(),
            model.id,
            classes.get(model.id.as_str()).copied().unwrap_or("?"),
            model.resource_path,
            model.position,
            model.rotation
        );
    }
    println!("Nearby constraint targets:");
    for link in &links {
        for target_id in [link.entity_a_id.as_deref(), link.entity_b_id.as_deref()]
            .into_iter()
            .flatten()
        {
            let Some(target) = direct_models.iter().find(|model| model.id == target_id) else {
                continue;
            };
            if distance_squared(target.position, point) <= 10.0 * 10.0 {
                println!(
                    "{} -> {} | {} {:?} {:?}",
                    link.entity_id,
                    target_id,
                    link.declaration_id,
                    link.angular_type,
                    link.cone_angle_degrees
                );
            }
        }
    }
    println!("Nearby non-LWO entity models:");
    let mut other_models = Vec::new();
    for definition in &document.definitions {
        if definition.kind != "entity" {
            continue;
        }
        let Some(edit) = nested(&definition.fields, "edit") else {
            continue;
        };
        let Some(render) = nested(edit, "renderModelInfo") else {
            continue;
        };
        let Some(model) = atom(render, "model") else {
            continue;
        };
        if model.ends_with(".lwo") {
            continue;
        }
        let Some(position) = nested(edit, "spawnPosition") else {
            continue;
        };
        let xyz = ["x", "y", "z"].map(|axis| {
            atom(position, axis)
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.0)
        });
        let distance = distance_squared(xyz, point).sqrt();
        if distance < 10.0 {
            other_models.push((distance, definition.id.as_str(), model));
        }
    }
    other_models.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (distance, id, model) in other_models.into_iter().take(20) {
        let compiled = model
            .strip_suffix(".md6")
            .map(|stem| format!("generated/model/{stem}.bmodel"));
        println!(
            "{distance:.1} m | {id} | {model} | compiled-bmodel={}",
            compiled.as_ref().is_some_and(|path| catalog.contains(path))
        );
    }
    let mut pickups = entities::pickups(&document);
    pickups.sort_by(|a, b| {
        distance_squared(a.position, start.position)
            .total_cmp(&distance_squared(b.position, start.position))
    });
    let mut runtime = runtime::WorldState::from_pickups(&pickups)?;
    println!(
        "Player start {:?}; {} map-local pickup instances",
        start.position,
        pickups.len()
    );
    for pickup in pickups.iter().take(20) {
        println!(
            "{:.1} m | {} | {} x{} | {:?}",
            distance_squared(pickup.position, start.position).sqrt(),
            pickup.entity_id,
            pickup.item_path,
            pickup.quantity,
            pickup.position
        );
    }
    if let Some(pickup) = pickups.first() {
        let event = runtime
            .collect_pickup(&pickup.entity_id)
            .map_err(|error| format!("runtime pickup failed: {error:?}"))?;
        assert!(!runtime.pickup_available(&pickup.entity_id));
        println!("Nearest pickup runtime event: {event:?}");
    }
    let pickup_ids: std::collections::HashSet<&str> = document.definitions.iter()
        .filter(|definition| definition.kind == "entity" && definition.fields.iter().any(|(key, value)| {
            key == "class" && matches!(value, entities::Value::Atom(name) if name.trim_matches('"') == "arkPickup")
        }))
        .map(|definition| definition.id.as_str()).collect();
    let mut visual_pickups: Vec<_> = entities::placed_models(&document)
        .into_iter()
        .filter(|model| pickup_ids.contains(model.id.as_str()))
        .collect();
    visual_pickups.sort_by(|a, b| {
        distance_squared(a.position, start.position)
            .total_cmp(&distance_squared(b.position, start.position))
    });
    println!(
        "{} pickup entities with explicit visual models:",
        visual_pickups.len()
    );
    for model in visual_pickups.iter().take(20) {
        println!(
            "{:.1} m | {} | {} | {:?}",
            distance_squared(model.position, start.position).sqrt(),
            model.id,
            model.resource_path,
            model.position
        );
    }
    Ok(())
}

fn nested<'a>(
    fields: &'a [(String, entities::Value)],
    key: &str,
) -> Option<&'a [(String, entities::Value)]> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| match value {
            entities::Value::Scope(nested) => Some(nested.as_slice()),
            _ => None,
        })
}

fn atom<'a>(fields: &'a [(String, entities::Value)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| match value {
            entities::Value::Atom(text) => Some(text.trim_matches('"')),
            _ => None,
        })
}

fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}
