//! Generated93bf20 type descriptors, read from the audited installed PE.
//! Decode only literal address/scalar stores; no game code executes here.
use super::{word, ComponentPoolDeclaration, Image};

#[derive(Debug)]
pub struct ComponentTypeDeclaration {
    pub pool_index: u32,
    pub class: usize,
    pub update_flags: u32,
    pub setup_rva: u32,
    pub definition_rva: u32,
}

fn displacement(code: &[u8], at: usize) -> Result<(usize, usize), String> {
    let modrm = *code.get(at).ok_or("truncated component descriptor store")?;
    if modrm & 7 != 3 {
        return Err("component descriptor destination is not its allocated table".into());
    }
    let (offset, length) = match modrm >> 6 {
        0 => (0, 1),
        1 => (
            *code.get(at + 1).ok_or("truncated descriptor offset")? as i8 as i32,
            2,
        ),
        2 => (
            i32::from_le_bytes(
                code.get(at + 1..at + 5)
                    .ok_or("truncated descriptor offset")?
                    .try_into()
                    .unwrap(),
            ),
            5,
        ),
        _ => return Err("component descriptor store is not a memory operand".into()),
    };
    Ok((
        usize::try_from(offset).map_err(|_| "negative component descriptor offset")?,
        length,
    ))
}

pub(super) fn read(
    image: &Image<'_>,
    pools: &[ComponentPoolDeclaration],
    classes: &[super::ClassDeclaration],
) -> Result<Vec<ComponentTypeDeclaration>, String> {
    const START: u32 = 0x93bf7f;
    const END: u32 = 0x93fbc2;
    const STRIDE: usize = 0x218;
    if image.read(0x93bf43, 2)? != [0x33, 0xff] {
        return Err("component descriptor zero-register setup differs".into());
    }
    let code = image.read(START, (END - START) as usize)?;
    let mut registers = [None; 8];
    registers[7] = Some(0); // EDI was zeroed by the audited constructor prologue.
    let mut fields = vec![[None; 4]; pools.len()];
    let mut at = 0;
    while at < code.len() {
        if code.get(at..at + 2) == Some(&[0x48, 0x8d]) {
            let modrm = *code.get(at + 2).ok_or("truncated descriptor LEA")?;
            if modrm & 0xc7 != 5 {
                return Err("component descriptor LEA is not RIP-relative".into());
            }
            let delta = i32::from_le_bytes(
                code.get(at + 3..at + 7)
                    .ok_or("truncated descriptor address")?
                    .try_into()
                    .unwrap(),
            );
            let target = u32::try_from(i64::from(START) + at as i64 + 7 + i64::from(delta))
                .map_err(|_| "component descriptor address exceeds image")?;
            registers[((modrm >> 3) & 7) as usize] = Some(target);
            at += 7;
            continue;
        }
        let wide = code[at] == 0x48;
        let opcode = at + usize::from(wide);
        let instruction = *code
            .get(opcode)
            .ok_or("truncated component descriptor instruction")?;
        if !matches!(instruction, 0x89 | 0xc7) || (wide && instruction != 0x89) {
            return Err(format!(
                "unsupported component descriptor instruction at RVA{:x}",
                START + at as u32
            ));
        }
        let modrm = *code.get(opcode + 1).ok_or("truncated descriptor operand")?;
        let (offset, length) = displacement(code, opcode + 1)?;
        let index = offset / STRIDE;
        let field = match offset % STRIDE {
            0 if wide => 0,
            8 if wide => 1,
            0x10 if !wide => 2,
            0x18 if wide => 3,
            _ => return Err("component descriptor writes an undefined field".into()),
        };
        let value = if instruction == 0xc7 {
            if modrm & 0x38 != 0 {
                return Err("unsupported descriptor immediate store".into());
            }
            let value = word(code, opcode + 1 + length, 4)? as u32;
            at = opcode + 1 + length + 4;
            value
        } else {
            let register = ((modrm >> 3) & 7) as usize;
            if !wide && register != 7 {
                return Err("descriptor scalar is not its defined zero register".into());
            }
            let value = registers[register].ok_or("descriptor uses an undefined register")?;
            at = opcode + 1 + length;
            value
        };
        let slot = fields
            .get_mut(index)
            .ok_or("descriptor write exceeds factory count")?;
        if slot[field].replace(value).is_some() {
            return Err("duplicate component descriptor store".into());
        }
    }
    fields
        .into_iter()
        .zip(pools)
        .map(|(fields, pool)| {
            let [class, setup, flags, definition] =
                fields.map(|v| v.ok_or("incomplete component descriptor"));
            let (class, setup, flags, definition) = (class?, setup?, flags?, definition?);
            if class != classes[pool.class].class_info_rva
                || !image.executable_rva(setup)
                || !image.mapped(definition, 1)
            {
                return Err(
                    "component descriptor class/setup/definition differs from installed factory"
                        .into(),
                );
            }
            Ok(ComponentTypeDeclaration {
                pool_index: pool.pool_index,
                class: pool.class,
                update_flags: flags,
                setup_rva: setup,
                definition_rva: definition,
            })
        })
        .collect()
}
