//! Melee component declarations and the authored WeaponActiveFrames callback.
//! Constructors 633c70/62db00 and track callback84e0c0; immutable damage
//! declarations remain leased from their canonical resource owner.
use crate::{
    archive::Catalog,
    entities::{
        Value,
        scene_motion::{atom, child},
    },
    gameplay::damage::number,
};
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;

#[derive(Debug)]
pub struct Declaration {
    pub damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub focused_damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub blink_damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub focused_blink_damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub bloodthirsty_damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub fatality_damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub material: Option<String>,
    pub focused_material: Option<String>,
    pub parry_material: Option<String>,
    pub whoosh_sound: Option<String>,
    pub dimensions: Vec3,
    pub npc_focus: FocusShape,
    pub world_focus: FocusShape,
    pub shape_step_back: f32,
    pub focus_step_back: f32,
    pub impulse: f32,
}
impl Declaration {
    pub fn acquire(catalog: &Catalog, fields: &[(String, Value)]) -> Result<Arc<Self>, String> {
        let path = |key| {
            atom(fields, key)
                .filter(|path| !path.is_empty() && *path != "NULL")
                .map(str::to_owned)
        };
        let damage = |key| {
            path(key)
                .map(|path| crate::assets::damage::Declaration::acquire(catalog, &path))
                .transpose()
        };
        let dimensions = child(fields, "m_phantomShapeDimensions");
        Ok(Arc::new(Self {
            damage: damage("m_damage")?,
            focused_damage: damage("m_focusedDamage")?,
            blink_damage: damage("m_blinkAttackDamage")?,
            focused_blink_damage: damage("m_focusedBlinkAttackDamage")?,
            bloodthirsty_damage: damage("m_bloodthirstyDamage")?,
            fatality_damage: damage("m_genericFatalityDamage")?,
            material: path("m_weaponMaterial"),
            focused_material: path("m_focusedMaterial"),
            parry_material: path("m_parryMaterial"),
            whoosh_sound: path("m_whooshSoundEvent"),
            dimensions: Vec3::new(
                number(dimensions, "x", f32::from_bits(0x4009999a))?,
                number(dimensions, "y", 0.1)?,
                number(dimensions, "z", f32::from_bits(0x3e0f5c29))?,
            ),
            npc_focus: FocusShape::read(child(fields, "m_npcFocusShape"))?,
            world_focus: FocusShape::read(child(fields, "m_worldFocusShape"))?,
            shape_step_back: number(fields, "m_shapeCastStepBackDistance", 0.8)?,
            focus_step_back: number(fields, "m_focusStepBackDistance", 0.3)?,
            impulse: number(fields, "m_physicImpulse", 50.0)?,
        }))
    }
    pub fn materials(&self) -> impl Iterator<Item = &str> {
        [&self.material, &self.focused_material, &self.parry_material]
            .into_iter()
            .flatten()
            .map(String::as_str)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FocusShape {
    pub extents: Vec3,
    pub cone_angle: f32,
    pub cone_height: f32,
    pub height_scale: f32,
}
impl FocusShape {
    ///62f900: actual focus cone plus635a00's oriented focus box.
    pub fn contains(
        self,
        origin: Vec3,
        eye: Vec3,
        camera: Mat3,
        step_back: f32,
        point: Vec3,
    ) -> bool {
        let base = origin + (eye - origin) * self.height_scale - camera.x_axis * step_back;
        let direction = (point - base).normalize_or_zero();
        if camera.x_axis.dot(direction) <= (self.cone_angle * 0.5).to_radians().cos() {
            return false;
        }
        let local = camera.transpose() * (point - base - camera.x_axis * self.extents.x);
        local.abs().cmple(self.extents).all()
    }
    /// 62db00 defaults and reflected1416ff540/1416c3de0.
    fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let extents = child(child(fields, "m_box"), "m_extents");
        let cone = child(fields, "m_cone");
        Ok(Self {
            extents: Vec3::new(
                number(extents, "x", 1.5)?,
                number(extents, "y", 0.4)?,
                number(extents, "z", 1.)?,
            ),
            cone_angle: number(cone, "m_angle", 75.)?,
            cone_height: number(cone, "m_height", 1.6)?,
            height_scale: number(fields, "m_heightScale", 0.75)?,
        })
    }
}

pub struct SweepFrame {
    pub from: Vec3,
    pub to: Vec3,
    pub blade_direction: Vec3,
    pub shape_length: f32,
}

/// 634990 and634ce0, using62f290's actual player focus OBB. This prepares
/// the original swept shape; no blade contact is replaced by a point ray.
pub fn sweep_frame(
    decl: &Declaration,
    previous: crate::gameplay::reference_frame::ReferenceFrame,
    current: crate::gameplay::reference_frame::ReferenceFrame,
    player_origin: Vec3,
    eye: Vec3,
    camera: Mat3,
    forward_attack: bool,
) -> SweepFrame {
    let start = Vec3::from_array(previous.position);
    let delta = Vec3::from_array(current.position) - start;
    let distance = delta.length();
    let direction = delta.normalize_or_zero();
    let a = Vec3::from_array(previous.basis[0]).dot(direction) * decl.dimensions.x;
    let b = Vec3::from_array(current.basis[0]).dot(direction) * decl.dimensions.x;
    let blade_direction = if forward_attack {
        Vec3::from_array(current.basis[0])
    } else {
        camera.x_axis
    };
    let mut from = start + direction * a.min(b).min(0.) - decl.shape_step_back * blade_direction;
    let mut to =
        start + direction * distance.max(a.max(b)) - decl.shape_step_back * blade_direction;
    let shape = decl.npc_focus;
    let center = player_origin + (eye - player_origin) * shape.height_scale
        - camera.x_axis * decl.focus_step_back
        + camera.x_axis * shape.extents.x;
    let ray_box = |position: Vec3, ray: Vec3| -> Option<(f32, f32)> {
        let position = camera.transpose() * (position - center);
        let ray = camera.transpose() * ray;
        let mut entry = f32::NEG_INFINITY;
        let mut exit = f32::INFINITY;
        for axis in 0..3 {
            if ray[axis] == 0. {
                if position[axis].abs() > shape.extents[axis] {
                    return None;
                }
            } else {
                let a = (-shape.extents[axis] - position[axis]) / ray[axis];
                let b = (shape.extents[axis] - position[axis]) / ray[axis];
                entry = entry.max(a.min(b));
                exit = exit.min(a.max(b));
                if exit < entry {
                    return None;
                }
            }
        }
        Some((entry, exit))
    };
    let segment = (to - from).normalize_or_zero();
    let length = (to - from).length();
    if let Some((entry, exit)) = ray_box(from, segment) {
        if entry > 0. {
            if entry < length {
                from += segment * entry;
            }
        } else if exit < length {
            to = from + segment * exit;
        }
    }
    let mut shape_length = decl.dimensions.x;
    if let Some((entry, exit)) = ray_box(from, blade_direction) {
        if entry > 0. {
            if entry < decl.dimensions.x {
                shape_length = decl.dimensions.x - entry;
            }
        } else if exit < decl.dimensions.x {
            shape_length = exit;
        }
    }
    if let Some((entry, exit)) = ray_box(to, blade_direction) {
        if entry > 0. {
            let remaining = decl.dimensions.x - entry;
            if remaining > 0. && shape_length < remaining {
                shape_length = remaining.min(decl.dimensions.x);
            }
        } else if shape_length < exit {
            shape_length = exit.min(decl.dimensions.x);
        }
    }
    SweepFrame {
        from,
        to,
        blade_direction,
        shape_length,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ActiveFrames {
    pub slot: i32,
    pub forward_attack: bool,
    pub heavy_block: bool,
    pub damage_factor: f32,
}
impl ActiveFrames {
    /// Reflected fields1415c5b10, defaults188780.84e0c0 maps enum0 to
    /// equipment slots1/3, enum1 to0/2, and enum2 to4/5.
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let slot = match atom(fields, "m_slot").unwrap_or("WEAPON_SLOT_RIGHT") {
            "WEAPON_SLOT_LEFT" => 0,
            "WEAPON_SLOT_RIGHT" => 1,
            "WEAPON_SLOT_HEAD" => 2,
            value => value
                .parse()
                .map_err(|_| format!("unknown weapon active-frame slot {value}"))?,
        };
        let boolean = |key| match atom(fields, key) {
            None | Some("false" | "0") => Ok(false),
            Some("true" | "1") => Ok(true),
            Some(value) => Err(format!("invalid weapon active-frame {key}={value}")),
        };
        Ok(Self {
            slot,
            forward_attack: boolean("m_forwardAttack")?,
            heavy_block: boolean("m_triggerHeavyBlock")?,
            damage_factor: number(fields, "m_damageFactor", 1.0)?,
        })
    }
    pub fn targets(self, slot: u8) -> bool {
        match self.slot {
            0 => matches!(slot, 1 | 3),
            1 => matches!(slot, 0 | 2),
            2 => matches!(slot, 4 | 5),
            _ => false,
        }
    }
}
