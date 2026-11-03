//! Shared encoded media ranges. Wwise sources reference their loaded bank
//! (`1410f1910` / `141122400`) rather than retaining a second media copy.
use std::{ops::{Deref, Range}, sync::Arc};

#[derive(Clone)]
pub struct MediaBytes {
    bytes: Arc<[u8]>,
    range: Range<usize>,
}

impl MediaBytes {
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(range.start <= range.end && range.end <= self.len());
        Self { bytes: self.bytes.clone(), range: self.range.start + range.start..self.range.start + range.end }
    }

    /// Identity and size of the retained allocation, for residency accounting.
    pub fn allocation(&self) -> (usize, usize) {
        (self.bytes.as_ptr() as usize, self.bytes.len())
    }
}

impl Deref for MediaBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] { &self.bytes[self.range.clone()] }
}

impl AsRef<[u8]> for MediaBytes {
    fn as_ref(&self) -> &[u8] { self }
}

impl std::fmt::Debug for MediaBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaBytes").field("len", &self.len()).finish()
    }
}

impl From<Arc<[u8]>> for MediaBytes {
    fn from(bytes: Arc<[u8]>) -> Self {
        let range = 0..bytes.len();
        Self { bytes, range }
    }
}

impl From<Vec<u8>> for MediaBytes {
    fn from(bytes: Vec<u8>) -> Self { Arc::<[u8]>::from(bytes).into() }
}

impl From<&[u8]> for MediaBytes {
    fn from(bytes: &[u8]) -> Self { Arc::<[u8]>::from(bytes).into() }
}
