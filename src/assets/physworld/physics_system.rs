//! `generated/physsystem/*.hkx` hknpPhysicsSystemData: body cinfos with their
//! shape mass properties and hkpRagdollConstraintData constraint atoms.
use super::{float, le32, mass, parse_embedded_havok, slice, PhysWorld};
use crate::gameplay::constraint_solver::atom_stack::{AngularLimit, AtomStack, FrictionAtom};

#[derive(Clone, Debug)]
pub struct SystemBody {
    pub name: String,
    /// Model-space body transform (hknpBodyCinfo position/orientation).
    pub position: [f32; 4],
    pub orientation: [f32; 4],
    pub motion_type: u8,
    /// Shape mass properties (property key 0xf100) with the cinfo mass set.
    pub mass: mass::PrincipalMassProperties,
}

#[derive(Clone, Debug)]
pub struct SystemConstraint {
    pub name: String,
    pub body_a: usize,
    pub body_b: usize,
    pub atoms: AtomStack,
}

#[derive(Clone, Debug)]
pub struct PhysicsSystem {
    pub name: String,
    pub bodies: Vec<SystemBody>,
    pub constraints: Vec<SystemConstraint>,
}

fn string(world: &PhysWorld, bytes: &[u8], reference: u32) -> Result<String, String> {
    if reference == 0 {
        return Ok(String::new());
    }
    let item = world.items.get(reference as usize).ok_or("invalid string reference")?;
    let start = world.data_start + item.offset;
    let data = slice(bytes, start, item.count)?;
    Ok(String::from_utf8_lossy(data.split(|b| *b == 0).next().unwrap_or(&[])).into_owned())
}

fn array(world: &PhysWorld, reference: u32) -> Result<(usize, usize), String> {
    if reference == 0 {
        return Ok((0, 0));
    }
    let item = world.items.get(reference as usize).ok_or("invalid array reference")?;
    Ok((reference as usize, item.count))
}

/// hkpRagdollConstraintData::Atoms from the tag layout (ragdoll motors are
/// 0x50 bytes here and 0x60 in the 64-bit runtime layout).
fn ragdoll_atoms(b: &[u8]) -> Result<AtomStack, String> {
    let f = |at: usize| float(b, at);
    let byte = |at: usize| b.get(at).copied().ok_or("truncated ragdoll atoms");
    let transform = |at: usize| -> Result<[[f32; 4]; 4], String> {
        let mut out = [[0.; 4]; 4];
        for (c, column) in out.iter_mut().enumerate() {
            for (i, v) in column.iter_mut().enumerate() {
                *v = f(at + 16 * c + 4 * i)?;
            }
        }
        Ok(out)
    };
    let limit = |at: usize, cone: bool| -> Result<AngularLimit, String> {
        Ok(AngularLimit {
            enabled: byte(at + 2)? != 0,
            axis: byte(at + 3)?,
            reference: byte(at + 4)?,
            mode: if cone { byte(at + 5)? } else { 0 },
            memory_offset: if cone { u16::from_le_bytes([byte(at + 6)?, byte(at + 7)?]) } else { 0 },
            minimum: f(at + 8)?,
            maximum: f(at + 12)?,
            tau: f(at + 16)?,
            damping: f(at + 20)?,
        })
    };
    let types = [(0, 2), (0x90, 0x17), (0xa0, 0x13), (0xf0, 0x11), (0x100, 0xf), (0x120, 0x10), (0x140, 0x10), (0x160, 5)];
    for (at, kind) in types {
        let found = u16::from_le_bytes([byte(at)?, byte(at + 1)?]);
        if found != kind {
            return Err(format!("ragdoll atom at {at:#x} has type {found:#x}, expected {kind:#x}"));
        }
    }
    if byte(0xa2)? != 0 {
        return Err("enabled ragdoll motors are not supported".into());
    }
    Ok(AtomStack {
        transform_a: transform(0x10)?,
        transform_b: transform(0x50)?,
        stabilization: (byte(0x92)? != 0).then(|| [f(0x94).unwrap(), f(0x98).unwrap(), f(0x9c).unwrap()]),
        disabled_motors: true,
        friction: Some(FrictionAtom {
            active: byte(0xf2)? != 0 && f(0xf8)? != 0.,
            first_axis: byte(0xf3)?,
            axes: byte(0xf4)?,
            torque: f(0xf8)?,
        }),
        twist: Some(limit(0x100, false)?),
        cone: Some(limit(0x120, true)?),
        planes: Some(limit(0x140, true)?),
        limited: byte(0x165)? != 0,
        breach_impulse: f(0x168)?,
        inertia_stabilization: f(0x16c)?,
    })
}

impl PhysWorld {
    fn property_object(&self, bytes: &[u8], properties: u32, key: u16) -> Result<Option<usize>, String> {
        if properties == 0 {
            return Ok(None);
        }
        let item = self.items.get(properties as usize).ok_or("invalid properties reference")?;
        let data = self.item_at(bytes, properties as usize, 0)?;
        let entries = le32(data, self.field_offset(item.type_index, "entries")?)?;
        let (entries, count) = array(self, entries)?;
        for k in 0..count {
            let entry = self.item_at(bytes, entries, k)?;
            if u16::from_le_bytes([entry[4], entry[5]]) == key {
                return Ok(Some(le32(entry, 0)? as usize));
            }
        }
        Ok(None)
    }
}

pub fn parse(bytes: &[u8]) -> Result<PhysicsSystem, String> {
    let world = parse_embedded_havok(bytes)?;
    let root = world.items.get(1).ok_or("physics system root missing")?;
    if world.types[root.type_index].name != "hknpPhysicsSystemData" {
        return Err("root is not hknpPhysicsSystemData".into());
    }
    let data = world.item_at(bytes, 1, 0)?;
    let field = |name: &str| world.field_offset(root.type_index, name).and_then(|at| le32(data, at));
    let name = string(&world, bytes, field("name")?)?;
    let (bodies_item, body_count) = array(&world, field("bodyCinfos")?)?;
    let mut bodies = Vec::with_capacity(body_count);
    for k in 0..body_count {
        let b = world.item_at(bytes, bodies_item, k)?;
        let vector = |at: usize| -> Result<[f32; 4], String> {
            Ok([float(b, at)?, float(b, at + 4)?, float(b, at + 8)?, float(b, at + 12)?])
        };
        let shape = le32(b, 0)? as usize;
        let shape_item = world.items.get(shape).ok_or("body shape missing")?;
        let shape_data = world.item_at(bytes, shape, 0)?;
        let properties = le32(shape_data, world.field_offset(shape_item.type_index, "properties")?)?;
        let object = world
            .property_object(bytes, properties, 0xf100)?
            .ok_or("body shape has no mass properties")?;
        let object_item = world.items.get(object).ok_or("mass properties missing")?;
        let object_data = world.item_at(bytes, object, 0)?;
        let compressed = world.field_offset(object_item.type_index, "compressedMassProperties")?;
        let mut mass = mass::decode_principal(slice(object_data, compressed, 32)?)?;
        let cinfo_mass = float(b, 0x70)?;
        // 14008da20: a non-positive cinfo mass takes the shape's own mass.
        if cinfo_mass > 0. {
            mass.set_mass(cinfo_mass);
        }
        bodies.push(SystemBody {
            name: string(&world, bytes, le32(b, 0x14)?)?,
            position: vector(0x30)?,
            orientation: vector(0x40)?,
            motion_type: b[0x20],
            mass,
        });
    }
    let (constraints_item, constraint_count) = array(&world, field("constraintCinfos")?)?;
    let mut constraints = Vec::with_capacity(constraint_count);
    for k in 0..constraint_count {
        let c = world.item_at(bytes, constraints_item, k)?;
        let data_ref = le32(c, 0)? as usize;
        let data_item = world.items.get(data_ref).ok_or("constraint data missing")?;
        if world.types[data_item.type_index].name != "hkpRagdollConstraintData" {
            return Err(format!(
                "unsupported physics-system constraint {}",
                world.types[data_item.type_index].name
            ));
        }
        let atoms = world.item_at(bytes, data_ref, 0)?;
        let index = |at: usize| -> Result<usize, String> {
            let id = le32(c, at)? & 0xff_ffff;
            if id as usize >= body_count {
                return Err("constraint body out of range".into());
            }
            Ok(id as usize)
        };
        constraints.push(SystemConstraint {
            name: string(&world, bytes, le32(c, 0x10)?)?,
            body_a: index(4)?,
            body_b: index(8)?,
            atoms: ragdoll_atoms(slice(atoms, 0x10, 0x170)?)?,
        });
    }
    Ok(PhysicsSystem { name, bodies, constraints })
}

/// One `.ragdoll` entry (14009a670): the skeleton joint a body drives and
/// the body-from-joint transform (three columns and the translation);
/// 14076a180 keyframes a body as joint * transform.
#[derive(Clone, Copy, Debug)]
pub struct RagdollEntry {
    pub joint: u16,
    pub transform: [[f32; 4]; 4],
}

/// 14009a530/14009a670: "RAG\x06", count, then per entry a u16 joint index
/// and 12 floats (+0/+4/+8, +0x10.., +0x20.., +0x30.. of an hkTransform).
pub fn parse_ragdoll(bytes: &[u8]) -> Result<Vec<RagdollEntry>, String> {
    if le32(bytes, 0)? != 0x0647_4152 {
        return Err("not a RAG6 ragdoll resource".into());
    }
    let count = le32(bytes, 4)? as usize;
    let mut at = 8;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let joint = u16::from_le_bytes(slice(bytes, at, 2)?.try_into().unwrap());
        at += 2;
        let mut transform = [[0.; 4]; 4];
        for column in &mut transform {
            for value in column.iter_mut().take(3) {
                *value = float(bytes, at)?;
                at += 4;
            }
        }
        entries.push(RagdollEntry { joint, transform });
    }
    Ok(entries)
}

/// The MD5 the resource system uses for generated names (14010caf0/14010c160).
fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.).sin().abs() * 4294967296.) as u32).collect();
    let mut state = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend((input.len() as u64 * 8).to_le_bytes());
    for block in message.chunks_exact(64) {
        let m: Vec<u32> = block.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]).rotate_left(S[i]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut out = [0; 16];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// 14009a3c0: an arkRagdollResource named "<skeleton>&<physics system>"
/// lives at "generated/<skeleton directory>%08x.ragdoll", the XOR of the
/// name's four MD5 words (14010c080).
pub fn ragdoll_path(skeleton: &str, physics_system: &str) -> String {
    let name = format!("{skeleton}&{physics_system}");
    let digest = md5(name.as_bytes());
    let word = |i: usize| u32::from_le_bytes(digest[i * 4..i * 4 + 4].try_into().unwrap());
    let hash = word(1) ^ word(3) ^ word(2) ^ word(0);
    let directory = skeleton.rsplit_once('/').map_or("", |(d, _)| d);
    format!("generated/{directory}/{hash:08x}.ragdoll")
}

#[cfg(test)]
mod tests {
    #[test]
    fn ragdoll_path_matches_installed_names() {
        let base = "models/environment/gameplay/speaker_01/cables_speaker/";
        assert_eq!(
            super::ragdoll_path(&format!("{base}cable_01.md6skl"), &format!("{base}cable_01_rgdl.hkt")),
            format!("generated/{base}74d2e855.ragdoll")
        );
        assert_eq!(
            super::ragdoll_path(&format!("{base}cable_03.md6skl"), &format!("{base}cable_03_rgdl.hkt")),
            format!("generated/{base}f0887d4a.ragdoll")
        );
    }

    #[test]
    #[ignore = "requires installed Dust District archives"]
    fn installed_speaker_cable_systems_parse() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        for k in 1..=3 {
            let path = format!(
                "generated/physsystem/models/environment/gameplay/speaker_01/cables_speaker/cable_0{k}_rgdl.hkx"
            );
            let system = super::parse(&catalog.read(&path).unwrap().unwrap()).unwrap();
            println!(
                "{path}: {} bodies {:?}, {} constraints",
                system.bodies.len(),
                system.bodies.iter().map(|b| (&b.name, b.mass.mass, b.motion_type)).collect::<Vec<_>>(),
                system.constraints.len()
            );
            for c in &system.constraints {
                println!("  {} {}->{} cone {:?}", c.name, c.body_a, c.body_b, c.atoms.cone.map(|l| (l.maximum, l.memory_offset)));
            }
            assert!(system.bodies.len() >= 12);
            assert_eq!(system.constraints.len(), system.bodies.len() - 1);
        }
    }
}
