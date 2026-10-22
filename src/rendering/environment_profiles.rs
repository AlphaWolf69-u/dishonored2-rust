//! Resolve effect profiles once, retaining supplied parameters and prepared
//! image identities. Each cache belongs to one immutable subsystem schema.
use super::environment_parameters::{EnvironmentParameters, EnvironmentValue};
use crate::assets::{archive::Catalog, entities};
use std::{collections::HashMap, sync::Arc};

pub struct PreparedEnvironmentProfile<I> {
    pub parameters: EnvironmentParameters,
    /// Image parameter name to the owner's prepared resource identity.
    pub images: HashMap<String, I>,
}

pub struct EnvironmentProfiles<I> {
    schema: EnvironmentParameters,
    profiles: HashMap<String, Arc<PreparedEnvironmentProfile<I>>>,
}
impl<I> EnvironmentProfiles<I> {
    pub fn new(schema: &EnvironmentParameters) -> Self {
        Self {
            schema: schema.clone(),
            profiles: HashMap::new(),
        }
    }

    /// The image owner supplies its existing archive/intrinsic loader and
    /// residency cache. Successful repeated requests reuse the same profile.
    /// Preparation failures are returned and never cached as empty profiles.
    pub fn load(
        &mut self,
        catalog: &Catalog,
        name: &str,
        mut prepare_image: impl FnMut(&str, &str) -> Result<I, String>,
    ) -> Result<Arc<PreparedEnvironmentProfile<I>>, String> {
        let key = name.to_ascii_lowercase();
        if let Some(profile) = self.profiles.get(&key) {
            return Ok(Arc::clone(profile));
        }
        let read = |name: &str| -> Result<Option<String>, String> {
            catalog
                .read(&format!("generated/decls/env/{name}.decl"))
                .map(|v| v.map(|bytes| entities::declaration_text(&bytes).into_owned()))
                .map_err(|e| e.to_string())
        };
        let source = read(&key)?.ok_or_else(|| format!("environment profile not found: {name}"))?;
        let authored =
            entities::environment_render_parameters_with_inheritance(&key, &source, read)?;
        let parameters = EnvironmentParameters::supplied(catalog, &authored, &self.schema)?;
        let mut images = HashMap::new();
        for (parameter, value) in &parameters.values {
            if let EnvironmentValue::Image(source) = value {
                images.insert(parameter.clone(), prepare_image(parameter, source)?);
            }
        }
        let profile = Arc::new(PreparedEnvironmentProfile { parameters, images });
        self.profiles.insert(key, Arc::clone(&profile));
        Ok(profile)
    }
}

#[cfg(test)]
#[path = "environment_profiles/tests.rs"]
mod tests;
