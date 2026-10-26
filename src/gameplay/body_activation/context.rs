//! Connect activation sequencing to the shared signal owner and hybrid index.
use super::*;
use crate::gameplay::{broadphase::HybridBroadphase, signal::Signal};

pub struct ActivationSignals<S, C> {
    pub activated: Signal<S, C>,
    pub deactivated: Signal<S, C>,
}
impl<S, C> Default for ActivationSignals<S, C> {
    fn default() -> Self {
        Self {
            activated: Signal::default(),
            deactivated: Signal::default(),
        }
    }
}

pub struct ActivationOwners<'a> {
    pub bodies: &'a mut BodyStorage,
    pub motions: &'a mut MotionStorage,
    pub deactivation: &'a mut DeactivationWorld,
    pub broadphase: &'a mut HybridBroadphase,
}

pub struct ActivationContext<'a, S, C, F> {
    pub signals: &'a mut ActivationSignals<S, C>,
    pub broadphase: &'a mut HybridBroadphase,
    pub dispatch: F,
}

impl<S: Copy + Eq, C: Copy + Eq, F> ActivationCallbacks for ActivationContext<'_, S, C, F>
where
    F: FnMut(&mut Signal<S, C>, S, C, ActivationOwners<'_>, &[u32], bool) -> Result<(), String>,
{
    fn notify(
        &mut self,
        deactivation: &mut DeactivationWorld,
        bodies: &mut BodyStorage,
        motions: &mut MotionStorage,
        ids: &[u32],
        activated: bool,
    ) -> Result<(), String> {
        let signal = if activated {
            &mut self.signals.activated
        } else {
            &mut self.signals.deactivated
        };
        let dispatch = &mut self.dispatch;
        let broadphase = &mut *self.broadphase;
        let mut error = None;
        signal.emit(&ids, |signal, subscriber, callback, ids| {
            if error.is_none() {
                let owners = ActivationOwners {
                    bodies: &mut *bodies,
                    motions: &mut *motions,
                    deactivation: &mut *deactivation,
                    broadphase: &mut *broadphase,
                };
                if let Err(failure) = dispatch(signal, subscriber, callback, owners, ids, activated)
                {
                    error = Some(failure);
                }
            }
        });
        error.map_or(Ok(()), Err)
    }

    fn update_broadphase(
        &mut self,
        _: &mut BodyStorage,
        ids: &[u32],
        stride: u32,
    ) -> Result<(), String> {
        if stride != std::mem::size_of::<u32>() as u32 {
            return Err("activation update does not contain contiguous body IDs".into());
        }
        self.broadphase.mark_bodies(ids.iter().copied());
        Ok(())
    }
}
