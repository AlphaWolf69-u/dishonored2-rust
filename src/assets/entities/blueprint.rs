//! Blueprint source/instance separation. Original registration1413517a0,
//! declaration ctor/dtor1401c10d0/1401c1270; assembly1401bc1f0 reads a
//! shared child array while1401bc3c0 retains separate runtime instance state.
//! Host: typed definitions live only through each static expansion batch.
use super::{PlacedModel, Value, field, float, parse_declaration, scope, vector};
use crate::archive::Catalog;
use std::collections::HashMap;

struct Child {
    resource_path: String,
    translation: [f32; 3],
    scale: [f32; 3],
    rotation: [[f32; 3]; 3],
}
pub(super) struct Blueprint {
    children: Vec<Child>,
}
impl Blueprint {
    pub(super) fn parse(declaration: &str) -> Result<Self, String> {
        let fields = parse_declaration(declaration)?;
        let edit = scope(field(&fields, "edit")).ok_or("blueprint has no edit scope")?;
        let submodels = scope(field(edit, "m_subModels")).ok_or("blueprint has no m_subModels")?;
        let count = match field(submodels, "num") {
            Some(Value::Atom(number)) => number
                .parse::<usize>()
                .map_err(|_| "invalid blueprint model count")?,
            _ => return Err("blueprint has no model count".into()),
        };
        if count > 4096 {
            return Err("blueprint model count exceeds 4096".into());
        }
        let mut children = Vec::with_capacity(count);
        for index in 0..count {
            let item = scope(field(submodels, &format!("item[{index}]")))
                .ok_or_else(|| format!("blueprint missing item[{index}]"))?;
            let model = match field(item, "m_model") {
                Some(Value::Atom(path)) => path.trim_matches('"'),
                _ => return Err(format!("blueprint item[{index}] has no model")),
            };
            let resource_path = if let Some(stem) = model.strip_suffix(".lwo") {
                format!("generated/model/{stem}.bmodel")
            } else if model.ends_with(".blueprint") {
                format!("generated/decls/blueprint/{model}.decl")
            } else {
                return Err(format!("unsupported blueprint child model: {model}"));
            };
            let translation = scope(field(item, "m_translation"))
                .and_then(|values| vector(values, [0.0; 3]))
                .ok_or_else(|| format!("invalid blueprint item[{index}] translation"))?;
            let child_scale = scope(field(item, "m_scale"))
                .and_then(|values| vector(values, [1.0; 3]))
                .ok_or_else(|| format!("invalid blueprint item[{index}] scale"))?;
            let angles = scope(field(item, "m_rotation"))
                .and_then(|values| {
                    Some([
                        float(values, "pitch", 0.0)?,
                        float(values, "yaw", 0.0)?,
                        float(values, "roll", 0.0)?,
                    ])
                })
                .ok_or_else(|| format!("invalid blueprint item[{index}] rotation"))?;
            // Blueprint pitch is authored around the upright Z axis. A 180-degree
            // fence repeat must reverse its horizontal run without turning the
            // upright spear and rails underneath the level.
            let (sp, cp) = angles[0].to_radians().sin_cos();
            let (sy, cy) = angles[1].to_radians().sin_cos();
            let (sr, cr) = angles[2].to_radians().sin_cos();
            let local = [
                [cp * cy, sp * cy, -sy],
                [cp * sy * sr - sp * cr, sp * sy * sr + cp * cr, cy * sr],
                [cp * sy * cr + sp * sr, sp * sy * cr - cp * sr, cy * cr],
            ];
            children.push(Child {
                resource_path,
                translation,
                scale: child_scale,
                rotation: local,
            });
        }
        Ok(Blueprint { children })
    }
    pub(super) fn expand(&self, parent: &PlacedModel) -> Vec<PlacedModel> {
        let mut children = Vec::with_capacity(self.children.len());
        for (index, child) in self.children.iter().enumerate() {
            let position = std::array::from_fn(|axis| {
                parent.position[axis]
                    + (0..3)
                        .map(|column| {
                            parent.rotation[column][axis]
                                * parent.scale[column]
                                * child.translation[column]
                        })
                        .sum::<f32>()
            });
            let rotation = std::array::from_fn(|column| {
                std::array::from_fn(|axis| {
                    (0..3)
                        .map(|basis| {
                            parent.rotation[basis][axis]
                                * parent.scale[basis]
                                * child.rotation[column][basis]
                                * child.scale[column]
                        })
                        .sum::<f32>()
                })
            });
            children.push(PlacedModel {
                id: format!("{}#blueprint[{index}]", parent.id),
                resource_path: child.resource_path.clone(),
                position,
                scale: [1.0; 3],
                rotation,
                custom_materials: Vec::new(),
                // Blueprint maxVisibleRangeVec ownership is not this parent's scalar.
                max_visible_range: None,
            });
        }
        children
    }
}

/// No source bytes or generic declaration tree survive parsing. This owner
/// retires before static instance setup continues; instances own their paths
/// and transforms independently. Failed immutable sources coalesce too.
pub struct Sources<'a> {
    catalog: &'a Catalog,
    definitions: HashMap<String, Result<Option<Blueprint>, String>>,
    requests: usize,
    reads: usize,
    trace: bool,
    separate: bool,
}
impl<'a> Sources<'a> {
    pub fn new(catalog: &'a Catalog) -> Self {
        Self {
            catalog,
            definitions: HashMap::new(),
            requests: 0,
            reads: 0,
            trace: std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"),
            separate: std::env::var("DH2_DEBUG_SEPARATE_BLUEPRINTS").as_deref() == Ok("1"),
        }
    }
    pub fn expand(&mut self, parent: &PlacedModel) -> Result<Option<Vec<PlacedModel>>, String> {
        let _tag = crate::platform::allocation_tags::tag("Blueprint preparation");
        self.requests += 1;
        let mut path = parent.resource_path.replace('\\', "/");
        path.make_ascii_lowercase();
        let shared = !self.separate && !self.catalog.has_overlay_file(&path);
        let load = |path: &str| -> Result<Option<Blueprint>, String> {
            self.catalog
                .read(path)
                .map_err(|error| error.to_string())?
                .map(|bytes| Blueprint::parse(&String::from_utf8_lossy(&bytes)))
                .transpose()
        };
        let result = if shared {
            let definition = match self.definitions.entry(path) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    self.reads += 1;
                    let definition = load(entry.key());
                    entry.insert(definition)
                }
            };
            definition
                .as_ref()
                .map(|value| value.as_ref().map(|value| value.expand(parent)))
                .map_err(Clone::clone)
        } else {
            self.reads += 1;
            load(&path).map(|value| value.map(|value| value.expand(parent)))
        };
        if self.trace {
            let mut digest = blake3::Hasher::new();
            digest.update(format!("{parent:?}:{result:?}").as_bytes());
            eprintln!(
                "BLUEPRINT_EXPANSION path={} shared={shared} signature={} success={}",
                parent.resource_path,
                digest.finalize(),
                result.is_ok()
            );
        }
        result
    }
}
impl Drop for Sources<'_> {
    fn drop(&mut self) {
        if self.trace {
            let mut logical_payload_bytes = 0;
            for definition in self
                .definitions
                .values()
                .filter_map(|value| value.as_ref().ok())
                .flatten()
            {
                logical_payload_bytes += std::mem::size_of::<Blueprint>()
                    + definition.children.capacity() * std::mem::size_of::<Child>()
                    + definition
                        .children
                        .iter()
                        .map(|child| child.resource_path.capacity())
                        .sum::<usize>();
            }
            eprintln!(
                "BLUEPRINT_PREPARATION_RELEASE requests={} reads={} definitions={} logical_payload_bytes={logical_payload_bytes}",
                self.requests,
                self.reads,
                self.definitions.len()
            );
        }
    }
}
