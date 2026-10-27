//! NodeEntityFx's actor lists and completion notification (52cdb0/52cf20/52c970).
//! Scene actors and manager instances remain owned by the world host.
#[derive(Default)]
pub struct EntityFxNode {
    started: Vec<u32>,
    stopped: Vec<u32>,
    finished_stops: Vec<u32>,
}
impl EntityFxNode {
    pub fn started(&mut self, actor: u32) {
        if !self.finished_stops.contains(&actor) {
            self.started.push(actor);
        }
    }
    pub fn stopped(&mut self, actor: u32) {
        if !self.stopped.contains(&actor) {
            self.stopped.push(actor);
        }
    }
    pub fn finished(&mut self, actor: u32) {
        let started = remove(&mut self.started, actor);
        let stopped = remove(&mut self.stopped, actor);
        if !started && stopped {
            self.finished_stops.push(actor);
        }
    }
}
fn remove(values: &mut Vec<u32>, actor: u32) -> bool {
    if let Some(index) = values.iter().position(|value| *value == actor) {
        values.swap_remove(index);
        true
    } else {
        false
    }
}
