//! Physics startup07b600 and Havok worker contextaa8f30 enable FTZ.
//! Bevy can move a physics/query job to a previously unrelated worker, so
//! initialize each executing thread lazily at its first physics boundary.
//! This is persistent thread state, as in the source, not a temporary draw or
//! body scope. DAZ, rounding and exception-mask/status bits remain unchanged.
use std::cell::Cell;

std::thread_local! {
    static INITIALIZED: Cell<bool> = const { Cell::new(false) };
}

#[inline]
pub(crate) fn enter() {
    INITIALIZED.with(|initialized| {
        if !initialized.get() {
            initialize();
            initialized.set(true);
        }
    });
}

#[cold]
#[inline(never)]
fn initialize() {
    #[cfg(not(target_arch = "x86_64"))]
    unimplemented!("physics floating-point initialization requires x86_64 MXCSR");
    #[cfg(target_arch = "x86_64")]
    {
        let mut control = 0u32;
        // SAFETY: x86_64 guarantees SSE/MXCSR. The stack pointer is valid for
        // one u32; only supported bit15 is added, retaining all reserved bits.
        unsafe {
            std::arch::asm!("stmxcsr [{address}]", address = in(reg) &mut control,
                options(nostack, preserves_flags));
            if control & 0x8000 == 0 {
                control |= 0x8000;
                std::arch::asm!("ldmxcsr [{address}]", address = in(reg) &control,
                    options(nostack, preserves_flags));
            }
        }
    }
}
