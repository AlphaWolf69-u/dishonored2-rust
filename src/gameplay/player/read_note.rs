//! READ_NOTE's descriptor lease (1408450b0) and release (140844e80).
//! The host prepares strings/map metadata through the existing item owner;
//! player state and the UI share that immutable presentation without copies.
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Presentation {
    Note { title: String, text: String, note_type: i32 },
    Map { title: String, image: String, legends: Vec<String>, map_type: i32 },
}

/// Host acknowledgement of a parameterized state request. A rejected or
/// cancelled request returns its lease so the interaction owner can retire it.
pub type ActivationResult = Result<Arc<Presentation>, Arc<Presentation>>;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Runtime {
    pending: Option<Arc<Presentation>>,
    pub descriptor: Option<Arc<Presentation>>,
    result: Option<ActivationResult>,
    pub admission: bool,
}

impl Runtime {
    pub fn busy(&self) -> bool { self.pending.is_some() || self.result.is_some() }
    pub fn pending(&self) -> bool { self.pending.is_some() }

    pub fn request(&mut self, presentation: Arc<Presentation>) -> bool {
        if self.busy() { return false; }
        self.pending = Some(presentation);
        true
    }

    pub fn activate(&mut self) {
        if let Some(presentation) = self.pending.take() {
            self.result = Some(Ok(presentation.clone()));
            self.descriptor = Some(presentation);
        }
    }

    pub fn reject(&mut self) {
        if let Some(presentation) = self.pending.take() {
            self.result = Some(Err(presentation));
        }
    }

    pub fn deactivate(&mut self) { self.descriptor = None; }

    pub fn take_result(&mut self) -> Option<ActivationResult> {
        self.result.take().map(|result| match result {
            Ok(presentation) if self.descriptor.as_ref().is_some_and(|descriptor| Arc::ptr_eq(descriptor, &presentation)) => Ok(presentation),
            Ok(presentation) | Err(presentation) => Err(presentation),
        })
    }
}
