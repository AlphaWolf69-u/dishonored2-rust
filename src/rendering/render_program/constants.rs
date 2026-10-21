//! Packed constant-buffer updates used by 2af360. The backend owns mapping and
//! upload; this copies the same parameter bytes without interpreting floats.

pub fn copy_constant_buffer(
    target: &mut [u8],
    bindings: &[u32],
    parameter_block: &[u8],
    mut type_flags: impl FnMut(u16) -> Result<u32, String>,
) -> Result<(), String> {
    // The native zero-byte buffer does not map or resolve any parameters.
    if target.is_empty() {
        return Ok(());
    }
    for &binding in bindings {
        let index = (binding & 0xfff) as u16;
        let flags = type_flags(index)?;
        let size = ((flags >> 12 & 63) * (flags >> 9 & 7) * (flags >> 6 & 7) * 4) as usize;
        if size == 0 {
            continue;
        }
        let source = (usize::from(index) + 1) * 16;
        let destination = (binding >> 12 & 0x7ff) as usize;
        let bytes = parameter_block
            .get(source..source + size)
            .ok_or_else(|| format!("constant parameter {index} exceeds its source block"))?;
        let output = target
            .get_mut(destination..destination + size)
            .ok_or_else(|| format!("constant parameter {index} exceeds its target buffer"))?;
        output.copy_from_slice(bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn packed_constant_updates_match_executed_uploader() {
        let decode = |text: &str| {
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>()
        };
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/constant_buffer_updates.txt").lines() {
            let columns: Vec<_> = line.split_whitespace().collect();
            let size = usize::from_str_radix(columns[0], 16).unwrap();
            let maps: Vec<_> = columns[1]
                .split(',')
                .map(|entry| {
                    let (binding, flags) = entry.split_once(':').unwrap();
                    (
                        u32::from_str_radix(binding, 16).unwrap(),
                        u32::from_str_radix(flags, 16).unwrap(),
                    )
                })
                .collect();
            let mut output = decode(columns[2]);
            let source = decode(columns[3]);
            let bindings: Vec<_> = maps.iter().map(|&(binding, _)| binding).collect();
            let mut resolved = 0;
            super::copy_constant_buffer(&mut output[..size], &bindings, &source, |index| {
                resolved += 1;
                Ok(maps
                    .iter()
                    .find(|&&(binding, _)| binding & 0xfff == u32::from(index))
                    .unwrap()
                    .1)
            })
            .unwrap();
            assert_eq!(output, decode(columns[4]), "native upload case {count}");
            assert_eq!(resolved, if size == 0 { 0 } else { bindings.len() });
            count += 1;
        }
        assert_eq!(count, 32);
        assert!(super::copy_constant_buffer(&mut [0; 4], &[0], &[0; 16], |_| Ok(0x1240)).is_err());
        assert!(super::copy_constant_buffer(&mut [0; 1], &[0], &[0; 32], |_| Ok(0x1240)).is_err());
    }
}
