//! FX manager deferred gameplay events219370/21a730. The FX manager owns
//! this FIFO; canonical entity/explosion owners consume accepted requests.
use std::sync::Arc;
#[derive(Clone)]
pub(crate) enum Request {
    Entity {
        definition: Arc<crate::assets::runtime_entity::Definition>,
        frame: crate::gameplay::reference_frame::ReferenceFrame,
        source_lease: Arc<()>,
    },
    Explosion {
        request: crate::gameplay::explosions::Request,
        source_lease: Arc<()>,
    },
}

///21da80 drains219820's entity records before21aaa0's explosions, even when
///several FX instances produced both families during the same update.
#[derive(Default)]
pub(crate) struct Requests {
    entities: std::collections::VecDeque<Request>,
    explosions: std::collections::VecDeque<Request>,
}
impl Requests {
    pub(crate) fn push_back(&mut self, request: Request) {
        match request {
            Request::Entity { .. } => self.entities.push_back(request),
            Request::Explosion { .. } => self.explosions.push_back(request),
        }
    }
    pub(crate) fn front(&self) -> Option<&Request> {
        self.entities.front().or_else(|| self.explosions.front())
    }
    pub(crate) fn pop_front(&mut self) -> Option<Request> {
        self.entities
            .pop_front()
            .or_else(|| self.explosions.pop_front())
    }
    pub(crate) fn clear(&mut self) {
        self.entities.clear();
        self.explosions.clear();
    }
}
