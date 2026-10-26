//! Case-sensitive atomic names and stable string ownership (131bb0/131f90).
//! Rust owns the blocks; a name's address remains its identity until pool drop.
use crate::entities::declaration_name_hash;

static EMPTY_NAME: [u8; 1] = [0];
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NameId(usize);
impl NameId {
    pub fn identity(self) -> usize {
        self.0
    }
    pub fn empty() -> Self {
        Self(EMPTY_NAME.as_ptr() as usize)
    }
}

struct Block {
    bytes: Box<[u8]>,
    aligned_offset: usize,
    allocation_size: usize,
    used: usize,
}
impl Block {
    fn new(size: usize) -> Result<Self, String> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size + 15)
            .map_err(|e| format!("name pool allocation failed: {e}"))?;
        bytes.resize(size + 15, 0);
        let bytes = bytes.into_boxed_slice();
        let address = bytes.as_ptr() as usize;
        let aligned_offset = ((address + 15) & !15) - address;
        Ok(Self {
            bytes,
            aligned_offset,
            allocation_size: size,
            used: 0,
        })
    }
    fn address(&self) -> usize {
        self.bytes.as_ptr() as usize + self.aligned_offset
    }
    fn remaining(&self) -> usize {
        self.allocation_size - 16 - self.used
    }
}
struct Entry {
    next: u32,
    block: usize,
    text_offset: usize,
    len: usize,
}

pub struct NamePool {
    buckets: Vec<u32>,
    entries: Vec<Entry>,
    blocks: Vec<Block>,
    cache_hash: u32,
    cache_entry: Option<u32>,
    cache_kind: u8,
}
impl Default for NamePool {
    fn default() -> Self {
        Self {
            buckets: vec![NONE; 65536],
            entries: Vec::new(),
            blocks: Vec::new(),
            cache_hash: 0,
            cache_entry: None,
            cache_kind: 0,
        }
    }
}
impl NamePool {
    #[cfg(test)]
    pub(crate) fn test_key_location(&self, key: Option<NameId>) -> [u32; 2] {
        let Some(key) = key else {
            return [u32::MAX, 0];
        };
        if key == NameId::empty() {
            return [u32::MAX, 1];
        }
        for (index, block) in self.blocks.iter().enumerate() {
            if let Some(offset) = key
                .identity()
                .checked_sub(block.address())
                .filter(|&v| v < block.allocation_size)
            {
                return [index as u32, offset as u32];
            }
        }
        panic!("name key is outside owned blocks");
    }

    fn key(&self, index: u32) -> NameId {
        let entry = &self.entries[index as usize];
        NameId(self.blocks[entry.block].address() + entry.text_offset)
    }
    fn entry_text(&self, index: u32) -> &[u8] {
        let entry = &self.entries[index as usize];
        let block = &self.blocks[entry.block];
        let start = block.aligned_offset + entry.text_offset;
        &block.bytes[start..start + entry.len]
    }
    /// Complete131bb0. Empty/null inputs preserve the last-name cache. NUL
    /// terminates input; byte comparison remains case-sensitive after hashing.
    /// Exclusive access replaces the pool's conditional lock, not its rules.
    pub fn intern(&mut self, input: Option<&[u8]>) -> Result<NameId, String> {
        let input = input.unwrap_or(&[]);
        let text = &input[..input.iter().position(|&v| v == 0).unwrap_or(input.len())];
        if text.is_empty() {
            return Ok(NameId::empty());
        }
        let hash = declaration_name_hash(text);
        if self.cache_hash == hash && self.cache_kind == 1 {
            if let Some(index) = self.cache_entry {
                if self.entry_text(index) == text {
                    return Ok(self.key(index));
                }
            }
        }
        let bucket = (hash & 0xffff) as usize;
        let mut at = self.buckets[bucket];
        while at != NONE {
            if self.entry_text(at) == text {
                self.cache_hash = hash;
                self.cache_entry = Some(at);
                self.cache_kind = 1;
                return Ok(self.key(at));
            }
            at = self.entries[at as usize].next;
        }
        let needed = text
            .len()
            .checked_add(16)
            .ok_or("name pool entry size overflow")?
            & !7;
        if needed > i32::MAX as usize - 16 || self.entries.len() >= NONE as usize {
            return Err("name pool entry exceeds supported size".into());
        }
        if self
            .blocks
            .last()
            .is_none_or(|block| block.remaining() < needed)
        {
            self.blocks.push(Block::new((needed + 16).max(65536))?);
        }
        let block_index = self.blocks.len() - 1;
        let block = &mut self.blocks[block_index];
        let text_offset = 16 + block.used + 8;
        let start = block.aligned_offset + text_offset;
        block.bytes[start..start + text.len()].copy_from_slice(text);
        block.bytes[start + text.len()] = 0;
        block.used += needed;
        let index = self.entries.len() as u32;
        self.entries.push(Entry {
            next: self.buckets[bucket],
            block: block_index,
            text_offset,
            len: text.len(),
        });
        self.buckets[bucket] = index;
        self.cache_hash = hash;
        self.cache_entry = Some(index);
        self.cache_kind = 1;
        Ok(self.key(index))
    }

    /// Resolve an owned immutable name without dereferencing an arbitrary key.
    pub fn text(&self, key: NameId) -> Option<&[u8]> {
        if key == NameId::empty() {
            return Some(&[]);
        }
        for block in &self.blocks {
            let offset = key.0.checked_sub(block.address());
            if let Some(offset) = offset.filter(|&v| v >= 24 && v < 16 + block.used) {
                let bytes = &block.bytes
                    [block.aligned_offset + offset..block.aligned_offset + 16 + block.used];
                let end = bytes.iter().position(|&v| v == 0)?;
                return Some(&bytes[..end]);
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "name_pool/tests.rs"]
mod tests;
