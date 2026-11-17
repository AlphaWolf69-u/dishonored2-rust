//! arkDeclCpntAIDangerZone41d420 has only the common declaration fields.
pub mod startup_managers;
pub mod crew;
pub mod crew_collectors;
pub mod crew_modules;
pub mod crew_tree;
#[derive(Debug)]
pub struct DangerDefinition;
impl DangerDefinition {
    pub fn read(_fields: &[(String, crate::entities::Value)]) -> Result<Self, String> { Ok(Self) }
}
