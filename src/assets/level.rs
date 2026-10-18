//! Native level resource relationships and direct archive reads.
use super::archive::Catalog;
use std::{error::Error, io};

pub const DEFAULT_LEVEL: &str = "eotw_p_lowchaos";
type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone)]
pub struct LevelResources {
    pub world: String,
    pub entities: String,
    pub tome: String,
    pub physics: Option<String>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

impl LevelResources {
    /// Accept an exact archive name or an unambiguous world stem. Never chooses
    /// the first level from an ambiguous name or falls back to extracted files.
    pub fn resolve(catalog: &Catalog, requested: &str) -> Result<Self> {
        let requested = requested.replace('\\', "/").to_ascii_lowercase();
        let candidates: Vec<_> = catalog
            .paths_with_suffix(".bwm")
            .into_iter()
            .filter(|path| {
                path == &requested
                    || (!requested.contains('/')
                        && path.rsplit('/').next().is_some_and(|name| {
                            name.trim_end_matches(".bwm") == requested.trim_end_matches(".bwm")
                        }))
            })
            .collect();
        let [world] = candidates.as_slice() else {
            return Err(invalid(format!(
                "expected one installed world for {requested:?}, found {}: {}",
                candidates.len(),
                candidates.join(", ")
            ))
            .into());
        };
        let without_extension = world.strip_suffix(".bwm").unwrap();
        let (directory, stem) = without_extension
            .rsplit_once('/')
            .ok_or_else(|| invalid("world has no resource directory"))?;
        let entities = format!("{directory}.entities");
        let tome = format!("{without_extension}.tome");
        // The index places compiled physics under the level directory, with a
        // filename that need not match the world stem. Resolve the indexed
        // relationship instead of inventing a filename or extracting/renaming.
        let physics_prefix = format!("generated/physworld/{directory}/");
        let physics_paths: Vec<_> = catalog
            .paths_with_suffix(".bphysworld")
            .into_iter()
            .filter(|path| path.starts_with(&physics_prefix))
            .collect();
        let exact = format!("{physics_prefix}{stem}.bphysworld");
        let physics = if physics_paths.contains(&exact) {
            Some(exact)
        } else {
            match physics_paths.as_slice() {
                [] => None,
                [path] => Some(path.clone()),
                _ => {
                    return Err(invalid(format!(
                        "ambiguous physics resources for {world}: {}",
                        physics_paths.join(", ")
                    ))
                    .into())
                }
            }
        };
        Ok(Self {
            world: world.clone(),
            entities,
            tome,
            physics,
        })
    }

    pub fn read_world(&self, catalog: &Catalog) -> Result<Vec<u8>> {
        catalog
            .read(&self.world)?
            .ok_or_else(|| invalid(format!("missing indexed world {}", self.world)).into())
    }
    pub fn read_entities(&self, catalog: &Catalog) -> Result<Option<Vec<u8>>> {
        catalog.read(&self.entities)
    }
    pub fn read_tome(&self, catalog: &Catalog) -> Result<Option<Vec<u8>>> {
        catalog.read(&self.tome)
    }
    pub fn read_physics(&self, catalog: &Catalog) -> Result<Option<Vec<u8>>> {
        match &self.physics {
            Some(path) => catalog.read(path),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{entities, physworld, tome, world_mesh};

    #[test]
    #[ignore = "requires installed Dishonored 2"]
    fn installed_level_companions_parse_directly_from_archives() {
        let catalog = Catalog::open(&super::super::installation::game_base().unwrap()).unwrap();
        let level = LevelResources::resolve(&catalog, DEFAULT_LEVEL).unwrap();
        let world = world_mesh::parse(&level.read_world(&catalog).unwrap()).unwrap();
        assert!(!world.meshes.is_empty());
        assert!(!world.objects.is_empty());
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = entities::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert!(entities::player_start(&document).is_some());
        assert!(!entities::render_placements(&document).is_empty());
        let bytes = level.read_tome(&catalog).unwrap().unwrap();
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
        tome::shadow_caster::ShadowCasterTome::parse(&bytes).unwrap();
        let bytes = level.read_physics(&catalog).unwrap().unwrap();
        let physics = physworld::parse(&bytes).unwrap();
        assert!(!physics
            .collision_shapes(&bytes)
            .unwrap()
            .triangles
            .is_empty());
    }

    #[test]
    #[ignore = "compares installed resources with private pre-refactor extraction"]
    fn archive_companions_are_byte_identical_to_verified_baseline() {
        let catalog = Catalog::open(&super::super::installation::game_base().unwrap()).unwrap();
        let level = LevelResources::resolve(&catalog, DEFAULT_LEVEL).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts");
        for (extension, bytes) in [
            ("bwm", level.read_world(&catalog).unwrap()),
            ("entities", level.read_entities(&catalog).unwrap().unwrap()),
            ("tome", level.read_tome(&catalog).unwrap().unwrap()),
            ("bphysworld", level.read_physics(&catalog).unwrap().unwrap()),
        ] {
            let baseline =
                std::fs::read(root.join(format!("{DEFAULT_LEVEL}.{extension}"))).unwrap();
            assert_eq!(bytes.len(), baseline.len(), "{extension} length");
            assert!(
                bytes == baseline,
                "{extension} differs from verified extraction"
            );
        }
    }
}
