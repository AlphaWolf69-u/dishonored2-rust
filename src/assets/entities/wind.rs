//! Authored winds: only an idWorldspawn-owned component registers the
//! camera/world wind (77fec0 -> 8c3eb0 -> 882ce0). Other entities' winds are
//! local to their triggers and are found only by the wind manager's queries.
use super::{
    components::{atom, component_fields, overlay, Fields},
    field, scope, vector, Definition, Document, Value,
};
use bevy_math::Vec3;
use std::collections::HashMap;
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// A resolved arkDeclCpntWind: settings plus `m_priority` (decl +0x68, zero
/// in constructor 77efe0), which 1405ff500 reads to order trigger winds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindComponent {
    pub settings: WindSettings,
    pub priority: i32,
}

/// An entity's trigger-linked wind. arkCpntTrigger init 140779530 stores the
/// entity's first arkCpntWind (877720) at trigger+0xc8 once its shape loads;
/// arkWindManager::PostLoadMap (7b7be0) finds these triggers per wind center.
#[derive(Clone, Debug)]
pub struct TriggerWind {
    pub entity: String,
    pub wind: WindComponent,
    /// renderModelInfo.model: the trigger shape resource read by 140779530.
    pub model: Option<String>,
    pub position: [f32; 3],
    pub rotation: [[f32; 3]; 3],
}

struct ComponentResolver<'a, R> {
    declarations: HashMap<&'a str, &'a Definition>,
    cache: HashMap<String, Fields>,
    read: R,
}

impl<'a, R: FnMut(&str) -> Result<Option<String>, String>> ComponentResolver<'a, R> {
    fn new(document: &'a Document, read: R) -> Self {
        Self {
            declarations: document
                .definitions
                .iter()
                .filter(|d| d.kind == "component")
                .map(|d| (d.id.as_str(), d))
                .collect(),
            cache: HashMap::new(),
            read,
        }
    }

    /// The lower-case component class an `item_add` entry attaches.
    fn class(&self, entry: &[(String, Value)]) -> Option<String> {
        if let Some(name) = atom(entry, "mapComponentDecl") {
            return self
                .declarations
                .get(name)
                .map(|d| d.class.to_ascii_lowercase());
        }
        let (_, extension) = atom(entry, "inheritedDecl")?.rsplit_once('.')?;
        Some(extension.to_ascii_lowercase())
    }

    fn wind(&mut self, entry: &[(String, Value)]) -> Result<Option<WindComponent>, String> {
        let mut fields = if let Some(name) = atom(entry, "mapComponentDecl") {
            let Some(decl) = self
                .declarations
                .get(name)
                .copied()
                .filter(|decl| decl.class.eq_ignore_ascii_case("cpntWind"))
            else {
                return Ok(None);
            };
            let mut fields = if let Some(parent) = atom(&decl.fields, "inherit") {
                component_fields("cpntwind", parent, 0, &mut self.cache, &mut self.read)?
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
            component_fields("cpntwind", name, 0, &mut self.cache, &mut self.read)?
        } else {
            return Ok(None);
        };
        if let Some(inline) = scope(field(entry, "inlineDecl")) {
            overlay(&mut fields, inline);
        }
        wind_component(&fields).map(Some)
    }
}

fn wind_component(fields: &[(String, Value)]) -> Result<WindComponent, String> {
    let defaults = WindSettings::default();
    let number = |key: &str, default: f32| -> Result<f32, String> {
        let value = match atom(fields, key) {
            Some(value) => value
                .parse::<f32>()
                .map_err(|_| format!("invalid wind {key}"))?,
            None => default,
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(format!("nonfinite wind {key}"))
        }
    };
    let integer = |key: &str, default: i32| -> Result<i32, String> {
        atom(fields, key)
            .map(|value| {
                value
                    .parse::<i32>()
                    .map_err(|_| format!("invalid wind {key}"))
            })
            .transpose()
            .map(|value| value.unwrap_or(default))
    };
    let direction = match scope(field(fields, "m_direction")) {
        Some(fields) => vector(fields, defaults.direction.to_array())
            .map(Vec3::from_array)
            .ok_or("invalid wind direction")?,
        None => defaults.direction,
    };
    if !direction.is_finite() {
        return Err("nonfinite wind direction".into());
    }
    Ok(WindComponent {
        settings: WindSettings {
            direction,
            main_speed: number("m_mainSpeed", defaults.main_speed)?,
            speed_variation: number("m_speedVariation", defaults.speed_variation)?,
            noise_ratio: number("m_noiseRatio", defaults.noise_ratio)?,
            frequency: number("m_frequency", defaults.frequency)?,
            persistence: number("m_persistence", defaults.persistence)?,
            octaves: integer("m_numOctaves", defaults.octaves)?,
        },
        priority: integer("m_priority", 0)?,
    })
}

/// `item_add` component entries in attachment order.
fn component_entries(definition: &Definition) -> Vec<&[(String, Value)]> {
    scope(field(&definition.fields, "edit"))
        .and_then(|edit| scope(field(edit, "m_componentDecls")))
        .map(|components| {
            components
                .iter()
                .filter(|(key, _)| key.starts_with("item_add["))
                .filter_map(|(_, entry)| scope(Some(entry)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn world_wind(
    document: &Document,
    read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Option<WindSettings>, String> {
    let mut result = None;
    let mut resolver = ComponentResolver::new(document, read);
    for world in document
        .definitions
        .iter()
        .filter(|d| d.kind == "entity" && atom(&d.fields, "class") == Some("idWorldspawn"))
    {
        for entry in component_entries(world) {
            // Registration assigns the pointer in component attachment order.
            if let Some(wind) = resolver.wind(entry)? {
                result = Some(wind.settings);
            }
        }
    }
    Ok(result)
}

/// Every non-world entity whose components include a trigger and a wind.
pub fn trigger_winds(
    document: &Document,
    read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<TriggerWind>, String> {
    let mut resolver = ComponentResolver::new(document, read);
    let mut result = Vec::new();
    for entity in document
        .definitions
        .iter()
        .filter(|d| d.kind == "entity" && atom(&d.fields, "class") != Some("idWorldspawn"))
    {
        let entries = component_entries(entity);
        let is = |entry: &&[(String, Value)], class: &str| {
            resolver.class(entry).as_deref() == Some(class)
        };
        if !entries.iter().any(|entry| is(entry, "cpnttrigger")) {
            continue;
        }
        let Some(entry) = entries.iter().find(|entry| is(entry, "cpntwind")).copied() else {
            continue;
        };
        let wind = resolver
            .wind(entry)?
            .ok_or_else(|| format!("{}: unresolved trigger wind declaration", entity.id))?;
        let edit = scope(field(&entity.fields, "edit")).unwrap_or(&[]);
        let position = match scope(field(edit, "spawnPosition")) {
            Some(fields) => vector(fields, [0.0; 3])
                .ok_or_else(|| format!("{}: invalid trigger spawnPosition", entity.id))?,
            None => [0.0; 3],
        };
        let mut rotation = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        if let Some(matrix) =
            scope(field(edit, "spawnOrientation")).and_then(|fields| scope(field(fields, "mat")))
        {
            for (index, axis) in rotation.iter_mut().enumerate() {
                if let Some(values) = scope(field(matrix, &format!("mat[{index}]"))) {
                    *axis = vector(values, *axis)
                        .ok_or_else(|| format!("{}: invalid trigger orientation", entity.id))?;
                }
            }
        }
        result.push(TriggerWind {
            entity: entity.id.clone(),
            wind,
            model: scope(field(edit, "renderModelInfo"))
                .and_then(|info| atom(info, "model"))
                .map(str::to_owned),
            position,
            rotation,
        });
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
component { cpntAudio sound { edit={m_enablePropagation=false;} } }
entity { entityDef world {class="idWorldspawn";edit={m_componentDecls={item_add["wind"]={mapComponentDecl="breeze";inlineDecl={m_noiseRatio=0.2;}}item_add["audio"]={mapComponentDecl="sound";}}}} }
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
    fn trigger_winds_need_a_trigger_and_take_the_first_wind_component() {
        let document = super::super::parse(r#"Version 6
component { cpntTrigger shape {} }
entity { entityDef world {class="idWorldspawn";edit={m_componentDecls={item_add["t"]={inheritedDecl="a.cpnttrigger";}item_add["w"]={inheritedDecl="calm.cpntwind";}}}} }
entity { entityDef open {class="arkAreaVolume";edit={m_componentDecls={item_add["w"]={inheritedDecl="calm.cpntwind";}}}} }
entity { entityDef room {class="arkAreaVolume";edit={
  m_componentDecls={item_add["t"]={mapComponentDecl="shape";}item_add["w"]={inheritedDecl="calm.cpntwind";inlineDecl={m_priority=3;}}item_add["x"]={inheritedDecl="strong.cpntwind";}}
  spawnPosition={x=1;y=2;z=3;} spawnOrientation={mat={mat[0]={x=0;y=1;z=0;}mat[1]={x=-1;y=0;z=0;}}}
  renderModelInfo={model="maps/test/room";}}} }
"#).unwrap();
        let winds = trigger_winds(&document, |name| {
            Ok(Some(match name {
                "generated/decls/cpntwind/calm.cpntwind.decl" => "{edit={m_mainSpeed=0.5;}}",
                "generated/decls/cpntwind/strong.cpntwind.decl" => "{edit={m_mainSpeed=9;}}",
                _ => panic!("{name}"),
            }.into()))
        })
        .unwrap();
        assert_eq!(winds.len(), 1);
        let room = &winds[0];
        assert_eq!(room.entity, "room");
        assert_eq!(room.wind.priority, 3);
        assert_eq!(room.wind.settings.main_speed, 0.5);
        assert_eq!(room.wind.settings.speed_variation, 2.);
        assert_eq!(room.model.as_deref(), Some("maps/test/room"));
        assert_eq!(room.position, [1., 2., 3.]);
        assert_eq!(room.rotation, [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]]);
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
            let read = |name: &str| {
                catalog
                    .read(name)
                    .map(|bytes| {
                        bytes.map(|bytes| super::super::declaration_text(&bytes).into_owned())
                    })
                    .map_err(|e| e.to_string())
            };
            let wind = world_wind(&document, read).unwrap();
            let triggers = trigger_winds(&document, read).unwrap();
            println!(
                "{environment}: {wind:?}; {} trigger winds, {} without a model",
                triggers.len(),
                triggers.iter().filter(|t| t.model.is_none()).count()
            );
            maps += 1;
            if path == "maps/campaign/dust/dust_p_lowchaos.entities" {
                assert_eq!(wind.unwrap().direction, Vec3::new(1., 0., 0.));
            }
        }
        assert_eq!(maps, 36);
    }
}
