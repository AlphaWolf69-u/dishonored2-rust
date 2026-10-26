//! Generated 946640 factory bindings. Capacity globals are runtime inputs,
//! not defaults read from uninitialized executable storage.
use super::{word, ClassDeclaration, Image};
use std::collections::HashMap;

#[derive(Debug)]
pub struct ComponentPoolDeclaration {
    /// The factory's +50 ordinal. This is not the class hierarchy's type ID.
    pub pool_index: u32,
    /// Index into ClassDeclarations::classes and its materialized registry.
    pub class: usize,
    pub initial_count_rva: u32,
    pub vtable_rva: u32,
    pub constructor_rva: u32,
    pub initial_count_cvar: String,
    pub initial_count_reset: String,
}

pub(super) fn read(
    image: &Image<'_>,
    classes: &[ClassDeclaration],
) -> Result<Vec<ComponentPoolDeclaration>, String> {
    const START: u32 = 0x946640;
    const END: u32 = 0x94dea9;
    let code = image.read(START, (END - START) as usize)?;
    let by_address: HashMap<_, _> = classes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.class_info_rva, i))
        .collect();
    let mut pools = Vec::new();
    for at in 22..code.len().saturating_sub(48) {
        if code[at] != 0xe8 {
            continue;
        }
        let relative = i32::from_le_bytes(code[at + 1..at + 5].try_into().unwrap());
        if i64::from(START) + at as i64 + 5 + i64::from(relative) != 0x48fa80 {
            continue;
        }
        let pool_index = pools.len() as u32;
        let ordinal_matches = if pool_index == 0 {
            code[at - 19..at - 16] == [0x45, 0x33, 0xc9]
        } else {
            code[at - 22..at - 20] == [0x41, 0xb9]
                && word(code, at - 20, 4)? == u64::from(pool_index)
        };
        if !ordinal_matches
            || code[at - 16..at - 13] != [0x4c, 0x8d, 0x05]
            || code[at - 9..at - 7] != [0x8b, 0x15]
            || code[at - 3..at] != [0x48, 0x8b, 0xc8]
            || code[at + 5..at + 8] != [0x48, 0x8d, 0x05]
            || code[at + 12..at + 15] != [0x48, 0x89, 0x07]
        {
            return Err(format!(
                "unsupported component factory registration at {:#x}",
                START + at as u32
            ));
        }
        let rip = |offset: usize, next: usize| -> Result<u32, String> {
            let relative = i32::from_le_bytes(code[offset..offset + 4].try_into().unwrap());
            u32::try_from(i64::from(START) + next as i64 + i64::from(relative))
                .map_err(|_| "component factory target is outside image".into())
        };
        let class_rva = rip(at - 13, at - 9)?;
        let class = *by_address
            .get(&class_rva)
            .ok_or("component factory class has no declaration")?;
        let initial_count_rva = rip(at - 7, at - 3)?;
        let vtable_rva = rip(at + 8, at + 12)?;
        let constructor_rva = word(image.read(vtable_rva + 8, 8)?, 0, 8)?
            .checked_sub(image.base)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or("component constructor is outside image")?;
        if !image.mapped(initial_count_rva, 4) || !image.executable_rva(constructor_rva) {
            return Err("component factory input or constructor is unmapped".into());
        }
        let expected_offset = 0x38 + 8 * pool_index;
        let stored = (at + 15..at + 40).any(|offset| {
            (code[offset..offset + 3] == [0x48, 0x89, 0x7b]
                && u32::from(code[offset + 3]) == expected_offset)
                || (code[offset..offset + 3] == [0x48, 0x89, 0xbb]
                    && word(code, offset + 3, 4).is_ok_and(|v| v == u64::from(expected_offset)))
        });
        if !stored {
            return Err("component factory manager slot does not match its ordinal".into());
        }
        pools.push(ComponentPoolDeclaration {
            pool_index,
            class,
            initial_count_rva,
            vtable_rva,
            constructor_rva,
            initial_count_cvar: String::new(),
            initial_count_reset: String::new(),
        });
    }
    if pools.len() != 350 {
        return Err("component factory registration table is incomplete".into());
    }
    read_capacity_names(image, &mut pools)?;
    Ok(pools)
}

fn read_capacity_names(
    image: &Image<'_>,
    pools: &mut [ComponentPoolDeclaration],
) -> Result<(), String> {
    let by_value: HashMap<_, _> = pools
        .iter()
        .enumerate()
        .map(|(i, p)| (p.initial_count_rva, i))
        .collect();
    let mut seen = std::collections::HashSet::new();
    for section in image.sections.iter().filter(|s| s.executable) {
        let code = section.raw(image.bytes)?;
        for at in 14..code.len().saturating_sub(32) {
            if code[at..at + 3] != [0x48, 0x8d, 0x0d] || code[at + 27] != 0xe8 {
                continue;
            }
            let rip = |offset: usize, next: usize| -> Result<u32, String> {
                let relative = i32::from_le_bytes(code[offset..offset + 4].try_into().unwrap());
                u32::try_from(i64::from(section.rva) + next as i64 + i64::from(relative))
                    .map_err(|_| "component capacity initializer target is outside image".into())
            };
            if rip(at + 28, at + 32).ok() != Some(0x1008f0) {
                continue;
            }
            let Ok(object) = rip(at + 3, at + 7) else {
                continue;
            };
            let Some(index) = object
                .checked_add(0x28)
                .and_then(|value| by_value.get(&value).copied())
            else {
                continue;
            };
            if !seen.insert(index)
                || code[at - 14..at - 11] != [0x4c, 0x8d, 0x05]
                || code[at - 7..at - 4] != [0x48, 0x8d, 0x15]
                || code[at + 7..at + 13] != [0x41, 0xb9, 2, 0, 0, 0]
                || code[at + 13..at + 22] != [0x48, 0xc7, 0x44, 0x24, 0x28, 0, 0, 0, 0]
                || code[at + 22..at + 27] != [0x48, 0x89, 0x44, 0x24, 0x20]
            {
                return Err("unsupported component capacity cvar initializer".into());
            }
            pools[index].initial_count_cvar = String::from_utf8(image.string(rip(at - 4, at)?)?)
                .map_err(|_| "component capacity name is not UTF-8")?;
            pools[index].initial_count_reset =
                String::from_utf8(image.string(rip(at - 11, at - 7)?)?)
                    .map_err(|_| "component capacity reset is not UTF-8")?;
        }
    }
    if seen.len() != pools.len() {
        return Err("component pool capacity registration is incomplete".into());
    }
    Ok(())
}
