//! Authored hinge motor bindings. Positions and basis vectors remain Z-up.
use super::{field, parse_declaration, scope, vector, Document, Value};
use std::collections::HashMap;
pub mod recipes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotorMode {
    Velocity,
    Position,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AngularMotorSettings {
    pub mode: MotorMode,
    pub target_velocity: f32,
    pub target_acceleration: f32,
    pub force: f32,
}

#[derive(Debug, Clone)]
pub struct ConstraintSettings {
    pub constraint_id: String,
    pub entity_a: Option<String>,
    pub entity_b: Option<String>,
    pub joint_a: Option<String>,
    pub joint_b: Option<String>,
    pub pivot: [f32; 3],
    pub axis: [f32; 3],
    pub basis: [[f32; 3]; 3],
    pub angular_axis: usize,
    pub angular_constraint: String,
    pub angular_minimum: f32,
    pub angular_maximum: f32,
    pub angular_cone: f32,
    pub started: bool,
    pub target_position: f32,
    pub linear_constraint: String,
    pub angular_friction: f32,
    pub motor: Option<AngularMotorSettings>,
}

type Fields = Vec<(String, Value)>;

fn atom<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a str> {
    match field(fields, key)? {
        Value::Atom(value) => Some(value.trim_matches('"')),
        _ => None,
    }
}

fn overlay(base: &mut Fields, local: &[(String, Value)]) {
    for (key, value) in local {
        if let Some((_, previous)) = base.iter_mut().find(|(name, _)| name == key) {
            if let (Value::Scope(previous), Value::Scope(local)) = (&mut *previous, value) {
                overlay(previous, local);
            } else {
                *previous = value.clone();
            }
        } else {
            base.push((key.clone(), value.clone()));
        }
    }
}

fn component_fields(
    name: &str,
    depth: usize,
    cache: &mut HashMap<String, Fields>,
    read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Fields, String> {
    if depth > 16 {
        return Err(format!("constraint inheritance exceeds 16 at {name}"));
    }
    if let Some(fields) = cache.get(name) {
        return Ok(fields.clone());
    }
    let path = format!("generated/decls/cpntconstraint/{name}.decl");
    let source = read(&path)?.ok_or_else(|| format!("constraint declaration missing: {path}"))?;
    let fields = parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
    let mut resolved = match atom(&fields, "inherit") {
        Some(parent) => component_fields(parent, depth + 1, cache, read)?,
        None => Vec::new(),
    };
    overlay(
        &mut resolved,
        scope(field(&fields, "edit")).unwrap_or(&fields),
    );
    cache.insert(name.to_owned(), resolved.clone());
    Ok(resolved)
}

/// Resolve map-local, inherited-declaration and inline constraint settings.
pub fn constraints(
    document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<ConstraintSettings>, String> {
    let declarations: HashMap<_, _> = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "component" && definition.class == "cpntConstraint")
        .map(|definition| (definition.id.as_str(), definition))
        .collect();
    let mut cache = HashMap::new();
    let mut result = Vec::new();
    for entity in document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "entity")
    {
        let Some(edit) = scope(field(&entity.fields, "edit")) else {
            continue;
        };
        let Some(components) = scope(field(edit, "m_componentDecls")) else {
            continue;
        };
        for (key, entry) in components
            .iter()
            .filter(|(key, _)| key.starts_with("item_add["))
        {
            let Some(entry) = scope(Some(entry)) else {
                continue;
            };
            let mut fields = if let Some(name) = atom(entry, "mapComponentDecl") {
                let Some(declaration) = declarations.get(name) else {
                    continue;
                };
                let mut fields = match atom(&declaration.fields, "inherit") {
                    Some(parent) => component_fields(parent, 0, &mut cache, &mut read)?,
                    None => Vec::new(),
                };
                overlay(
                    &mut fields,
                    scope(field(&declaration.fields, "edit")).unwrap_or(&[]),
                );
                fields
            } else if let Some(name) = atom(entry, "inheritedDecl") {
                if !name.ends_with(".cpntconstraint") {
                    continue;
                }
                component_fields(name, 0, &mut cache, &mut read)?
            } else {
                continue;
            };
            if let Some(inline) = scope(field(entry, "inlineDecl")) {
                overlay(&mut fields, inline);
            }
            let mode = match atom(&fields, "m_angularMotorType") {
                Some("CONSTRAINT_MOTOR_VELOCITY") => Some(MotorMode::Velocity),
                Some("CONSTRAINT_MOTOR_POSITION") => Some(MotorMode::Position),
                None | Some("CONSTRAINT_MOTOR_NONE") => None,
                Some(value) => return Err(format!("{} {key}: unknown motor {value}", entity.id)),
            };
            let number = |key: &str, default: f32| -> Result<f32, String> {
                let Some(value) = atom(&fields, key) else {
                    return Ok(default);
                };
                value
                    .parse::<f32>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| format!("{}: invalid {key}={value}", entity.id))
            };
            let axis_index = match atom(&fields, "m_angularAxis").unwrap_or("AXIS_FORWARD") {
                "AXIS_FORWARD" => 0,
                "AXIS_LEFT" => 1,
                "AXIS_UP" => 2,
                value => return Err(format!("{}: unknown hinge axis {value}", entity.id)),
            };
            let basis = scope(field(edit, "spawnOrientation")).and_then(|v| scope(field(v, "mat")));
            let mut rotation = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
            for (index, axis) in rotation.iter_mut().enumerate() {
                if let Some(values) = basis.and_then(|v| scope(field(v, &format!("mat[{index}]"))))
                {
                    *axis = vector(values, *axis)
                        .ok_or_else(|| format!("{}: invalid constraint basis", entity.id))?;
                }
            }
            let axis = rotation[axis_index];
            let pivot = scope(field(edit, "spawnPosition"))
                .and_then(|v| vector(v, [0.; 3]))
                .ok_or_else(|| format!("{}: missing hinge pivot", entity.id))?;
            // 140712640 defaults; conversion coefficient read by 140713850.
            let radians = f32::from_bits(0x3c8e_fa35);
            let velocity = number("m_angularTargetVelocity", 360.)? * radians;
            let acceleration = (number("m_angularTargetAcceleration", 360.)? * radians).abs();
            let force = number("m_angularMotorForce", 1_000_000.)?;
            let started = match atom(&fields, "m_angularMotorStarted").unwrap_or("true") {
                "true" => true,
                "false" => false,
                value => return Err(format!("{}: invalid motor started {value}", entity.id)),
            };
            let endpoint = |key| {
                atom(&fields, key)
                    .filter(|value| !value.is_empty() && *value != "NULL")
                    .map(str::to_owned)
            };
            result.push(ConstraintSettings {
                constraint_id: entity.id.clone(),
                entity_a: endpoint("m_entityA"),
                entity_b: endpoint("m_entityB"),
                joint_a: endpoint("m_jointIdA"),
                joint_b: endpoint("m_jointIdB"),
                pivot,
                axis,
                basis: rotation,
                angular_axis: axis_index,
                angular_constraint: atom(&fields, "m_angularConstraintType")
                    .unwrap_or("ANGULAR_CONSTRAINT_FIXED")
                    .to_owned(),
                angular_minimum: number("m_angularMinAngle", -45.)? * radians,
                angular_maximum: number("m_angularMaxAngle", 45.)? * radians,
                angular_cone: number("m_angularConeAngle", 90.)? * radians,
                started,
                target_position: number("m_angularTargetPosition", 0.)? * radians,
                linear_constraint: atom(&fields, "m_linearConstraintType")
                    .unwrap_or("LINEAR_CONSTRAINT_FIXED")
                    .to_owned(),
                angular_friction: number("m_angularFriction", -1.)?,
                motor: mode.map(|mode| AngularMotorSettings {
                    mode,
                    target_velocity: if mode == MotorMode::Position {
                        velocity.abs()
                    } else {
                        velocity
                    },
                    target_acceleration: acceleration,
                    force,
                }),
            });
        }
    }
    Ok(result)
}

/// Physical material density at declaration +0x58, selected by 14080951a.
/// 14008b640 initializes it to 1000; inherited declarations override m_Density.
pub fn material_density(
    name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<f32, String> {
    fn resolve(
        name: &str,
        depth: usize,
        read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<f32, String> {
        if depth > 32 {
            return Err(format!("physics material inheritance exceeds 32: {name}"));
        }
        let path = format!("generated/decls/physicsmaterial/{name}.decl");
        let source = read(&path)?.ok_or_else(|| format!("missing physics material {path}"))?;
        let fields = parse_declaration(&source)?;
        let inherited = match atom(&fields, "inherit") {
            Some(parent) => resolve(parent, depth + 1, read)?,
            None => 1000.,
        };
        let edit = scope(field(&fields, "edit")).unwrap_or(&fields);
        match atom(edit, "m_Density") {
            Some(value) => value
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.)
                .ok_or_else(|| format!("invalid density in {path}")),
            None => Ok(inherited),
        }
    }
    resolve(name, 0, &mut read)
}

/// Convenience selection for the ambient motor audit; passive supporting
/// constraints are retained by `constraints` and participate in simulation.
pub fn hinge_motors(
    document: &Document,
    read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<ConstraintSettings>, String> {
    constraints(document, read).map(|all| {
        all.into_iter()
            .filter(|c| c.angular_constraint == "ANGULAR_CONSTRAINT_HINGE" && c.motor.is_some())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_use_constructor_defaults_and_inherited_inline_overrides() {
        let document = super::super::parse(
            r#"Version 6
            component { cpntConstraint motor { edit = {
                m_entityA = "blade"; m_angularConstraintType = "ANGULAR_CONSTRAINT_HINGE";
                m_angularMotorType = "CONSTRAINT_MOTOR_VELOCITY";
            } } }
            entity { entityDef hinge { edit = {
                spawnPosition = { x = 2; y = 3; z = 4; }
                m_componentDecls = { item_add["a"] = { mapComponentDecl = "motor"; } }
            } } }
            entity { entityDef fan { edit = {
                spawnPosition = { x = 8; }
                spawnOrientation = { mat = { mat[2] = { x = 0; y = 1; z = 0; } } }
                m_componentDecls = { item_add["b"] = {
                    inheritedDecl = "fan.cpntconstraint";
                    inlineDecl = { m_angularTargetVelocity = -150; m_angularMotorStarted = false; }
                } }
            } } }"#,
        )
        .unwrap();
        let motors = hinge_motors(&document, |name| {
            assert_eq!(
                name,
                "generated/decls/cpntconstraint/fan.cpntconstraint.decl"
            );
            Ok(Some(
                r#"{ edit = {
                m_entityA = "rotor"; m_angularConstraintType = "ANGULAR_CONSTRAINT_HINGE";
                m_angularAxis = "AXIS_UP"; m_angularMotorType = "CONSTRAINT_MOTOR_VELOCITY";
                m_angularTargetVelocity = 50;
            } }"#
                    .into(),
            ))
        })
        .unwrap();
        assert_eq!(motors.len(), 2);
        assert_eq!(motors[0].entity_a.as_deref(), Some("blade"));
        assert_eq!(motors[0].pivot, [2., 3., 4.]);
        assert_eq!(motors[0].axis, [1., 0., 0.]);
        assert!(motors[0].started);
        assert_eq!(motors[0].motor.unwrap().force, 1_000_000.);
        assert_eq!(
            motors[0].motor.unwrap().target_velocity,
            360. * f32::from_bits(0x3c8e_fa35)
        );
        assert_eq!(
            motors[1].motor.unwrap().target_velocity,
            -150. * f32::from_bits(0x3c8e_fa35)
        );
        assert_eq!(motors[1].axis, [0., 1., 0.]);
        assert!(!motors[1].started);
    }

    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_machinery_body_recipes() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(&base).unwrap();
        let bytes = catalog
            .read("maps/campaign/dust/dust_p_highchaos.entities")
            .unwrap()
            .unwrap();
        let document = super::super::parse(&super::super::declaration_text(&bytes)).unwrap();
        let read = |path: &str| {
            catalog
                .read(path)
                .map(|b| b.map(|b| super::super::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        };
        let motors = hinge_motors(&document, read).unwrap();
        let mut placements = super::super::render_placements(&document);
        placements.extend(super::super::placed_models_with_inheritance(&document, read).unwrap());
        let bodies = super::super::rigid_body_settings(&document);
        let links = super::super::constraint_links(&document);
        for motor in motors {
            println!(
                "MOTOR {} A={:?} B={:?} axis={:?} pivot={:?}",
                motor.constraint_id, motor.entity_a, motor.entity_b, motor.axis, motor.pivot
            );
            for id in [motor.entity_a, motor.entity_b].into_iter().flatten() {
                let Some(p) = placements.iter().find(|p| p.id == id) else {
                    println!(" no model {id}");
                    continue;
                };
                let Some(body) = bodies.iter().find(|b| b.entity_id == id) else {
                    println!(" no body {id}");
                    continue;
                };
                println!(
                    " BODY {id} {} position={:?} rotation={:?} scale={:?} {:?}",
                    p.resource_path, p.position, p.rotation, p.scale, body
                );
                let source = p
                    .resource_path
                    .strip_prefix("generated/model/")
                    .and_then(|s| s.strip_suffix(".bmodel"))
                    .map(|s| crate::physworld::replace_shape_extension(s, "lwo"));
                if let Some(source) = source {
                    let kind =
                        super::super::rigid_body_shape_kind(body.shape_type.as_deref().unwrap())
                            .unwrap();
                    let key = crate::physworld::physics_shape_key(
                        &source,
                        crate::physworld::physics_shape_kind(
                            ["hkt", "cvx", "msh", "box", "cvd"][kind as usize],
                        ),
                        p.scale,
                        0.01,
                        false,
                    )
                    .unwrap();
                    let path = format!("generated/{key}.massProps");
                    if let Some(bytes) = catalog.read(&path).unwrap() {
                        println!(" MASS {path} {:?}", crate::physworld::mass::parse(&bytes));
                    } else {
                        println!(" MISSING MASS {path}");
                    }
                }
                for link in links.iter().filter(|l| {
                    l.entity_a_id.as_deref() == Some(&id) || l.entity_b_id.as_deref() == Some(&id)
                }) {
                    println!(" LINK {link:?}");
                }
            }
        }
    }

    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_levels_resolve_hinge_motor_bindings() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::archive::Catalog::open(&base).unwrap();
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let document = super::super::parse(&super::super::declaration_text(&bytes)).unwrap();
            let motors = hinge_motors(&document, |path| {
                catalog
                    .read(path)
                    .map(|bytes| {
                        bytes.map(|bytes| super::super::declaration_text(&bytes).into_owned())
                    })
                    .map_err(|error| error.to_string())
            })
            .unwrap_or_else(|error| panic!("{path}: {error}"));
            println!("MACHINERY {path}: {} hinge motors", motors.len());
        }
    }
}
