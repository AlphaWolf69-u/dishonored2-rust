//! The authored 32-byte float BVH node layout used by compound shape queries.
#[derive(Clone, Copy, Debug)]
pub enum CompoundChildren {
    Branch { left: u16, right: u16 },
    Instance(u16),
}
#[derive(Clone, Copy, Debug)]
pub struct CompoundNode {
    pub bounds: [[f32; 3]; 2],
    pub children: CompoundChildren,
}

#[derive(Debug)]
pub struct CompoundInstance {
    /// Raw columns retain the packed flags in column zero's W component.
    pub transform: [[f32; 4]; 4],
    pub flags: u32,
    pub scale: [f32; 4],
    pub shape: usize,
    pub shape_tag: u16,
}
