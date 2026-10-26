//! Installed executable's constant class declarations and startup order.
//! Reads metadata only; no game code is loaded or executed and no extraction is
//! required. The audited compiler layout and executable version are explicit.
use super::executable::{word, Image};
use crate::gameplay::class_registry::{ClassDefinition, ClassRegistry};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};
mod component_definitions;
mod component_types;
mod components;
pub use component_definitions::{
    ComponentDefinition, ComponentDependencyDeclaration, ComponentOrderingDeclaration,
};
pub use component_types::ComponentTypeDeclaration;
pub use components::ComponentPoolDeclaration;

pub(crate) const IMAGE_HASH: &str =
    "09cd68ead8e2ee8f4d67b2ab901df5ae404981744a9aba309b58ccbb25c763f1";
const LAYOUT_HASH: &str = "b56537cfaf78ed9659cecde2a8af5aad7d36827131ac48604ca9d9adccfdf96e";
const REGISTER_RVA: u32 = 0x1990d0;
const CALL_OFFSET: usize = 0x89;
const INITIALIZER_SIZE: usize = 0x8e;
const VARIABLES: &[(usize, usize)] = &[
    (0x0d, 4),
    (0x14, 4),
    (0x1b, 4),
    (0x28, 4),
    (0x37, 4),
    (0x50, 4),
    (0x66, 4),
    (0x71, 4),
    (0x7c, 4),
    (0x88, 1),
    (0x8a, 4),
];

#[derive(Debug)]
pub struct ClassDeclaration {
    pub name: Vec<u8>,
    pub base_name: Vec<u8>,
    pub instance_size: u32,
    pub flags: u8,
    pub initializer_rva: u32,
    pub class_info_rva: u32,
    pub dispatch_rvas: [u32; 4],
    pub callback_method_rva: u32,
}
impl ClassDeclaration {
    pub fn definition(&self) -> ClassDefinition<'_> {
        ClassDefinition {
            name: &self.name,
            base_name: &self.base_name,
            instance_size: self.instance_size,
            flags: self.flags,
            dispatch_keys: self.dispatch_rvas.map(u64::from),
            // The initializer defines the method and zero adjustment/flag data.
            // Its unwritten trailing stack bytes are not imported into Rust.
            callback_words: [u64::from(self.callback_method_rva), 0, 0],
        }
    }
}

pub struct ClassDeclarations {
    pub classes: Vec<ClassDeclaration>,
    pub startup_function_count: usize,
    /// Generated manager registration order, separate from hierarchy type IDs.
    pub component_pools: Vec<ComponentPoolDeclaration>,
    pub component_types: Vec<ComponentTypeDeclaration>,
    pub component_definitions: Vec<ComponentDefinition>,
}
impl ClassDeclarations {
    pub fn load(game_base: &Path) -> Result<Self, String> {
        let executable = game_base
            .parent()
            .ok_or("game asset folder has no parent")?
            .join("Dishonored2.exe");
        let bytes = fs::read(&executable).map_err(|e| format!("{}: {e}", executable.display()))?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if blake3::hash(bytes).to_hex().as_str() != IMAGE_HASH {
            return Err(
                "installed executable version is unsupported by the class metadata reader".into(),
            );
        }
        let image = Image::parse(bytes)?;
        let mut classes = Vec::new();
        let mut initializer_indices = HashMap::new();
        let mut class_addresses = HashSet::new();
        for section in image.sections.iter().filter(|s| s.executable) {
            let code = section.raw(bytes)?;
            for at in 0..code.len().saturating_sub(4) {
                if code[at] != 0xe8 {
                    continue;
                }
                let relative = i32::from_le_bytes(code[at + 1..at + 5].try_into().unwrap());
                let target = i64::from(section.rva) + at as i64 + 5 + i64::from(relative);
                if target != i64::from(REGISTER_RVA) {
                    continue;
                }
                let start = (u64::from(section.rva) + at as u64)
                    .checked_sub(CALL_OFFSET as u64)
                    .ok_or("class initializer begins outside image")?;
                let start = u32::try_from(start).map_err(|_| "class initializer RVA overflow")?;
                let raw = image.read(start, INITIALIZER_SIZE)?;
                let mut layout = [0; INITIALIZER_SIZE];
                layout.copy_from_slice(raw);
                for &(offset, size) in VARIABLES {
                    layout[offset..offset + size].fill(0);
                }
                if blake3::hash(&layout).to_hex().as_str() != LAYOUT_HASH {
                    return Err(format!(
                        "unsupported class initializer layout at {start:#x}"
                    ));
                }
                let rip = |offset, next| -> Result<u32, String> {
                    let displacement =
                        i32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
                    u32::try_from(i64::from(start) + next as i64 + i64::from(displacement))
                        .map_err(|_| "class initializer target is outside image".into())
                };
                let class_info_rva = rip(0x28, 0x2c)?;
                if !image.mapped(class_info_rva, 0x88) || !class_addresses.insert(class_info_rva) {
                    return Err(
                        "class declaration has a missing or repeated storage address".into(),
                    );
                }
                let dispatch_rvas = [
                    rip(0x7c, 0x80)?,
                    rip(0x71, 0x75)?,
                    rip(0x66, 0x6a)?,
                    rip(0x37, 0x3b)?,
                ];
                let callback_method_rva = rip(0x0d, 0x11)?;
                if dispatch_rvas
                    .iter()
                    .chain(std::iter::once(&callback_method_rva))
                    .any(|&rva| !image.executable_rva(rva))
                {
                    return Err("class declaration method is outside executable code".into());
                }
                let index = classes.len();
                if initializer_indices.insert(start, index).is_some() {
                    return Err("class initializer occurs more than once".into());
                }
                classes.push(ClassDeclaration {
                    name: image.string(rip(0x1b, 0x1f)?)?,
                    base_name: image.string(rip(0x14, 0x18)?)?,
                    instance_size: u32::from_le_bytes(raw[0x50..0x54].try_into().unwrap()),
                    flags: raw[0x88],
                    initializer_rva: start,
                    class_info_rva,
                    dispatch_rvas,
                    callback_method_rva,
                });
            }
        }
        let first = classes
            .first()
            .ok_or("installed executable has no recognized class declarations")?
            .initializer_rva;
        let first_va = image
            .base
            .checked_add(u64::from(first))
            .ok_or("initializer address overflow")?;
        let mut order = None;
        for section in image.sections.iter().filter(|s| !s.executable) {
            let data = section.raw(bytes)?;
            for at in (0..data.len().saturating_sub(7)).step_by(8) {
                if word(data, at, 8)? != first_va {
                    continue;
                }
                let mut start = at;
                while start >= 8 && image.executable_va(word(data, start - 8, 8)?) {
                    start -= 8;
                }
                let mut end = at;
                while end + 8 <= data.len() && image.executable_va(word(data, end, 8)?) {
                    end += 8;
                }
                let mut seen = HashSet::new();
                let mut indices = Vec::new();
                for offset in (start..end).step_by(8) {
                    let rva = word(data, offset, 8)?
                        .checked_sub(image.base)
                        .and_then(|v| u32::try_from(v).ok());
                    if let Some(index) = rva.and_then(|rva| initializer_indices.get(&rva).copied())
                    {
                        if !seen.insert(index) {
                            return Err("startup table repeats a class initializer".into());
                        }
                        indices.push(index);
                    }
                }
                if indices.len() == classes.len() {
                    if order.is_some() {
                        return Err("class startup table is ambiguous".into());
                    }
                    order = Some((indices, (end - start) / 8));
                }
            }
        }
        let (indices, startup_function_count) =
            order.ok_or("startup table does not contain every class initializer")?;
        let mut slots: Vec<_> = classes.into_iter().map(Some).collect();
        let classes: Vec<_> = indices
            .into_iter()
            .map(|index| slots[index].take().unwrap())
            .collect();
        let component_pools = components::read(&image, &classes)?;
        let component_types = component_types::read(&image, &component_pools, &classes)?;
        let component_definitions =
            component_definitions::read(&image, &component_types, &classes)?;
        Ok(Self {
            classes,
            startup_function_count,
            component_pools,
            component_types,
            component_definitions,
        })
    }

    /// Replay class declarations only, through the existing owned registry.
    /// Other startup subsystems keep their own initialization and name inputs.
    pub fn registry(&self) -> Result<ClassRegistry, String> {
        let mut registry = ClassRegistry::default();
        for class in &self.classes {
            registry.register(class.definition())?;
        }
        registry.initialize()?;
        Ok(registry)
    }
}

#[cfg(test)]
#[path = "class_metadata/tests.rs"]
mod tests;
