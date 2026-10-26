//! Worldspawn bounds passed to the physics initializer, not render/Tome bounds.
use super::{components::atom, field, scope, vector, Document, Value};

/// 882300's idWorldspawn constructor. The map loader passes this member at
/// +490 to physics init (8a9b68/8a9b77); 15e0280 identifies m_havokBroadPhase.
pub const DEFAULT_PHYSICS_BOUNDS: [[f32; 4]; 2] =
    [[-1000., -1000., -1000., 0.], [1000., 1000., 1000., 0.]];

/// All installed worlds provide this override directly. If it is inherited,
/// require the declaration resolver instead of silently using a different box.
/// W is zeroed by 0932d0 when converting the two float3 bounds to world Cinfo.
pub fn world_physics_bounds(document: &Document) -> Result<Option<[[f32; 4]; 2]>, String> {
    let mut worlds = document
        .definitions
        .iter()
        .filter(|d| d.kind == "entity" && atom(&d.fields, "class") == Some("idWorldspawn"));
    let Some(world) = worlds.next() else {
        return Ok(None);
    };
    if worlds.next().is_some() {
        return Err("multiple worldspawn physics owners".into());
    }
    let edit = scope(field(&world.fields, "edit")).ok_or("worldspawn edit fields missing")?;
    let authored = scope(field(edit, "m_havokBroadPhase"))
        .ok_or("worldspawn physics bounds require their authored or resolved inherited fields")?;
    let mut bounds = DEFAULT_PHYSICS_BOUNDS;
    let Some(value) = field(authored, "b") else {
        return Ok(Some(bounds));
    };
    let Value::Scope(sides) = value else {
        return Err("physics bounds b is not a scope".into());
    };
    for (side, output) in bounds.iter_mut().enumerate() {
        let Some(value) = field(sides, &format!("b[{side}]")) else {
            continue;
        };
        let Value::Scope(values) = value else {
            return Err("physics bounds side is not a vector".into());
        };
        let xyz = vector(values, [output[0], output[1], output[2]])
            .ok_or("invalid worldspawn physics bounds vector")?;
        output[..3].copy_from_slice(&xyz);
    }
    Ok(Some(bounds))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_bounds_preserve_partial_constructor_values_and_zero_w() {
        let document = super::super::parse(
            r#"Version 6
entity { entityDef world { class = "idWorldspawn"; edit = {
 m_havokBroadPhase = { b = { b[0] = { x = -128; y = -64; }; b[1] = { z = 256; }; }; };
}; } }"#,
        )
        .unwrap();
        assert_eq!(
            world_physics_bounds(&document).unwrap().unwrap(),
            [[-128., -64., -1000., 0.], [1000., 1000., 256., 0.]]
        );
    }

    #[test]
    #[ignore = "requires installed level archives"]
    fn installed_world_physics_bounds() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let mut count = 0;
        for path in catalog.paths_with_suffix(".bwm") {
            let resources = crate::assets::level::LevelResources::resolve(&catalog, &path).unwrap();
            let bytes = catalog.read(&resources.entities).unwrap().unwrap();
            let document = super::super::parse(&super::super::declaration_text(&bytes)).unwrap();
            let bounds = world_physics_bounds(&document).unwrap().unwrap();
            for axis in 0..3 {
                assert!(bounds[0][axis] < bounds[1][axis], "{path}");
            }
            assert_eq!(bounds[0][3].to_bits(), 0);
            assert_eq!(bounds[1][3].to_bits(), 0);
            // This is the real input to the already compared world quantizer.
            let quantization = crate::gameplay::broadphase::BroadphaseQuantization::new(bounds);
            assert!(quantization.scale[..3]
                .iter()
                .all(|v| v.is_finite() && *v > 0.));
            count += 1;
        }
        assert_eq!(count, 36);
        println!("Resolved physics bounds and quantization for {count} installed worlds");
    }
}
