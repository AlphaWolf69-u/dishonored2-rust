//! On-demand numeric evidence for the selected adaptive-depth preparation.
use super::shadow_updates::ParallelShadowCpu;
use std::fmt::Write;

pub(super) fn append_adaptive_input(report: &mut String, cpu: &ParallelShadowCpu) {
    report.push_str("\nAdaptive shadow preparation input (CPU attempt, mapped GPU depth):\n");
    let Some(input) = &cpu.adaptive_input else {
        report.push_str("not selected in this preparation\n");
        return;
    };
    let source = &input.snapshot.metadata;
    writeln!(report, "frame={} source_sequence={} source_extent={:?} reference_interval={:?} effective_far={} effective_far_bits={:08x}",
        input.frame,source.sequence,source.source_extent,input.reference_interval,input.effective_far,input.effective_far.to_bits()).unwrap();
    writeln!(report, "current_projection={:?}\ncurrent_inverse_projection={:?}\nsource_projection={:?}\nsource_inverse_projection={:?}\nsource_view_from_world={:?}\nfar_state={:?}",
        input.projection,input.inverse_projection,source.projection,source.inverse_projection,source.view_from_world,cpu.prepared.far_state).unwrap();
    // Both raw R/G channels in source row order. G is the exact adaptive scan
    // input; keeping R also allows the producer/reduction contract to be tested.
    report.push_str("coarse_depth_rg_f32_bits_64x64:\n");
    for row in input.snapshot.pixels.chunks_exact(64) {
        for [r, g] in row {
            write!(report, " {r:08x}/{g:08x}", r = r.to_bits(), g = g.to_bits()).unwrap();
        }
        report.push('\n');
    }
}
