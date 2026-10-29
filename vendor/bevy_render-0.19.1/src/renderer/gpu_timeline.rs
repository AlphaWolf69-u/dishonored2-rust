//! Diagnostic GPU timeline (`DH2_GPU_TIMELINE=1`): one timestamp after the
//! commands of every render system, in submission order, plus one when the
//! frame's GPU work begins, and one before them. End minus begin is the GPU
//! time of the system's work; the time from one system's end to the next
//! one's begin ("(gap) next") is queue writes, direct submissions and GPU
//! idle waiting for the CPU. Together they cover the whole frame without
//! instrumenting individual passes.
use super::{PendingCommandBuffers, RenderDevice, RenderQueue};
use alloc::borrow::Cow;
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, CommandBuffer, Features, MapMode, QuerySet,
    QuerySetDescriptor, QueryType,
};

const CAPACITY: u32 = 2048;
/// Label prefix of the timestamp written before a system's commands.
pub const BEGIN: &str = "begin|";
const READBACKS: usize = 4;

struct Readback {
    resolve: Buffer,
    buffer: Buffer,
    names: Vec<Cow<'static, str>>,
    ready: Arc<AtomicBool>,
    in_flight: bool,
}

/// Per-system totals across collected frames: (nanoseconds, samples).
#[derive(Default)]
struct Totals {
    systems: HashMap<String, (f64, u64)>,
    frames: u64,
}

static TOTALS: Mutex<Option<Totals>> = Mutex::new(None);
/// Timestamps written, frames resolved, readbacks collected: the report's sanity check.
pub static GPU_TIMELINE_COUNTS: [core::sync::atomic::AtomicU64; 3] =
    [const { core::sync::atomic::AtomicU64::new(0) }; 3];

/// Discard collected totals, e.g. when a benchmark's measured interval begins.
pub fn reset_gpu_timeline() {
    if let Ok(mut totals) = TOTALS.lock() {
        *totals = Some(Totals::default());
    }
}

/// Collected frames and each system's mean GPU milliseconds per frame, largest first.
pub fn gpu_timeline_report() -> Option<(u64, Vec<(String, f64)>)> {
    let totals = TOTALS.lock().ok()?;
    let totals = totals.as_ref()?;
    let frames = totals.frames.max(1) as f64;
    let mut rows: Vec<_> = totals
        .systems
        .iter()
        .map(|(name, (nanos, _))| (name.clone(), nanos / frames / 1e6))
        .collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    Some((totals.frames, rows))
}

#[derive(Resource)]
pub struct GpuTimeline(Mutex<Inner>);

struct Inner {
    query_set: QuerySet,
    names: Vec<Cow<'static, str>>,
    readbacks: Vec<Readback>,
    period_ns: f32,
}

impl GpuTimeline {
    fn new(device: &RenderDevice, queue: &RenderQueue) -> Option<Self> {
        let required = Features::TIMESTAMP_QUERY | Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if !device.features().contains(required) {
            bevy_log::warn!("DH2_GPU_TIMELINE needs timestamp queries inside encoders");
            return None;
        }
        let wgpu = device.wgpu_device();
        let query_set = wgpu.create_query_set(&QuerySetDescriptor {
            label: Some("gpu timeline"),
            ty: QueryType::Timestamp,
            count: CAPACITY,
        });
        let size = u64::from(CAPACITY) * 8;
        let readbacks = (0..READBACKS)
            .map(|_| Readback {
                resolve: wgpu.create_buffer(&BufferDescriptor {
                    label: Some("gpu timeline resolve"),
                    size,
                    usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                buffer: wgpu.create_buffer(&BufferDescriptor {
                    label: Some("gpu timeline readback"),
                    size,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                names: Vec::new(),
                ready: Arc::new(AtomicBool::new(false)),
                in_flight: false,
            })
            .collect();
        Some(Self(Mutex::new(Inner {
            query_set,
            names: Vec::new(),
            readbacks,
            period_ns: queue.get_timestamp_period(),
        })))
    }

    /// A command buffer writing the next timestamp, labelled `name`.
    pub fn mark(&self, device: &RenderDevice, name: Cow<'static, str>) -> Option<CommandBuffer> {
        let mut inner = self.0.lock().ok()?;
        let index = u32::try_from(inner.names.len()).ok().filter(|i| *i < CAPACITY)?;
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.write_timestamp(&inner.query_set, index);
        inner.names.push(name);
        GPU_TIMELINE_COUNTS[0].fetch_add(1, Ordering::Relaxed);
        Some(encoder.finish())
    }

    /// Collect finished readbacks, then start this frame with its first timestamp.
    fn begin_frame(&self, device: &RenderDevice, queue: &RenderQueue) {
        let Ok(mut inner) = self.0.lock() else { return };
        let period = f64::from(inner.period_ns);
        for readback in &mut inner.readbacks {
            if !readback.in_flight || !readback.ready.load(Ordering::Acquire) {
                continue;
            }
            {
                let data = readback.buffer.slice(..).get_mapped_range();
                let ticks: Vec<u64> = data
                    .chunks_exact(8)
                    .take(readback.names.len())
                    .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
                    .collect();
                GPU_TIMELINE_COUNTS[2].fetch_add(1, Ordering::Relaxed);
                if let Ok(mut totals) = TOTALS.lock() {
                    let totals = totals.get_or_insert_with(Totals::default);
                    totals.frames += 1;
                    for (pair, name) in ticks.windows(2).zip(&readback.names[1..]) {
                        let key = match name.strip_prefix(BEGIN) {
                            Some(system) => format!("(gap) {system}"),
                            None => name.to_string(),
                        };
                        let entry = totals.systems.entry(key).or_default();
                        entry.0 += pair[1].saturating_sub(pair[0]) as f64 * period;
                        entry.1 += 1;
                    }
                }
            }
            readback.buffer.unmap();
            readback.in_flight = false;
            readback.ready.store(false, Ordering::Release);
        }
        inner.names.clear();
        drop(inner);
        if let Some(buffer) = self.mark(device, Cow::Borrowed("frame start")) {
            queue.submit([buffer]);
        }
    }

    /// Resolve this frame's timestamps into a free readback buffer.
    fn finish_frame(&self, device: &RenderDevice, pending: &mut PendingCommandBuffers) {
        let Ok(mut inner) = self.0.lock() else { return };
        let count = inner.names.len() as u32;
        let names = core::mem::take(&mut inner.names);
        let Inner { query_set, readbacks, .. } = &mut *inner;
        let Some(readback) = readbacks.iter_mut().find(|r| !r.in_flight) else {
            return; // Every readback still in flight: skip this frame's sample.
        };
        if count < 2 {
            return;
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.resolve_query_set(query_set, 0..count, &readback.resolve, 0);
        encoder.copy_buffer_to_buffer(&readback.resolve, 0, &readback.buffer, 0, u64::from(count) * 8);
        let ready = readback.ready.clone();
        encoder.map_buffer_on_submit(&readback.buffer, MapMode::Read, .., move |result| {
            if result.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
        readback.names = names;
        readback.in_flight = true;
        GPU_TIMELINE_COUNTS[1].fetch_add(1, Ordering::Relaxed);
        pending.push([encoder.finish()]);
    }
}

/// Schedule the timeline in the render app. Call after the render plugins
/// are built: they install the RenderGraph schedule afresh.
pub fn register_gpu_timeline(render_app: &mut bevy_app::SubApp) {
    render_app.add_systems(
        crate::Render,
        begin_gpu_timeline.before(crate::RenderSystems::ExtractCommands),
    );
    render_app.add_systems(
        super::RenderGraph,
        finish_gpu_timeline
            .after(super::RenderGraphSystems::Render)
            .before(super::RenderGraphSystems::Submit),
    );
}

fn begin_gpu_timeline(
    mut commands: Commands,
    timeline: Option<Res<GpuTimeline>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut initialized: Local<bool>,
) {
    match timeline {
        Some(timeline) => timeline.begin_frame(&device, &queue),
        None if !*initialized => {
            *initialized = true;
            if std::env::var("DH2_GPU_TIMELINE").as_deref() == Ok("1") {
                if let Some(timeline) = GpuTimeline::new(&device, &queue) {
                    commands.insert_resource(timeline);
                }
            }
        }
        None => {}
    }
}

fn finish_gpu_timeline(world: &mut World) {
    world.resource_scope(|world, mut pending: Mut<PendingCommandBuffers>| {
        if let Some(timeline) = world.get_resource::<GpuTimeline>() {
            timeline.finish_frame(world.resource::<RenderDevice>(), &mut pending);
        }
    });
}
