//! Canonical immutable environment declarations leased by FX and view slots.
//! Descriptor resource check21a080, lazy per-view binding21a190; source syntax
//! is temporary and the existing registered resource owner controls residency.
use crate::{archive::Catalog, assets::entities::{self, EnvironmentRenderParameters}};
use std::{collections::{HashMap, HashSet}, sync::{Arc, atomic::AtomicBool}};

#[derive(Clone, Debug)]
pub enum PreparedValue { Float([f32;4]), Unsigned([u32;4]), Signed([i32;4]), Boolean(bool), Image(String), Material(Option<String>) }

pub struct PreparedEnvironment {
    pub name: String,
    pub parameters: EnvironmentRenderParameters,
    pub values: HashMap<String, PreparedValue>,
    /// Set by the canonical consumer only after every required projection/image is admitted.
    pub prepared: AtomicBool,
}
impl PreparedEnvironment {
    /// Called by existing background FX source acquisition, never Start/update.
    pub fn acquire(catalog: &Catalog, name: &str, marker:u8) -> Result<Arc<Self>, String> {
        let path = format!("generated/decls/env/{name}.decl").replace('\\', "/").to_ascii_lowercase();
        let mut owner = catalog.environment_resources.get_or_init(Default::default).lock()
            .map_err(|_| "environment resource registry is poisoned")?;
        owner.acquire(&path, marker, |_| {
            let read = |name: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog,
                &format!("generated/decls/env/{name}.decl"));
            let text = read(name)?.ok_or_else(|| format!("environment {name} is not installed"))?;
            let parameters = entities::environment_render_parameters_with_inheritance(name, &text, read)?;
            let reader = crate::assets::environment::ParameterReader { catalog, parameters: &parameters };
            let mut values = HashMap::new();
            let mut seen = HashSet::new();
            for layer in &parameters.layers {
                for (key, _) in layer.parameters.iter().rev() {
                    let key = key.to_ascii_lowercase();
                    if !seen.insert(key.clone()) { continue; }
                    let declaration = crate::assets::render_parameter::acquire(catalog, &key)?;
                    let metadata = declaration.metadata()?;
                    use crate::render_program::{RenderParameterKind as Kind, NumericStorage};
                    let value = match metadata.kind {
                        Kind::Texture => PreparedValue::Image(reader.image_source(&key)?),
                        Kind::Material => PreparedValue::Material(reader.material_source(&key)?),
                        Kind::Uniform => {
                            let text = declaration.text();
                            let tokens = entities::declaration_lexemes(&text)?;
                            if tokens.get(2).is_some_and(|kind| kind.eq_ignore_ascii_case("bool")) {
                                PreparedValue::Boolean(reader.boolean(&key)?)
                            } else { match metadata.numeric {
                                Some(NumericStorage::Float | NumericStorage::Color) => PreparedValue::Float(reader.vector(&key)?),
                                Some(storage @ NumericStorage::Unsigned) => PreparedValue::Unsigned(integer_values(&parameters, &key, storage, metadata.lanes)?),
                                Some(storage @ NumericStorage::Signed) => PreparedValue::Signed(integer_values(&parameters, &key, storage, metadata.lanes)?.map(|word| word as i32)),
                                _ => return Err(format!("environment {name}: unsupported numeric override {key}")),
                            }}
                        },
                        _ => return Err(format!("environment {name}: unsupported registered override {key}: {:?}", metadata.kind)),
                    };
                    values.insert(key, value);
                }
            }
            Ok(Arc::new(Self { name: name.to_owned(), parameters, values, prepared: false.into() }))
        })
    }
}
pub(crate) fn release(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.environment_resources.get() {
        owner.lock().unwrap_or_else(|error| error.into_inner()).release(catalog.identity(), "env", marker);
    }
}

/// Reuse the existing1401461c0 numeric token conversion, preserving integer bits.
fn integer_values(parameters:&EnvironmentRenderParameters, key:&str,
    storage:crate::render_program::NumericStorage, lanes:u8)->Result<[u32;4],String>{
    let source=parameters.parameter(key).ok_or("explicit integer environment parameter disappeared")?;
    let tokens=entities::declaration_lexemes(source)?;
    let spelling=tokens.iter().copied().filter(|token| !matches!(*token,"{"|"}")).collect::<Vec<_>>().join(" ");
    let parts=spelling.split(',').collect::<Vec<_>>();
    if parts.is_empty() || parts.len()>usize::from(lanes) || parts.len()>4 {return Err(format!("{key}: invalid integer lanes"));}
    let mut words=[0;4];
    for (word,part) in words.iter_mut().zip(&parts){
        *word=crate::assets::parm_block::numeric_word(part,storage)?;
    }
    if parts.len()==1 {words=[words[0];4];}
    Ok(words)
}
