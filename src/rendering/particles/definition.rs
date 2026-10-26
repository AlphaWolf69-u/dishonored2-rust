//! Installed particle definitions and shared material preparation. Both player
//! and world FX use this loader; playback ownership stays with their caller.
use super::material::ParticleMaterial;
use crate::{
    archive::Catalog,
    assets::{
        entities::Value,
        fx::{particle_declaration, particle_system},
    },
    gameplay::options::decl::read_edit,
    shader_cache::ProgramLibrary,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub struct ParticleDefinition {
    pub owner: Arc<particle_system::ParticleSystemAsset>,
    pub declarations: Vec<Option<Arc<particle_declaration::ParticleDeclaration>>>,
    pub materials: HashMap<String, Arc<ParticleMaterial>>,
}
impl ParticleDefinition {
    pub fn load(
        fields: &[(String, Value)],
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        material_cache: &mut HashMap<String, Arc<ParticleMaterial>>,
    ) -> Result<Self, String> {
        let owner = particle_system::particle_system(fields)?;
        let mut models = HashMap::new();
        let declarations = owner
            .declarations
            .iter()
            .map(|path| {
                let Some(path) = path else { return Ok(None) };
                let mut read = |p: &str| catalog.read(p).ok().flatten();
                let fields = read_edit(&mut read, "particledt", path)?;
                let mut declaration = particle_declaration::declaration(&fields)
                    .map_err(|error| format!("{path}: {error}"))?;
                // Emission meshes are dedicated PMODEL archive resources,
                // independent of the ordinary rendered-prop mesh loader.
                for stage in &mut declaration.stages {
                    let mesh = &mut stage.spawn_mesh;
                    let Some(source) = &mesh.source else { continue };
                    let resource = crate::assets::particle_model::resource_path(source)?;
                    if !models.contains_key(&resource) {
                        let bytes = catalog
                            .read(&resource)
                            .map_err(|error| format!("{resource}: {error}"))?
                            .ok_or_else(|| format!("missing particle emission mesh {resource}"))?;
                        models.insert(
                            resource.clone(),
                            Arc::new(
                                crate::assets::particle_model::parse(&bytes)
                                    .map_err(|error| format!("{resource}: {error}"))?,
                            ),
                        );
                    }
                    mesh.model = Some(models[&resource].clone());
                }
                Ok(Some(Arc::new(declaration)))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut materials = HashMap::new();
        for declaration in declarations.iter().flatten() {
            for stage in declaration.stages.iter().filter(|stage| stage.enabled) {
                for sprite in stage.sprites.iter().filter(|sprite| sprite.enabled) {
                    let Some(name) = &sprite.material else {
                        continue;
                    };
                    if !material_cache.contains_key(name) {
                        material_cache.insert(
                            name.clone(),
                            Arc::new(ParticleMaterial::load(catalog, library, name)?),
                        );
                    }
                    materials.insert(name.clone(), material_cache[name].clone());
                }
            }
        }
        Ok(Self {
            owner: Arc::new(owner),
            declarations,
            materials,
        })
    }
}
