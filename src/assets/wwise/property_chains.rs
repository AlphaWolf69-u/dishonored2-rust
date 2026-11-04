//! Host sharing of immutable actor property chains. Actor registration and
//! property decoding follow 1410f2370 / 1410de430 / 1410deb00; voice state is
//! deliberately outside this owner. Diagnostics retain only weak references.
use super::NodeProperties;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::collections::BTreeMap;

pub(super) type Chain = Arc<[NodeProperties]>;

pub(super) fn separate() -> bool {
    static SEPARATE: OnceLock<bool> = OnceLock::new();
    *SEPARATE.get_or_init(|| std::env::var("DH2_DEBUG_COPY_SOUND_PROPERTIES").as_deref() == Ok("1"))
}
fn tracing() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"))
}
#[derive(Default)]
struct Diagnostics {
    allocations: Vec<Weak<[NodeProperties]>>,
    created: usize,
    created_bytes: usize,
    requests: BTreeMap<(u32, usize, u32), u64>,
}
static DIAGNOSTICS: Mutex<Option<Diagnostics>> = Mutex::new(None);

fn bytes(chain: &[NodeProperties]) -> usize {
    use std::mem::{size_of, size_of_val};
    // Arc header plus the actual flat slice and each owned nested Vec capacity.
    2 * size_of::<usize>() + size_of_val(chain) + chain.iter().map(|node| {
        node.values.capacity() * size_of::<(u8,u32)>()
        + node.ranges.capacity() * size_of::<super::PropertyRange>()
        + node.curves.capacity() * size_of::<super::RtpcCurve>()
        + node.curves.iter().map(|curve| curve.points.capacity() * size_of::<super::CurvePoint>()).sum::<usize>()
        + node.effects.slots.capacity() * size_of::<super::EffectReference>()
        + node.bus_settings.as_ref().map_or(0, |bus| bus.ducks.capacity() * size_of::<super::BusDuck>())
    }).sum::<usize>()
}

pub(super) fn make(nodes: Vec<NodeProperties>) -> Chain {
    let chain: Chain = nodes.into();
    if tracing() {
        let mut diagnostics = DIAGNOSTICS.lock().unwrap();
        let diagnostics = diagnostics.get_or_insert_with(Default::default);
        diagnostics.created += 1;
        diagnostics.created_bytes += bytes(&chain);
        diagnostics.allocations.push(Arc::downgrade(&chain));
    }
    chain
}
pub(super) fn clone(chain: &Chain) -> Chain {
    if separate() { make(chain.to_vec()) } else { chain.clone() }
}
pub(super) fn requested(id: u32, chain: &Chain) {
    if !tracing() { return; }
    let mut crc = flate2::Crc::new();
    crc.update(format!("{chain:?}").as_bytes());
    let mut diagnostics = DIAGNOSTICS.lock().unwrap();
    *diagnostics.get_or_insert_with(Default::default).requests.entry((id, chain.len(), crc.sum())).or_default() += 1;
}
pub(crate) fn capture() {
    if !tracing() { return; }
    let mut diagnostics = DIAGNOSTICS.lock().unwrap();
    let Some(diagnostics) = diagnostics.as_mut() else { return; };
    let mut live = 0;
    let mut live_bytes = 0;
    let mut nodes = 0;
    diagnostics.allocations.retain(|weak| {
        let Some(chain) = weak.upgrade() else { return false; };
        live += 1;
        nodes += chain.len();
        live_bytes += bytes(&chain);
        true
    });
    let mut crc = flate2::Crc::new();
    for (&(id, count, content), &requests) in &diagnostics.requests {
        crc.update(&id.to_le_bytes());
        crc.update(&(count as u64).to_le_bytes());
        crc.update(&content.to_le_bytes());
        crc.update(&requests.to_le_bytes());
    }
    eprintln!("MEMORY sound property chains: created={} created_bytes={} live={live} live_bytes={live_bytes} live_nodes={nodes} requests={} identities={} fingerprint={:08x}",
        diagnostics.created, diagnostics.created_bytes, diagnostics.requests.values().sum::<u64>(), diagnostics.requests.len(), crc.sum());
}
