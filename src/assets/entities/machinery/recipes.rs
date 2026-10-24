//! Archive-backed physical recipes for constrained ambient assemblies.
use super::{constraints, material_density, ConstraintSettings};
use crate::{
    archive::Catalog,
    entities::{self, Document, PlacedModel},
    physworld::{self, mass::PrincipalMassProperties},
};
use std::collections::{HashMap, HashSet};
pub struct BodyRecipe {
    pub entity_id: String,
    pub placement: usize,
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub mass: Option<PrincipalMassProperties>,
    pub angular_damping: f32,
    pub wind: Option<BodyWind>,
}

/// A body registered with the wind manager (positive `m_windSpeedFactor`):
/// its factor and its shape's aero faces (14008be50 -> 14008b830).
pub struct BodyWind {
    pub speed_factor: f32,
    pub faces: Vec<crate::gameplay::aerodynamics::AeroFace>,
    /// Body-space convex hulls, for trigger-wind overlap.
    pub hulls: Vec<BodyHull>,
}

#[derive(Clone)]
pub struct BodyHull {
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub edges: Vec<[f32; 3]>,
}
pub struct AssemblyRecipe {
    pub bodies: Vec<BodyRecipe>,
    pub joints: Vec<ConstraintSettings>,
}
pub struct MachineryRecipes {
    pub windmills: Vec<super::WindmillSettings>,
    pub assemblies: Vec<AssemblyRecipe>,
    pub unsupported: Vec<String>,
}

pub fn load(
    catalog: &Catalog,
    document: &Document,
    placements: &[PlacedModel],
) -> Result<MachineryRecipes, String> {
    let read = |path: &str| {
        catalog
            .read(path)
            .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
            .map_err(|e| e.to_string())
    };
    let mut constraints = constraints(document, read)?;
    let entity_ids: HashSet<_> = document
        .definitions
        .iter()
        .filter(|d| d.kind == "entity")
        .map(|d| d.id.as_str())
        .collect();
    // 140712da0 initializes each body ID to world(0); an unresolved entity
    // reference leaves it there. Installed vista rotors retain deleted tops.
    for joint in &mut constraints {
        for endpoint in [&mut joint.entity_a, &mut joint.entity_b] {
            if endpoint
                .as_ref()
                .is_some_and(|id| !entity_ids.contains(id.as_str()))
            {
                *endpoint = None;
            }
        }
    }
    let settings = entities::rigid_body_settings(document);
    let bodies: HashMap<_, _> = settings.iter().map(|b| (b.entity_id.as_str(), b)).collect();
    let models: HashMap<_, _> = placements
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), (i, p)))
        .collect();
    let dynamic = |id: &str| {
        bodies
            .get(id)
            .is_some_and(|b| b.physics_behavior.as_deref() == Some("BEHAVIOR_DYNAMIC"))
    };
    let mut visited = HashSet::new();
    let mut result = MachineryRecipes {
        windmills: super::windmills(document, read)?,
        assemblies: Vec::new(),
        unsupported: Vec::new(),
    };
    // Assemblies a force drives: motors, or a dynamic body that the wind
    // manager pushes (809d80 registers positive m_windSpeedFactor).
    let windy = |id: &str| {
        dynamic(id) && bodies.get(id).is_some_and(|b| b.wind_speed_factor.unwrap_or(0.) > 0.)
    };
    for (seed, joint) in constraints.iter().enumerate().filter(|(_, j)| {
        j.started
            && (j.motor.is_some()
                || [j.entity_a.as_deref(), j.entity_b.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(windy))
    }) {
        if visited.contains(&seed) {
            continue;
        }
        let mut selected = HashSet::from([seed]);
        let mut ids: HashSet<&str> = [joint.entity_a.as_deref(), joint.entity_b.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        loop {
            let previous = selected.len();
            for (index, j) in constraints.iter().enumerate() {
                if [j.entity_a.as_deref(), j.entity_b.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|id| ids.contains(id) && dynamic(id))
                {
                    selected.insert(index);
                    ids.extend(
                        [j.entity_a.as_deref(), j.entity_b.as_deref()]
                            .into_iter()
                            .flatten(),
                    );
                }
            }
            if previous == selected.len() {
                break;
            }
        }
        visited.extend(selected.iter().copied());
        let mut selected: Vec<_> = selected.into_iter().collect();
        selected.sort_unstable();
        let assemble = || -> Result<AssemblyRecipe, String> {
            let mut joints = Vec::new();
            for index in &selected {
                let j = &constraints[*index];
                if j.joint_a.is_some() || j.joint_b.is_some() {
                    return Err(format!(
                        "{}: articulated joint endpoints are not supported",
                        j.constraint_id
                    ));
                }
                if j.linear_constraint != "LINEAR_CONSTRAINT_FIXED"
                    || !matches!(
                        j.angular_constraint.as_str(),
                        "ANGULAR_CONSTRAINT_HINGE" | "ANGULAR_CONSTRAINT_LIMITED_HINGE"
                    )
                {
                    return Err(format!(
                        "{}: unsupported constraint {}/{}",
                        j.constraint_id, j.linear_constraint, j.angular_constraint
                    ));
                }
                joints.push(j.clone());
            }
            let mut ids: Vec<_> = ids.iter().copied().collect();
            ids.sort_unstable();
            let mut recipes = Vec::new();
            for id in ids {
                let body = bodies
                    .get(id)
                    .ok_or_else(|| format!("{id}: missing body declaration"))?;
                let &(index, p) = models
                    .get(id)
                    .ok_or_else(|| format!("{id}: missing render placement"))?;
                let basis = physworld::rigid_body_basis(p.rotation.map(|v| [v[0], v[1], v[2], 0.]));
                let orientation = physworld::rotation_quaternion(basis);
                let mut wind = None;
                let mass = match body.physics_behavior.as_deref() {
                    Some("BEHAVIOR_STATIC") => None,
                    Some("BEHAVIOR_DYNAMIC") => {
                        let source = p
                            .resource_path
                            .strip_prefix("generated/model/")
                            .and_then(|s| s.strip_suffix(".bmodel"))
                            .ok_or_else(|| {
                                format!("{id}: unsupported model {}", p.resource_path)
                            })?;
                        let source = physworld::replace_shape_extension(source, "lwo");
                        let kind = body
                            .shape_type
                            .as_deref()
                            .and_then(entities::rigid_body_shape_kind)
                            .ok_or_else(|| format!("{id}: unknown shape"))?;
                        let key = physworld::physics_shape_key(
                            &source,
                            physworld::physics_shape_kind(
                                ["hkt", "cvx", "msh", "box", "cvd"][kind as usize],
                            ),
                            p.scale,
                            0.01,
                            false,
                        )
                        .ok_or_else(|| format!("{id}: invalid shape scale"))?;
                        let path = format!("generated/{key}.massProps");
                        let bytes = catalog
                            .read(&path)
                            .map_err(|e| e.to_string())?
                            .ok_or_else(|| format!("{id}: missing {path}"))?;
                        let mut mass = physworld::mass::parse_principal(&bytes)?;
                        let target = if let Some(m) = body.descriptor_mass_override {
                            m
                        } else {
                            let name = body.material.as_deref().ok_or_else(|| {
                                format!("{id}: unresolved model physics material")
                            })?;
                            mass.volume * material_density(name, read)?
                        };
                        mass.set_mass(target);
                        // 140808ea0: a locked axis (decl +0xc4 bits 3..5)
                        // replaces the major-axis space with identity and
                        // zeroes that inertia lane; set-zero reciprocals then
                        // give it no inverse inertia.
                        if body.lock_rotation.into_iter().any(|v| v) {
                            mass.orientation = [0., 0., 0., 1.];
                            for (axis, locked) in body.lock_rotation.into_iter().enumerate() {
                                if locked {
                                    mass.inertia[axis] = 0.;
                                }
                            }
                        }
                        if let Some(speed_factor) =
                            body.wind_speed_factor.filter(|factor| *factor > 0.)
                        {
                            let path = physworld::physics_shape_asset_path(&key);
                            let bytes = catalog
                                .read(&path)
                                .map_err(|e| e.to_string())?
                                .ok_or_else(|| format!("{id}: missing {path}"))?;
                            let geometry = physworld::parse_hkx(&bytes)?.standalone_shapes(&bytes)?;
                            if !geometry.unknown_shapes.is_empty()
                                || !geometry.compressed_decode_failures.is_empty()
                            {
                                return Err(format!(
                                    "{id}: unsupported wind shape geometry {:?}",
                                    geometry.unknown_shapes
                                ));
                            }
                            let faces = crate::gameplay::aerodynamics::aero_faces(
                                geometry.triangles.iter().map(|t| t.vertices),
                            );
                            let sub = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] {
                                std::array::from_fn(|i| b[i] - a[i])
                            };
                            let hulls = geometry
                                .convex
                                .iter()
                                .map(|hull| {
                                    let triangles: Vec<_> = geometry
                                        .triangles
                                        .iter()
                                        .filter(|t| t.source_shape_item == hull.source_shape_item)
                                        .map(|t| t.vertices)
                                        .collect();
                                    BodyHull {
                                        vertices: hull.vertices.clone(),
                                        normals: crate::gameplay::aerodynamics::aero_faces(
                                            triangles.iter().copied(),
                                        )
                                        .iter()
                                        .map(|f| [f.normal[0], f.normal[1], f.normal[2]])
                                        .collect(),
                                        edges: triangles
                                            .iter()
                                            .flat_map(|[a, b, c]| {
                                                [sub(*a, *b), sub(*a, *c), sub(*b, *c)]
                                            })
                                            .collect(),
                                    }
                                })
                                .collect();
                            wind = Some(BodyWind {
                                speed_factor,
                                faces,
                                hulls,
                            });
                        }
                        Some(mass)
                    }
                    other => return Err(format!("{id}: unsupported behavior {other:?}")),
                };
                recipes.push(BodyRecipe {
                    entity_id: id.to_owned(),
                    placement: index,
                    position: p.position,
                    orientation,
                    mass,
                    angular_damping: body
                        .angular_damping
                        .ok_or_else(|| format!("{id}: invalid angular damping"))?,
                    wind,
                });
            }
            Ok(AssemblyRecipe {
                bodies: recipes,
                joints,
            })
        };
        match assemble() {
            Ok(assembly) => result.assemblies.push(assembly),
            Err(error) => result.unsupported.push(error),
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "audit; requires installed level archives"]
    fn installed_wind_body_audit() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        for path in catalog.paths_with_suffix(".decl") {
            if !path.contains("cpntrigidbody") {
                continue;
            }
            let text = entities::declaration_text(&catalog.read(&path).unwrap().unwrap()).into_owned();
            if text.contains("m_windSpeedFactor") {
                println!("DECL {path}: {}", text.lines().find(|l| l.contains("m_windSpeedFactor")).unwrap().trim());
            }
        }
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let document =
                entities::parse(&entities::declaration_text(&bytes)).unwrap();
            let read = |p: &str| {
                catalog
                    .read(p)
                    .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
                    .map_err(|e| e.to_string())
            };
            let joints = super::super::constraints(&document, read).unwrap();
            let bodies = entities::rigid_body_settings(&document);
            let mut kinds = std::collections::BTreeMap::<String, usize>::new();
            for body in bodies.iter().filter(|b| b.wind_speed_factor.unwrap_or(0.) > 0.) {
                let linked: Vec<_> = joints
                    .iter()
                    .filter(|j| {
                        [j.entity_a.as_deref(), j.entity_b.as_deref()].contains(&Some(&*body.entity_id))
                    })
                    .map(|j| {
                        format!(
                            "{}/{}/{}/started={}/motor={}",
                            j.linear_constraint,
                            j.angular_constraint,
                            [j.entity_a.is_some(), j.entity_b.is_some()]
                                .map(|b| if b { "E" } else { "W" })
                                .concat(),
                            j.started,
                            j.motor.is_some()
                        )
                    })
                    .collect();
                *kinds
                    .entry(format!(
                        "{:?} f={:?} {:?}",
                        body.physics_behavior,
                        body.wind_speed_factor,
                        linked
                    ))
                    .or_default() += 1;
            }
            if !kinds.is_empty() {
                println!("{path}");
                for (kind, count) in kinds {
                    println!("  {count:4} {kind}");
                }
            }
        }
    }

    use super::*;
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_level_machinery_recipe_audit() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let mut total = 0;
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let document = entities::parse(&entities::declaration_text(&bytes)).unwrap();
            let mut placements = entities::render_placements(&document);
            placements.extend(
                entities::placed_models_with_inheritance(&document, |p| {
                    catalog
                        .read(p)
                        .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
                        .map_err(|e| e.to_string())
                })
                .unwrap(),
            );
            let result = load(&catalog, &document, &placements).unwrap();
            let mut world = crate::gameplay::machinery_world::MachineryWorld::from_recipes(&result)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            for _ in 0..10 {
                world.step([1. / 60.; 5]);
            }
            assert!(
                world.bodies.iter().all(|b| b
                    .motion
                    .pose
                    .center
                    .into_iter()
                    .chain(b.motion.pose.orientation)
                    .all(f32::is_finite)),
                "{path}: nonfinite machinery pose"
            );
            println!(
                "{path}: {} assemblies; unsupported {:?}",
                result.assemblies.len(),
                result.unsupported
            );
            total += result.assemblies.len();
        }
        assert!(total > 100);
    }
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_machinery_assemblies_resolve() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let bytes = catalog
            .read("maps/campaign/dust/dust_p_highchaos.entities")
            .unwrap()
            .unwrap();
        let document = entities::parse(&entities::declaration_text(&bytes)).unwrap();
        let mut placements = entities::render_placements(&document);
        placements.extend(
            entities::placed_models_with_inheritance(&document, |p| {
                catalog
                    .read(p)
                    .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
                    .map_err(|e| e.to_string())
            })
            .unwrap(),
        );
        let loaded = load(&catalog, &document, &placements).unwrap();
        println!(
            "Assemblies {}, joints {}, bodies {}, unsupported {:?}",
            loaded.assemblies.len(),
            loaded
                .assemblies
                .iter()
                .map(|a| a.joints.len())
                .sum::<usize>(),
            loaded
                .assemblies
                .iter()
                .map(|a| a.bodies.len())
                .sum::<usize>(),
            loaded.unsupported
        );
        assert!(loaded.assemblies.len() >= 35);
        assert!(loaded.unsupported.is_empty());
    }
}
