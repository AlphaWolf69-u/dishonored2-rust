//! Actual CoordinationBB+68 bark records. Speech identity is a retained
//! canonical declaration generation, never an event hash or guessed NPC name.
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct BarkRecord {
    pub line: u32,
    pub speech: Arc<crate::assets::speech::Definition>,
    pub expires: f64,
    pub time_domain: i32,
}
impl BarkRecord {
    pub fn matches(&self, line: u32, speech: &Arc<crate::assets::speech::Definition>) -> bool {
        self.line == line && Arc::ptr_eq(&self.speech, speech)
    }
}
