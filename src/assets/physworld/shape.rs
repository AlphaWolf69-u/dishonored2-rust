//! Common shape fields from the file schema, whose offsets differ from the
//! expanded 64-bit runtime object (for example, key bits at13 instead of23).
use super::{float, PhysWorld};

#[derive(Clone, Copy, Debug)]
pub struct ShapeHeader {
    pub flags: u16,
    pub shape_type: u8,
    pub key_bits: u8,
    pub dispatch_type: u8,
    pub radius: f32,
    pub user_data: u64,
}

impl PhysWorld {
    pub fn shape_header(&self, bytes: &[u8], index: usize) -> Result<ShapeHeader, String> {
        let item = self.items.get(index).ok_or("shape item missing")?;
        let data = self.item_at(bytes, index, 0)?;
        let offset = |name| self.field_offset(item.type_index, name);
        let short = |name| -> Result<u16, String> {
            let at = offset(name)?;
            Ok(u16::from_le_bytes(
                data.get(at..at + 2)
                    .ok_or("shape short outside item")?
                    .try_into()
                    .unwrap(),
            ))
        };
        let byte = |name| -> Result<u8, String> {
            data.get(offset(name)?)
                .copied()
                .ok_or("shape byte outside item".into())
        };
        let user = offset("userData")?;
        Ok(ShapeHeader {
            flags: short("flags")?,
            shape_type: byte("type")?,
            key_bits: byte("numShapeKeyBits")?,
            dispatch_type: byte("dispatchType")?,
            radius: float(data, offset("convexRadius")?)?,
            user_data: u64::from_le_bytes(
                data.get(user..user + 8)
                    .ok_or("shape user data outside item")?
                    .try_into()
                    .unwrap(),
            ),
        })
    }
}
