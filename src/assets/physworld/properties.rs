//! Retained shape properties and ae3b50's last-matching-entry lookup.
use super::{le32, slice, PhysWorld};

#[derive(Clone, Copy, Debug)]
pub struct ShapeProperty {
    pub object: usize,
    pub key: u16,
    pub flags: u16,
}

pub fn property_object(properties: &[ShapeProperty], key: u16) -> Option<usize> {
    properties
        .iter()
        .rev()
        .find(|property| property.key == key)
        .and_then(|property| (property.object != 0).then_some(property.object))
}

impl PhysWorld {
    pub fn shape_properties(
        &self,
        bytes: &[u8],
        shape: usize,
    ) -> Result<Vec<ShapeProperty>, String> {
        let item = self
            .items
            .get(shape)
            .ok_or("shape property owner missing")?;
        let data = self.item_at(bytes, shape, 0)?;
        let owner = le32(data, self.field_offset(item.type_index, "properties")?)? as usize;
        if owner == 0 {
            return Ok(Vec::new());
        }
        let owner_type = self
            .items
            .get(owner)
            .ok_or("shape properties missing")?
            .type_index;
        let data = self.item_at(bytes, owner, 0)?;
        let entries = le32(data, self.field_offset(owner_type, "entries")?)? as usize;
        if entries == 0 {
            return Ok(Vec::new());
        }
        let array = self
            .items
            .get(entries)
            .ok_or("shape property entries missing")?;
        let object = self.field_offset(array.type_index, "object")?;
        let key = self.field_offset(array.type_index, "key")?;
        let flags = self.field_offset(array.type_index, "flags")?;
        (0..array.count)
            .map(|index| {
                let entry = self.item_at(bytes, entries, index)?;
                let short = |at| -> Result<u16, String> {
                    Ok(u16::from_le_bytes(slice(entry, at, 2)?.try_into().unwrap()))
                };
                Ok(ShapeProperty {
                    object: le32(entry, object)? as usize,
                    key: short(key)?,
                    flags: short(flags)?,
                })
            })
            .collect()
    }

    pub fn shape_mass_properties(
        &self,
        bytes: &[u8],
        shape: usize,
    ) -> Result<Option<super::mass::PrincipalMassProperties>, String> {
        let Some(reference) = property_object(&self.shape_properties(bytes, shape)?, 0xf100) else {
            return Ok(None);
        };
        let item = self
            .items
            .get(reference)
            .ok_or("shape mass property object missing")?;
        let data = self.item_at(bytes, reference, 0)?;
        let at = self.field_offset(item.type_index, "compressedMassProperties")?;
        super::mass::decode_principal(slice(data, at, 32)?).map(Some)
    }
}

#[cfg(test)]
#[path = "properties/tests.rs"]
mod tests;
