//! Container readers live with the catalogue (14006a680 / 14006a310).
//! Host: exclusive leased cursors preserve parallel resource acquisition.
use super::*;
use std::{collections::HashMap, ops::{Deref, DerefMut}, sync::Mutex};

type Key = (u8, u16);

pub(super) struct Reader {
    pub file: File,
    pub length: u64,
}

#[derive(Default)]
struct State {
    idle: HashMap<Key, Vec<Reader>>,
    requests: u64,
    opened: u64,
    reused: u64,
}

impl State {
    fn report(&self, stage: &str) {
        eprintln!("MEMORY archive readers {stage}: requests={} opened={} reused={} containers={} cached_handles={}",
            self.requests, self.opened, self.reused, self.idle.len(), self.idle.values().map(Vec::len).sum::<usize>());
    }
}

#[derive(Default)]
pub(super) struct Containers(Mutex<State>);

impl Containers {
    pub fn acquire(&self, base: &Path, group: u8, slot: u16) -> Result<Lease<'_>> {
        let key = if slot == 8192 && matches!(group, 2 | 3) { (2, slot) } else { (group, slot) };
        let reader = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.requests += 1;
            let reader = state.idle.get_mut(&key).and_then(Vec::pop);
            state.reused += u64::from(reader.is_some());
            if state.requests % 1024 == 0
                && std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                state.report("active");
            }
            reader
        };
        let reader = match reader {
            Some(reader) => reader,
            None => {
                let file = File::open(archive_path(base, key.0, key.1)?)?;
                let length = file.metadata()?.len();
                self.0.lock().unwrap_or_else(|e| e.into_inner()).opened += 1;
                Reader { file, length }
            }
        };
        Ok(Lease { owner: self, key, reader: Some(reader) })
    }
}

/// A cursor is never shared by active reads. No pool lock spans IO or decode.
pub(super) struct Lease<'a> {
    owner: &'a Containers,
    key: Key,
    reader: Option<Reader>,
}
impl Deref for Lease<'_> {
    type Target = Reader;
    fn deref(&self) -> &Reader { self.reader.as_ref().unwrap() }
}
impl DerefMut for Lease<'_> {
    fn deref_mut(&mut self) -> &mut Reader { self.reader.as_mut().unwrap() }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let reader = self.reader.take().unwrap();
        self.owner.0.lock().unwrap_or_else(|e| e.into_inner())
            .idle.entry(self.key).or_default().push(reader);
    }
}
impl Drop for Containers {
    fn drop(&mut self) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            let state = self.0.get_mut().unwrap_or_else(|e| e.into_inner());
            state.report("release");
        }
    }
}
