//! Authored world wind: only an idWorldspawn-owned component registers the
//! camera wind (77fec0 -> 8c3eb0 -> 882ce0). Area-volume winds remain local.
use super::{
    components::{atom, component_fields, overlay},
    field, scope, vector, Document,
};
use bevy_math::Vec3;
use std::collections::HashMap;
#[derive(Clone, Copy, Debug)]
pub struct WindSettings {
    pub direction: Vec3,
    pub main_speed: f32,
    pub speed_variation: f32,
    pub noise_ratio: f32,
    pub frequency: f32,
    pub persistence: f32,
    pub octaves: i32,
}
impl Default for WindSettings {
    fn default() -> Self {
        // arkDeclCpntWind constructor 77efe0.
        Self {
            direction: Vec3::X,
            main_speed: 3.,
            speed_variation: 2.,
            noise_ratio: 0.1,
            frequency: 0.1,
            persistence: 0.5,
            octaves: 4,
        }
    }
}

pub fn world_wind(
    document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Option<WindSettings>, String> {
    let mut result = None;
    let mut cache = HashMap::new();
    let declarations: HashMap<_, _> = document
        .definitions
        .iter()
        .filter(|d| d.kind == "component" && d.class == "cpntWind")
        .map(|d| (d.id.as_str(), d))
        .collect();
    for world in document
        .definitions
        .iter()
        .filter(|d| d.kind == "entity" && atom(&d.fields, "class") == Some("idWorldspawn"))
    {
        let Some(edit) = scope(field(&world.fields, "edit")) else {
            continue;
        };
        let Some(components) = scope(field(edit, "m_componentDecls")) else {
            continue;
        };
        for (_, entry) in components
            .iter()
            .filter(|(key, _)| key.starts_with("item_add["))
        {
            let Some(entry) = scope(Some(entry)) else {
                continue;
            };
            let mut fields = if let Some(name) = atom(entry, "mapComponentDecl") {
                let Some(decl) = declarations.get(name) else {
                    continue;
                };
                let mut fields = if let Some(parent) = atom(&decl.fields, "inherit") {
                    component_fields("cpntwind", parent, 0, &mut cache, &mut read)?
                } else {
                    Vec::new()
                };
                overlay(
                    &mut fields,
                    scope(field(&decl.fields, "edit")).unwrap_or(&[]),
                );
                fields
            } else if let Some(name) =
                atom(entry, "inheritedDecl").filter(|name| name.ends_with(".cpntwind"))
            {
                component_fields("cpntwind", name, 0, &mut cache, &mut read)?
            } else {
                continue;
            };
            if let Some(inline) = scope(field(entry, "inlineDecl")) {
                overlay(&mut fields, inline);
            }
            let defaults = WindSettings::default();
            let number = |key: &str, default: f32| -> Result<f32, String> {
                let value = match atom(&fields, key) {
                    Some(value) => value
                        .parse::<f32>()
                        .map_err(|_| format!("invalid world wind {key}"))?,
                    None => default,
                };
                if value.is_finite() {
                    Ok(value)
                } else {
                    Err(format!("nonfinite world wind {key}"))
                }
            };
            let direction = match scope(field(&fields, "m_direction")) {
                Some(fields) => vector(fields, defaults.direction.to_array())
                    .map(Vec3::from_array)
                    .ok_or("invalid world wind direction")?,
                None => defaults.direction,
            };
            if !direction.is_finite() {
                return Err("nonfinite world wind direction".into());
            }
            let octaves = atom(&fields, "m_numOctaves")
                .map(|value| {
                    value
                        .parse::<i32>()
                        .map_err(|_| "invalid world wind octaves".to_string())
                })
                .transpose()?
                .unwrap_or(defaults.octaves);
            // Registration assigns the pointer in component attachment order.
            result = Some(WindSettings {
                direction,
                main_speed: number("m_mainSpeed", defaults.main_speed)?,
                speed_variation: number("m_speedVariation", defaults.speed_variation)?,
                noise_ratio: number("m_noiseRatio", defaults.noise_ratio)?,
                frequency: number("m_frequency", defaults.frequency)?,
                persistence: number("m_persistence", defaults.persistence)?,
                octaves,
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn world_binding_uses_inheritance_inline_defaults_and_ignores_local_winds() {
        let document=super::super::parse(r#"Version 6
component { cpntWind breeze { inherit="parent.cpntwind"; edit={m_direction={z=0.5;} m_mainSpeed=12;} } }
entity { entityDef world {class="idWorldspawn";edit={m_componentDecls={item_add["wind"]={mapComponentDecl="breeze";inlineDecl={m_noiseRatio=0.2;}}}}} }
entity { entityDef local {class="arkAreaVolume";edit={m_componentDecls={item_add["wind"]={inheritedDecl="local.cpntwind";}}}} }
"#).unwrap();
        let wind = world_wind(&document, |name| {
            assert_eq!(name, "generated/decls/cpntwind/parent.cpntwind.decl");
            Ok(Some(
                "{edit={m_direction={x=0;y=-1;}m_speedVariation=4;}}".into(),
            ))
        })
        .unwrap()
        .unwrap();
        assert_eq!(wind.direction, Vec3::new(0., -1., 0.5));
        assert_eq!(wind.main_speed, 12.);
        assert_eq!(wind.speed_variation, 4.);
        assert_eq!(wind.noise_ratio, 0.2);
        assert_eq!(wind.octaves, 4);
    }
    #[test]
    #[ignore = "requires installed level and component archives"]
    fn installed_world_wind_settings() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let mut maps = 0;
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let document = super::super::parse(&super::super::declaration_text(&bytes)).unwrap();
            let Some(environment) = super::super::world_environment(&document) else {
                continue;
            };
            let wind = world_wind(&document, |name| {
                catalog
                    .read(name)
                    .map(|bytes| {
                        bytes.map(|bytes| super::super::declaration_text(&bytes).into_owned())
                    })
                    .map_err(|e| e.to_string())
            })
            .unwrap();
            println!("{environment}: {wind:?}");
            maps += 1;
            if path == "maps/campaign/dust/dust_p_lowchaos.entities" {
                assert_eq!(wind.unwrap().direction, Vec3::new(1., 0., 0.));
            }
        }
        assert_eq!(maps, 36);
    }
}
