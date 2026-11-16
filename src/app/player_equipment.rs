//! bevy_backend: equipment and ordinary world models use the existing render
//! queue's submission acknowledgement before their resource leases retire.
//! Geometry, materials, probe queries and skinning stay with each actor owner.
use super::PlayerArms;
use bevy::{
    prelude::*,
    render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems, renderer::RenderQueue},
};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

struct Lease {
    // Own the exact model, material, mesh and inverse-bind handles while the
    // preceding render world can still reference them.
    _model: Box<dyn Send + Sync>,
    state: AtomicU8,
}

#[derive(Clone)]
pub(in crate::app) struct RetiredModel(Arc<Lease>);

impl RetiredModel {
    pub(in crate::app) fn new(model: impl Send + Sync + 'static) -> Self {
        Self(Arc::new(Lease {
            _model: Box::new(model),
            state: AtomicU8::new(0),
        }))
    }
    pub(in crate::app) fn complete(&self) -> bool {
        self.0.state.load(Ordering::Acquire) == 2
    }
}

#[derive(Resource, Default)]
struct Retirements(Vec<RetiredModel>);

fn extract(mut commands: Commands, arms: Extract<Option<Res<PlayerArms>>>,world_models:Extract<Option<Res<super::super::world_models::WorldModels>>>) {
    let mut receipts=arms.as_ref().map_or_else(Vec::new,|arms|arms.retiring_equipment.clone());
    if let Some(models)=world_models.as_ref(){receipts.extend(super::super::world_models::retirement_receipts(models));}
    commands.insert_resource(Retirements(receipts));
}

fn acknowledge(retirements: Res<Retirements>, queue: Res<RenderQueue>) {
    for retired in &retirements.0 {
        if retired
            .0
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            continue;
        }
        let lease = retired.0.clone();
        // Cleanup follows this frame's submission, after extraction has
        // removed the equipment's actual draw entities. This also covers
        // previously submitted frames; no frame-count delay substitutes it.
        queue.on_submitted_work_done(move || {
            lease.state.store(2, Ordering::Release);
        });
    }
}

pub(in crate::app) fn install_equipment_retirement(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<Retirements>()
            .add_systems(ExtractSchedule, extract)
            .add_systems(Render, acknowledge.in_set(RenderSystems::Cleanup));
    }
}
