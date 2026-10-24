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
    /// `m_startAsleep` (default true): 809d80 adds the body deactivated.
    pub start_asleep: bool,
}

/// A body registered with the wind manager (positive `m_windSpeedFactor`):
/// its factor and its shape's aero faces (14008be50 -> 14008b830).
pub struct BodyWind {
    pub speed_factor: f32,
    pub faces: Vec<crate::gameplay::aerodynamics::AeroFace>,
    /// Body-space convex hulls, for trigger-wind distance.
    pub hulls: Vec<BodyHull>,
    /// The cooked shape convex radius (the shape key radius, 0.01).
    pub convex_radius: f32,
}

/// Body-space hull vertices of the cooked shape (convex radius in the key).
#[derive(Clone)]
pub struct BodyHull {
    pub vertices: Vec<[f32; 3]>,
}
pub struct AssemblyRecipe {
    pub bodies: Vec<BodyRecipe>,
    pub joints: Vec<ConstraintSettings>,
}

/// A dynamic arkCpntPhysicsSystem: the MD6's init `ragdoll` physics system,
/// its skeleton and the arkRagdollResource body-to-joint map (14076abc0).
pub struct SystemRecipe {
    pub entity_id: String,
    pub placement: usize,
    /// Entity spawn transform; 14008e560 places each body cinfo pose by it.
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub system: physworld::physics_system::PhysicsSystem,
    pub skeleton: String,
    pub ragdoll: Vec<physworld::physics_system::RagdollEntry>,
}

pub struct MachineryRecipes {
    pub windmills: Vec<super::WindmillSettings>,
    pub assemblies: Vec<AssemblyRecipe>,
    pub systems: Vec<SystemRecipe>,
    pub unsupported: Vec<String>,
}

/// Dynamic physics systems of render-placed MD6 entities.
fn physics_systems(
    catalog: &Catalog,
    document: &Document,
    placements: &[PlacedModel],
    unsupported: &mut Vec<String>,
) -> Vec<SystemRecipe> {
    let behavior = |declaration: &str| {
        document
            .definitions
            .iter()
            .find(|d| d.kind == "component" && d.id == declaration)
            .and_then(|d| d.fields.iter().find(|(k, _)| k == "edit"))
            .and_then(|(_, v)| match v {
                entities::Value::Scope(fields) => fields
                    .iter()
                    .find(|(k, _)| k == "m_PhysicsBehavior")
                    .and_then(|(_, v)| match v {
                        entities::Value::Atom(a) => Some(a.trim_matches('"').to_owned()),
                        _ => None,
                    }),
                _ => None,
            })
    };
    let read = |path: &str| {
        catalog
            .read(path)
            .ok()
            .flatten()
            .map(|bytes| entities::declaration_text(&bytes).into_owned())
    };
    let mut systems = Vec::new();
    for binding in entities::component_bindings(document) {
        if binding.class != "cpntPhysicsSystem" {
            continue;
        }
        if behavior(&binding.declaration_id).as_deref() != Some("BEHAVIOR_DYNAMIC") {
            continue;
        }
        let id = binding.entity_id.as_str();
        let load = || -> Result<Option<SystemRecipe>, String> {
            let Some((index, p)) = placements
                .iter()
                .enumerate()
                .find(|(_, p)| p.id == id && p.resource_path.ends_with(".bmd6model"))
            else {
                return Ok(None);
            };
            let Some(binding) = crate::bmd6::resolve_skeleton_binding(&p.resource_path, read)? else {
                return Err(format!("{id}: MD6 has no skeleton"));
            };
            let ragdoll = binding.ragdoll.ok_or_else(|| format!("{id}: MD6 init has no ragdoll"))?;
            let stem = ragdoll
                .strip_suffix(".hkt")
                .ok_or_else(|| format!("{id}: unsupported ragdoll {ragdoll}"))?;
            let path = format!("generated/physsystem/{stem}.hkx");
            let bytes = catalog
                .read(&path)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("{id}: missing {path}"))?;
            let system = physworld::physics_system::parse(&bytes).map_err(|e| format!("{id}: {e}"))?;
            let map = physworld::physics_system::ragdoll_path(&binding.name, &ragdoll);
            let bytes = catalog
                .read(&map)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("{id}: missing {map}"))?;
            let ragdoll = physworld::physics_system::parse_ragdoll(&bytes)?;
            if ragdoll.len() != system.bodies.len() {
                return Err(format!("{id}: ragdoll map does not cover every body"));
            }
            let basis = physworld::rigid_body_basis(p.rotation.map(|v| [v[0], v[1], v[2], 0.]));
            Ok(Some(SystemRecipe {
                entity_id: id.to_owned(),
                placement: index,
                position: p.position,
                orientation: physworld::rotation_quaternion(basis),
                system,
                skeleton: binding.name,
                ragdoll,
            }))
        };
        match load() {
            Ok(Some(system)) => systems.push(system),
            Ok(None) => {}
            Err(error) => unsupported.push(error),
        }
    }
    systems
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
    let mut unsupported = Vec::new();
    let systems = physics_systems(catalog, document, placements, &mut unsupported);
    let system_ids: HashSet<String> = systems.iter().map(|s| s.entity_id.clone()).collect();
    let is_system = |id: &str| system_ids.contains(id);
    let dynamic = |id: &str| dynamic(id) || is_system(id);
    let mut result = MachineryRecipes {
        windmills: super::windmills(document, read)?,
        assemblies: Vec::new(),
        systems: Vec::new(),
        unsupported,
    };
    // Assemblies a force drives: motors, or a dynamic body that the wind
    // manager pushes (809d80 registers positive m_windSpeedFactor).
    let windy = |id: &str| {
        dynamic(id) && bodies.get(id).is_some_and(|b| b.wind_speed_factor.unwrap_or(0.) > 0.)
    };
    // Physics systems simulate from spawn (live Dust cables settle under
    // gravity before their LOD freezes them), so their joints seed too.
    for (seed, joint) in constraints.iter().enumerate().filter(|(_, j)| {
        j.started
            && (j.motor.is_some()
                || [j.entity_a.as_deref(), j.entity_b.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|id| windy(id) || is_system(id)))
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
                for (entity, joint) in [(&j.entity_a, &j.joint_a), (&j.entity_b, &j.joint_b)] {
                    if joint.is_some() && !entity.as_deref().is_some_and(is_system) {
                        return Err(format!(
                            "{}: articulated joint endpoints are not supported",
                            j.constraint_id
                        ));
                    }
                }
                if j.linear_constraint != "LINEAR_CONSTRAINT_FIXED"
                    || !matches!(
                        j.angular_constraint.as_str(),
                        "ANGULAR_CONSTRAINT_HINGE"
                            | "ANGULAR_CONSTRAINT_LIMITED_HINGE"
                            | "ANGULAR_CONSTRAINT_CONE"
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
            for id in ids.into_iter().filter(|id| !is_system(id)) {
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
                            let hulls = geometry
                                .convex
                                .iter()
                                .map(|hull| BodyHull {
                                    vertices: hull.vertices.clone(),
                                })
                                .collect();
                            wind = Some(BodyWind {
                                speed_factor,
                                faces,
                                hulls,
                                convex_radius: 0.01,
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
                    start_asleep: body
                        .start_asleep
                        .ok_or_else(|| format!("{id}: invalid m_startAsleep"))?,
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
    result.systems = systems;
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
                        "{:?} asleep={:?} f={:?} {:?}",
                        body.physics_behavior,
                        body.start_asleep,
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
    #[test]
    #[ignore = "audit; requires installed game archives"]
    fn installed_machinery_sleep_audit() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        let mut kinds = std::collections::BTreeMap::<String, usize>::new();
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let Ok(document) = entities::parse(&entities::declaration_text(&bytes)) else { continue };
            let read = |p: &str| {
                catalog
                    .read(p)
                    .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
                    .map_err(|e| e.to_string())
            };
            let mut placements = entities::render_placements(&document);
            let Ok(more) = entities::placed_models_with_inheritance(&document, read) else { continue };
            placements.extend(more);
            let Ok(recipes) = super::load(&catalog, &document, &placements) else { continue };
            for system in &recipes.systems {
                *kinds.entry(format!("SYSTEM {} {} bodies", system.system.name, system.system.bodies.len())).or_default() += 1;
            }
            for problem in recipes.unsupported.iter().filter(|p| p.contains("physics") || p.contains("ragdoll") || p.contains("MD6") || p.contains("articulated")) {
                *kinds.entry(format!("UNSUPPORTED {problem}")).or_default() += 1;
            }
            for assembly in &recipes.assemblies {
                let motor = assembly.joints.iter().any(|j| j.motor.is_some() && j.started);
                for body in assembly.bodies.iter().filter(|b| b.mass.is_some()) {
                    *kinds
                        .entry(format!(
                            "{} asleep={} motor={} wind={}",
                            path.split('/').nth(2).unwrap_or(&path),
                            body.start_asleep,
                            motor,
                            body.wind.is_some()
                        ))
                        .or_default() += 1;
                }
            }
        }
        for (kind, count) in kinds {
            println!("{count:5} {kind}");
        }
    }

    #[test]
    #[ignore = "audit; requires installed Dust District archives"]
    fn installed_dust_cable_constraint_audit() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let read = |p: &str| {
            catalog
                .read(p)
                .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        };
        for joint in super::super::constraints(&document, read).unwrap() {
            let ends = [joint.entity_a.as_deref().unwrap_or("-"), joint.entity_b.as_deref().unwrap_or("-")];
            if ends.iter().any(|e| e.contains("cable") || e.contains("speaker")) || joint.constraint_id.contains("cable") {
                println!("{joint:?}");
            }
        }
        for body in entities::rigid_body_settings(&document) {
            if body.entity_id.contains("speaker") || body.entity_id.contains("cable") {
                println!("BODY {} {:?} {:?}", body.entity_id, body.physics_behavior, body.start_asleep);
            }
        }
    }

}
