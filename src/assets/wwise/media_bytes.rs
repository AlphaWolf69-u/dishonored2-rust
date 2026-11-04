//! Shared encoded media ranges. Wwise sources reference their loaded bank
//! (`1410f1910` / `141122400`) rather than retaining a second media copy.
use std::{ops::{Deref, Range}, sync::Arc};

#[derive(Clone, Copy)]
enum Extent { Resident, Header(usize), Prefetch(usize) }

#[derive(Clone)]
pub struct MediaBytes {
    bytes: Arc<[u8]>,
    range: Range<usize>,
    extent: Extent,
}

impl MediaBytes {
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(range.start <= range.end && range.end <= self.len());
        Self { bytes: self.bytes.clone(), range: self.range.start + range.start..self.range.start + range.end, extent: Extent::Resident }
    }

    /// Format-only acquisition for a file source. The audio payload stays in
    /// its package; dereferencing this value exposes only the acquired header.
    pub fn stream_header(bytes: Vec<u8>, source_size: usize) -> Self {
        let mut header = Self::from(bytes);
        header.extent = Extent::Header(source_size);
        header
    }

    pub fn source_size(&self) -> usize {
        match self.extent { Extent::Resident => self.range.len(), Extent::Header(size) | Extent::Prefetch(size) => size }
    }

    pub(crate) fn is_stream_header(&self) -> bool { matches!(self.extent, Extent::Header(_)) }
    pub(crate) fn is_prefetched(&self) -> bool { matches!(self.extent, Extent::Prefetch(_)) }
    pub(crate) fn stream_prefetch(mut self, source_size: usize) -> Self {
        self.extent = Extent::Prefetch(source_size);
        self
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
        Self { bytes, range, extent: Extent::Resident }
    }
}

impl From<Vec<u8>> for MediaBytes {
    fn from(bytes: Vec<u8>) -> Self { Arc::<[u8]>::from(bytes).into() }
}

impl From<&[u8]> for MediaBytes {
    fn from(bytes: &[u8]) -> Self { Arc::<[u8]>::from(bytes).into() }
}
