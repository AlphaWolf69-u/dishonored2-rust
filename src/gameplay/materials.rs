//! Material records and library registration (c9c210/c9c350/c9c940/c9ce00).
//! Names and declaration handles have Rust ownership; numeric record words
//! retain the source's bitwise comparison, including the unused free records.
use super::signal::Signal;
use crate::assets::{
    entities::physics_material::PhysicsMaterial, physics_material::MaterialConstants,
};
use std::sync::Arc;
#[cfg(test)]
mod tests;

/// The level's declaration cache and shared runtime library. Archive names
/// resolve once to stable declaration handles; body IDs always refer to this
/// same library, including materials applied later by water startup.
pub(crate) struct MaterialOwner<S, C> {
    pub library: MaterialLibrary<S, C>,
    declarations: Vec<PhysicsMaterial>,
    names: std::collections::HashMap<String, usize>,
}
impl<S: Copy + Eq, C: Copy + Eq> MaterialOwner<S, C> {
    pub fn new(constants: Arc<MaterialConstants>, capacity: usize) -> Result<Self, String> {
        Ok(Self {
            library: MaterialLibrary::new(constants, capacity)?,
            declarations: Vec::new(),
            names: Default::default(),
        })
    }
    pub fn declaration(
        &mut self,
        catalog: &crate::assets::archive::Catalog,
        name: &str,
    ) -> Result<usize, String> {
        if let Some(&handle) = self.names.get(name) {
            return Ok(handle);
        }
        let material = crate::assets::entities::physics_material::physics_material(name, |path| {
            catalog
                .read(path)
                .map(|bytes| {
                    bytes.map(|b| crate::assets::entities::declaration_text(&b).into_owned())
                })
                .map_err(|e| e.to_string())
        })?;
        let handle = self.declarations.len();
        self.declarations.push(material);
        self.names.insert(name.to_owned(), handle);
        Ok(handle)
    }
    pub fn get(&self, handle: usize) -> Option<&PhysicsMaterial> {
        self.declarations.get(handle)
    }
    pub fn resolve(
        &mut self,
        handle: usize,
        dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16),
    ) -> Result<u16, String> {
        let material = self
            .declarations
            .get(handle)
            .ok_or("material declaration is missing")?;
        self.library.resolve(material, handle, dispatch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaterialRecord {
    pub name: Option<Arc<str>>,
    /// Source+28..67, including the stable declaration handle in the last8 bytes.
    pub words: [u32; 16],
    pub tail: [u8; 8],
}
impl MaterialRecord {
    pub fn new(constants: &MaterialConstants) -> Self {
        let mut value = Self {
            name: None,
            words: [0; 16],
            tail: [0; 8],
        };
        value.set_byte(9, constants.pack_distance(constants.maximum_distance));
        value.set_short(10, 0x3f00);
        value.set_short(12, 0x3f00);
        value.set_short(16, 0x0201);
        value.set_short(18, constants.contact_tolerance);
        value.words[5] = 0x7f7f_ffee;
        value.words[6] = 0x3f80_0000;
        value.set_short(30, 0x3f80);
        value.set_short(48, constants.material_limit);
        value
    }
    fn byte(&self, offset: usize) -> u8 {
        (self.words[offset / 4] >> ((offset % 4) * 8)) as u8
    }
    fn short(&self, offset: usize) -> u16 {
        (self.words[offset / 4] >> ((offset % 4) * 8)) as u16
    }
    fn set_byte(&mut self, offset: usize, value: u8) {
        let shift = (offset % 4) * 8;
        self.words[offset / 4] =
            (self.words[offset / 4] & !(0xff << shift)) | (u32::from(value) << shift);
    }
    fn set_short(&mut self, offset: usize, value: u16) {
        let shift = (offset % 4) * 8;
        self.words[offset / 4] =
            (self.words[offset / 4] & !(0xffff << shift)) | (u32::from(value) << shift);
    }
    pub fn from_declaration(
        constants: &MaterialConstants,
        material: &PhysicsMaterial,
        handle: usize,
    ) -> Result<Self, String> {
        let mut value = Self::new(constants);
        value.name = Some(Arc::from(material.name.split('\0').next().unwrap()));
        if material.flags & 0x20 != 0 {
            value.set_byte(8, (material.flags | 0xbf) >> 6);
            value.set_byte(9, constants.pack_distance(constants.trigger_distance));
        } else {
            value.set_short(10, (material.dynamic_friction.to_bits() >> 16) as u16);
            value.set_short(12, (material.static_friction.to_bits() >> 16) as u16);
            value.set_short(14, (material.restitution.to_bits() >> 16) as u16);
            value.set_byte(16, 2);
        }
        let resource = (handle as u64)
            .checked_add(1)
            .ok_or("material declaration handle overflow")?;
        value.words[14] = resource as u32;
        value.words[15] = (resource >> 32) as u32;
        Ok(value)
    }
    pub fn sanitize(&mut self, constants: &MaterialConstants) {
        let mut flags = 0;
        if self.short(14) & 0x7fff != 0 {
            flags |= 0x10_0000;
        }
        if self.byte(8) != 0 {
            flags |= 0x20_0000;
        }
        if f32::from_bits(self.words[5]) < constants.maximum_distance {
            flags |= 0x40_0000;
        }
        if self.byte(28) == 2 || self.words[1] & 0x80_0000 != 0 {
            flags |= 0x80_0000;
        }
        let surface = f32::from_bits(u32::from(self.short(32)) << 16);
        // UCOMISS/JZ also takes the zero branch for unordered NaNs.
        if surface != 0. && !surface.is_nan() {
            flags |= 0x100_0000;
        }
        if self.words[10] != 0 || self.words[11] != 0 {
            flags |= 0x200_0000;
        }
        self.words[1] = (self.words[1] & 0xfc0f_ffff) | flags;
    }
}

pub(crate) struct MaterialLibrary<S, C> {
    constants: Arc<MaterialConstants>,
    records: Vec<MaterialRecord>,
    free_head: i32,
    pub added: Signal<S, C>,
}
impl<S: Copy + Eq, C: Copy + Eq> MaterialLibrary<S, C> {
    pub fn new(constants: Arc<MaterialConstants>, capacity: usize) -> Result<Self, String> {
        let mut value = Self {
            constants,
            records: Vec::new(),
            free_head: -1,
            added: Signal::default(),
        };
        value.grow(capacity)?;
        for kind in 0..4 {
            let mut preset = MaterialRecord::new(&value.constants);
            preset.words[0] = 1;
            if kind != 0 {
                preset.name = Some(Arc::from(match kind {
                    1 => "[Preset] Trigger (broad phase)",
                    2 => "[Preset] Trigger (narrow phase)",
                    _ => "[Preset] Trigger (contact solver)",
                }));
                preset.set_byte(8, kind);
                preset.set_byte(
                    9,
                    value.constants.pack_distance(if kind == 3 {
                        0.
                    } else {
                        f32::from_bits(0x7f7f_ffee)
                    }),
                );
            }
            value.add(preset, |_, _, _, _| {
                unreachable!("constructor signals have no subscribers")
            })?;
        }
        Ok(value)
    }
    pub fn get(&self, id: u16) -> Option<&MaterialRecord> {
        self.records.get(usize::from(id))
    }
    pub fn records(&self) -> &[MaterialRecord] {
        &self.records
    }
    pub fn free_head(&self) -> i32 {
        self.free_head
    }
    fn grow(&mut self, increment: usize) -> Result<(), String> {
        let old = self.records.len();
        let count = old
            .checked_mul(2)
            .ok_or("material table capacity overflow")?
            .max(usize::from(old == 0))
            .max(
                old.checked_add(increment)
                    .ok_or("material table capacity overflow")?,
            );
        if count > 0x10000 {
            return Err("material table exceeds16-bit IDs".into());
        }
        self.records
            .try_reserve(count - old)
            .map_err(|e| format!("material table allocation failed: {e}"))?;
        for index in old..count {
            let mut record = MaterialRecord::new(&self.constants);
            record.words[5] = (-1f32).to_bits();
            record.words[0] = if index + 1 == count {
                self.free_head as u32
            } else {
                (index + 1) as u32
            };
            self.records.push(record);
        }
        self.free_head = old as i32;
        Ok(())
    }
    pub fn find_name(&self, name: Option<&str>) -> Option<u16> {
        self.records
            .iter()
            .position(|record| {
                // COMISS(0,value)/JA skips only strictly negative values.
                // Unordered NaNs remain eligible for the name collision test.
                !(f32::from_bits(record.words[5]) < 0.) && record.name.as_deref() == name
            })
            .map(|i| i as u16)
    }
    pub fn add(
        &mut self,
        mut record: MaterialRecord,
        mut dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16),
    ) -> Result<u16, String> {
        record.sanitize(&self.constants);
        if record.words[0] == 0 {
            if let Some(index) = self
                .records
                .iter()
                .position(|entry| entry.name == record.name && entry.words == record.words)
            {
                self.records[index].tail[0] = 1;
                return Ok(index as u16);
            }
        }
        if record.name.as_deref().is_some_and(|name| !name.is_empty())
            && self.find_name(record.name.as_deref()).is_some()
        {
            return Ok(u16::MAX);
        }
        if self.free_head < 0 {
            self.grow(8)?;
        }
        let id = self.free_head as u16;
        let target = self
            .records
            .get_mut(usize::from(id))
            .ok_or("material free slot is missing")?;
        self.free_head = target.words[0] as i32;
        *target = record;
        self.added.emit(&id, |signal, subscriber, callback, &id| {
            dispatch(signal, subscriber, callback, id)
        });
        Ok(id)
    }
    ///092f80 composes constructor/conversion with the same shared registry.
    pub fn resolve(
        &mut self,
        material: &PhysicsMaterial,
        handle: usize,
        dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16),
    ) -> Result<u16, String> {
        self.add(
            MaterialRecord::from_declaration(&self.constants, material, handle)?,
            dispatch,
        )
    }
}
