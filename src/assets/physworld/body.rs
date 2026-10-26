//! Authored body creation data. Serialized array positions are not runtime IDs.
use super::{float, le32, slice, PhysWorld};

/// hknpMassDistribution, including all authored padding/vector W lanes.
#[derive(Clone, Copy, Debug)]
pub struct MassDistribution {
    pub center_of_mass_and_volume: [f32; 4],
    pub major_axis_space: [f32; 4],
    pub inertia_tensor: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct BodyInfo {
    pub shape: usize,
    pub flags: u32,
    pub collision_control: u16,
    pub filter_info: u32,
    pub material_id: u16,
    pub quality_id: u8,
    pub name: usize,
    pub user_data: u64,
    pub motion_type: u8,
    pub position: [f32; 4],
    pub orientation: [f32; 4],
    pub linear_velocity: [f32; 4],
    pub angular_velocity: [f32; 4],
    pub mass: f32,
    pub mass_distribution: usize,
    pub motion_properties_id: u16,
    pub reserved_body_id: u32,
    pub reserved_motion_id: u32,
    pub collision_look_ahead_distance: f32,
    pub local_frame: usize,
    /// Present on bodyCinfoWithAttachment; preserves the signed source index.
    pub attached_body: Option<i32>,
}

/// ca65f0 initializes body creation data independently of serialized assets.
impl Default for BodyInfo {
    fn default() -> Self {
        Self {
            shape: 0,
            flags: 0,
            collision_control: 0,
            filter_info: 0,
            material_id: 0,
            quality_id: u8::MAX,
            name: 0,
            user_data: 0,
            motion_type: 0,
            position: [0.; 4],
            orientation: [0., 0., 0., 1.],
            linear_velocity: [0.; 4],
            angular_velocity: [0.; 4],
            mass: -1.,
            mass_distribution: 0,
            motion_properties_id: u16::MAX,
            reserved_body_id: 0x00ff_ffff,
            reserved_motion_id: 0x7fff_ffff,
            collision_look_ahead_distance: 0.,
            local_frame: 0,
            attached_body: None,
        }
    }
}

impl PhysWorld {
    /// Read the physics system's creation array, retaining its source order,
    /// inherited fields, raw four-lane values and unresolved item references.
    pub fn body_infos(&self, bytes: &[u8]) -> Result<Vec<BodyInfo>, String> {
        let root = self.items.get(1).ok_or("physics system root missing")?;
        let data = self.item_at(bytes, 1, 0)?;
        let reference = le32(data, self.field_offset(root.type_index, "bodyCinfos")?)? as usize;
        if reference == 0 {
            return Ok(Vec::new());
        }
        let item = self
            .items
            .get(reference)
            .ok_or("body creation array missing")?;
        let ty = item.type_index;
        // Resolve the declared layout once, including inherited fields. The
        // installed schema supplies offsets rather than a fixed176-byte cast.
        let fields = [
            "shape",
            "flags",
            "collisionCntrl",
            "collisionFilterInfo",
            "materialId",
            "qualityId",
            "name",
            "userData",
            "motionType",
            "position",
            "orientation",
            "linearVelocity",
            "angularVelocity",
            "mass",
            "massDistribution",
            "motionPropertiesId",
            "reservedBodyId",
            "reservedMotionId",
            "collisionLookAheadDistance",
            "localFrame",
        ];
        let offsets: Vec<usize> = fields
            .into_iter()
            .map(|name| self.field_offset(ty, name))
            .collect::<Result<_, _>>()?;
        let attachment = self.field_offset(ty, "attachedBody").ok();
        let mut bodies = Vec::with_capacity(item.count);
        for index in 0..item.count {
            let data = self.item_at(bytes, reference, index)?;
            let word = |field| le32(data, offsets[field]);
            let short = |field| {
                Ok::<_, String>(u16::from_le_bytes(
                    slice(data, offsets[field], 2)?.try_into().unwrap(),
                ))
            };
            let vector = |field| -> Result<[f32; 4], String> {
                let at = offsets[field];
                Ok([
                    float(data, at)?,
                    float(data, at + 4)?,
                    float(data, at + 8)?,
                    float(data, at + 12)?,
                ])
            };
            bodies.push(BodyInfo {
                shape: word(0)? as usize,
                flags: word(1)?,
                collision_control: short(2)?,
                filter_info: word(3)?,
                material_id: short(4)?,
                quality_id: slice(data, offsets[5], 1)?[0],
                name: word(6)? as usize,
                user_data: u64::from_le_bytes(slice(data, offsets[7], 8)?.try_into().unwrap()),
                motion_type: slice(data, offsets[8], 1)?[0],
                position: vector(9)?,
                orientation: vector(10)?,
                linear_velocity: vector(11)?,
                angular_velocity: vector(12)?,
                mass: float(data, offsets[13])?,
                mass_distribution: word(14)? as usize,
                motion_properties_id: short(15)?,
                reserved_body_id: word(16)?,
                reserved_motion_id: word(17)?,
                collision_look_ahead_distance: float(data, offsets[18])?,
                local_frame: word(19)? as usize,
                attached_body: attachment
                    .map(|at| le32(data, at).map(|v| v as i32))
                    .transpose()?,
            });
        }
        Ok(bodies)
    }
}

#[cfg(test)]
#[path = "body/tests.rs"]
mod tests;
