//! Opt-in same-build control/verification of the real map query, with no
//! permanent recorder or substitute scene. Read once while loading.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use super::InterpolatedProbeLighting;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Strategy { Spatial, Linear, Verify }
#[derive(Debug, Default)]
struct Cost { calls: AtomicU64, visits: AtomicU64, nanos: AtomicU64, cycles: AtomicU64 }
#[derive(Debug)]
pub(super) struct Diagnostics {
    mode: Strategy, spatial: Cost, linear: Cost,
    comparisons: AtomicU64, mismatches: AtomicU64,
    spatial_signature: AtomicU64, linear_signature: AtomicU64,
}
impl Diagnostics {
    pub(super) fn from_environment() -> Result<Option<std::sync::Arc<Self>>, String> {
        let Some(value) = std::env::var_os("DH2_DEBUG_PROBE_QUERY") else { return Ok(None); };
        let mode = match value.to_str() {
            Some("spatial") => Strategy::Spatial, Some("linear") => Strategy::Linear,
            Some("verify") => Strategy::Verify,
            _ => return Err("DH2_DEBUG_PROBE_QUERY requires spatial, linear, or verify".into()),
        };
        Ok(Some(std::sync::Arc::new(Self { mode, spatial: Cost::default(), linear: Cost::default(),
            comparisons: AtomicU64::new(0), mismatches: AtomicU64::new(0),
            spatial_signature: AtomicU64::new(0), linear_signature: AtomicU64::new(0) })))
    }
    pub(super) fn strategy(&self) -> Strategy { self.mode }
    pub(super) fn selection(&self, spatial: bool, visits: usize, nanos: u64, cycles: u64) {
        let cost = if spatial { &self.spatial } else { &self.linear };
        cost.calls.fetch_add(1, Relaxed); cost.visits.fetch_add(visits as u64, Relaxed);
        cost.nanos.fetch_add(nanos, Relaxed);
        cost.cycles.fetch_add(cycles, Relaxed);
    }
    pub(super) fn compare(&self, point: [f32; 3], spatial: Option<&InterpolatedProbeLighting>,
                          linear: Option<&InterpolatedProbeLighting>) {
        let record = |value: Option<&InterpolatedProbeLighting>| value.map(|v|
            v.object_shader_record().map(|row| row.map(f32::to_bits)));
        let (spatial, linear) = (record(spatial), record(linear));
        let signature = |record: &Option<[[u32; 4]; 9]>| {
            let mut crc = flate2::Crc::new();
            for value in point.map(f32::to_bits) { crc.update(&value.to_le_bytes()); }
            crc.update(&[u8::from(record.is_some())]);
            if let Some(record) = record { for row in record { for v in row { crc.update(&v.to_le_bytes()); } } }
            crc.sum() as u64
        };
        self.spatial_signature.fetch_add(signature(&spatial), Relaxed);
        self.linear_signature.fetch_add(signature(&linear), Relaxed);
        self.comparisons.fetch_add(1, Relaxed);
        if spatial != linear && self.mismatches.fetch_add(1, Relaxed) < 8 {
            eprintln!("PROBE_QUERY_MISMATCH point={point:?} spatial={spatial:?} linear={linear:?}");
        }
    }
    pub(super) fn report(&self, phase: &str) {
        eprintln!("PROBE_QUERY_COMPARISON phase={phase} mode={:?} spatial_calls={} spatial_visits={} spatial_ns={} spatial_cycles={} linear_calls={} linear_visits={} linear_ns={} linear_cycles={} comparisons={} mismatches={} spatial_signature={:016x} linear_signature={:016x}",
            self.mode, self.spatial.calls.load(Relaxed), self.spatial.visits.load(Relaxed), self.spatial.nanos.load(Relaxed), self.spatial.cycles.load(Relaxed),
            self.linear.calls.load(Relaxed), self.linear.visits.load(Relaxed), self.linear.nanos.load(Relaxed), self.linear.cycles.load(Relaxed),
            self.comparisons.load(Relaxed), self.mismatches.load(Relaxed),
            self.spatial_signature.load(Relaxed), self.linear_signature.load(Relaxed));
    }
}
impl Drop for Diagnostics { fn drop(&mut self) { self.report("drop"); } }
