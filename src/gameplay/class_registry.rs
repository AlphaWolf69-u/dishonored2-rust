//! Class metadata registration, late base resolution and shared names (1990d0).
use super::{
    class_hierarchy::ClassHierarchy,
    name_pool::{NameId, NamePool},
};

pub struct ClassDefinition<'a> {
    pub name: &'a [u8],
    pub base_name: &'a [u8],
    pub instance_size: u32,
    pub flags: u8,
    /// Owned dispatch identities for class slots +10..+28. Registration copies
    /// them; invocation remains with the rewrite's typed implementations.
    pub dispatch_keys: [u64; 4],
    /// Raw callback descriptor at +30..+40, including uninterpreted words.
    pub callback_words: [u64; 3],
}
pub struct ClassMetadata {
    pub name: NameId,
    pub base_name: NameId,
    pub instance_size: u32,
    pub flags: u8,
    pub dispatch_keys: [u64; 4],
    pub callback_words: [u64; 3],
}
pub struct ClassRegistry {
    names: NamePool,
    root_name: Option<NameId>,
    metadata: Vec<ClassMetadata>,
    hierarchy: ClassHierarchy,
}
impl Default for ClassRegistry {
    fn default() -> Self {
        Self {
            names: NamePool::default(),
            root_name: None,
            metadata: Vec::new(),
            hierarchy: ClassHierarchy::new(Vec::new(), Vec::new()).expect("empty class hierarchy"),
        }
    }
}
impl ClassRegistry {
    /// Names are shared with other runtime owners; interning before registration
    /// can affect pointer-key order, exactly as in the game's global name pool.
    pub fn intern_name(&mut self, bytes: Option<&[u8]>) -> Result<NameId, String> {
        self.names.intern(bytes)
    }
    pub fn name_text(&self, name: NameId) -> Option<&[u8]> {
        self.names.text(name)
    }
    pub fn metadata(&self, class: usize) -> Option<&ClassMetadata> {
        self.metadata.get(class)
    }
    pub fn hierarchy(&self) -> &ClassHierarchy {
        &self.hierarchy
    }
    pub fn initialize(&mut self) -> Result<bool, String> {
        self.hierarchy.initialize()
    }

    fn find_name(&self, name: NameId) -> Option<usize> {
        if !self.hierarchy.is_initialized() {
            return self
                .hierarchy
                .registration_order()
                .iter()
                .copied()
                .find(|&index| self.metadata[index].name == name);
        }
        // The initialized table is intentionally not refreshed by a later
        // registration. Its signed pointer-difference search is 1990d0's.
        let table = self.hierarchy.by_name();
        let mut low = 0i32;
        let mut high = table.len() as i32 - 1;
        while low <= high {
            let middle = (low + high) / 2;
            let index = table[middle as usize];
            let difference = self.metadata[index]
                .name
                .identity()
                .wrapping_sub(name.identity()) as isize;
            if difference == 0 {
                return Some(index);
            }
            if difference > 0 {
                high = middle - 1;
            } else {
                low = middle + 1;
            }
        }
        None
    }

    /// Complete1990d0 logical registration. Root naming, parent lookup, unresolved
    /// child repair and equal-key insertion preserve the source's distinct rules.
    pub fn register(&mut self, definition: ClassDefinition<'_>) -> Result<usize, String> {
        let root = match self.root_name {
            Some(name) => name,
            None => {
                let name = self.names.intern(Some(b"idClass"))?;
                self.root_name = Some(name);
                name
            }
        };
        let requested_base = self.names.intern(Some(definition.base_name))?;
        let name = self.names.intern(Some(definition.name))?;
        let base_name = if name == root {
            self.names.intern(Some(b""))?
        } else {
            requested_base
        };
        // Even the root branch resolves using the original parent argument,
        // independently of the stored empty parent name (1991a3/199220).
        let base = self.find_name(requested_base);
        let position = self
            .hierarchy
            .registration_order()
            .iter()
            .position(|&index| name < self.metadata[index].name)
            .unwrap_or(self.metadata.len());
        let index = self.hierarchy.insert_class(base, position)?;
        self.metadata.push(ClassMetadata {
            name,
            base_name,
            instance_size: definition.instance_size,
            flags: definition.flags,
            dispatch_keys: definition.dispatch_keys,
            callback_words: definition.callback_words,
        });
        for at in 0..self.hierarchy.registration_order().len() {
            let other = self.hierarchy.registration_order()[at];
            if other != index
                && self.hierarchy.node(other).unwrap().base.is_none()
                && self.metadata[other].name != root
                && self.metadata[other].base_name == name
            {
                self.hierarchy.set_base(other, index);
            }
        }
        Ok(index)
    }
}

#[cfg(test)]
#[path = "class_registry/tests.rs"]
mod tests;
