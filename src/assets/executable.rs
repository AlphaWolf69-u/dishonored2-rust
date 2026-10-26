//! Shared checked Win64 PE sections and RVA reads for installed metadata.
pub(crate) fn word(bytes: &[u8], offset: usize, size: usize) -> Result<u64, String> {
    let data = bytes
        .get(
            offset
                ..offset
                    .checked_add(size)
                    .ok_or("executable offset overflow")?,
        )
        .ok_or("truncated executable metadata")?;
    Ok(data.iter().enumerate().fold(0, |value, (at, &byte)| {
        value | (u64::from(byte) << (at * 8))
    }))
}
pub(crate) struct Section {
    pub(crate) rva: u32,
    pub(crate) virtual_size: u32,
    pub(crate) file_offset: usize,
    pub(crate) file_size: usize,
    pub(crate) executable: bool,
}
impl Section {
    pub(crate) fn raw<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], String> {
        bytes
            .get(
                self.file_offset
                    ..self
                        .file_offset
                        .checked_add(self.file_size)
                        .ok_or("section size overflow")?,
            )
            .ok_or_else(|| "executable section exceeds file".into())
    }
    pub(crate) fn contains(&self, rva: u32, size: usize) -> bool {
        rva.checked_sub(self.rva).is_some_and(|offset| {
            u64::from(offset) + size as u64
                <= u64::from(self.virtual_size.max(self.file_size as u32))
        })
    }
}
pub(crate) struct Image<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) base: u64,
    pub(crate) sections: Vec<Section>,
}
impl<'a> Image<'a> {
    /// Win64 exception directory gives constructor boundaries without guessing
    /// from padding bytes or following arbitrary executable control flow.
    pub(crate) fn function_ranges(&self) -> Result<Vec<std::ops::Range<u32>>, String> {
        let pe = word(self.bytes, 0x3c, 4)? as usize;
        let entry = pe + 24 + 112 + 3 * 8;
        let rva = word(self.bytes, entry, 4)? as u32;
        let size = word(self.bytes, entry + 4, 4)? as usize;
        if size % 12 != 0 {
            return Err("invalid Win64 exception directory size".into());
        }
        self.read(rva, size)?
            .chunks_exact(12)
            .map(|row| {
                let start = word(row, 0, 4)? as u32;
                let end = word(row, 4, 4)? as u32;
                if start >= end || !self.executable_rva(start) || !self.executable_rva(end - 1) {
                    return Err("invalid Win64 function range".into());
                }
                Ok(start..end)
            })
            .collect()
    }
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.get(..2) != Some(b"MZ") {
            return Err("executable lacks DOS header".into());
        }
        let pe = word(bytes, 0x3c, 4)? as usize;
        if bytes.get(pe..pe + 4) != Some(b"PE\0\0") || word(bytes, pe + 4, 2)? != 0x8664 {
            return Err("class metadata requires a Win64 PE image".into());
        }
        let count = word(bytes, pe + 6, 2)? as usize;
        let optional = pe + 24;
        if word(bytes, optional, 2)? != 0x20b {
            return Err("executable has an unsupported optional header".into());
        }
        let base = word(bytes, optional + 24, 8)?;
        let section_table = optional + word(bytes, pe + 20, 2)? as usize;
        let mut sections = Vec::new();
        for index in 0..count {
            let at = section_table + index * 40;
            sections.push(Section {
                virtual_size: word(bytes, at + 8, 4)? as u32,
                rva: word(bytes, at + 12, 4)? as u32,
                file_size: word(bytes, at + 16, 4)? as usize,
                file_offset: word(bytes, at + 20, 4)? as usize,
                executable: word(bytes, at + 36, 4)? & 0x20000000 != 0,
            });
        }
        Ok(Self {
            bytes,
            base,
            sections,
        })
    }
    pub(crate) fn mapped(&self, rva: u32, size: usize) -> bool {
        self.sections
            .iter()
            .any(|section| section.contains(rva, size))
    }
    pub(crate) fn executable_rva(&self, rva: u32) -> bool {
        self.sections
            .iter()
            .any(|section| section.executable && section.contains(rva, 1))
    }
    pub(crate) fn executable_va(&self, va: u64) -> bool {
        va.checked_sub(self.base)
            .and_then(|v| u32::try_from(v).ok())
            .is_some_and(|rva| self.executable_rva(rva))
    }
    pub(crate) fn read(&self, rva: u32, size: usize) -> Result<&'a [u8], String> {
        let section = self
            .sections
            .iter()
            .find(|s| s.contains(rva, size))
            .ok_or("metadata RVA has no image section")?;
        let offset = (rva - section.rva) as usize;
        section
            .raw(self.bytes)?
            .get(offset..offset + size)
            .ok_or_else(|| "metadata RVA has no file bytes".into())
    }
    pub(crate) fn string(&self, rva: u32) -> Result<Vec<u8>, String> {
        let section = self
            .sections
            .iter()
            .find(|s| s.contains(rva, 1))
            .ok_or("class name has no image section")?;
        let tail = section
            .raw(self.bytes)?
            .get((rva - section.rva) as usize..)
            .ok_or("class name has no file bytes")?;
        let length = tail
            .iter()
            .position(|&v| v == 0)
            .ok_or("class name is not terminated")?;
        Ok(tail[..length].to_vec())
    }
}
