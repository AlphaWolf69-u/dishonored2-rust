//! Process memory counters for the gameplay performance overlay.

#[derive(Clone, Copy, Debug)]
pub struct ProcessMemory {
    /// Pages currently resident in physical RAM, including shared pages.
    pub resident_bytes: usize,
    /// Private committed memory, including pages that may be paged out.
    pub private_bytes: usize,
}

/// Samples the current process without walking allocations or GPU resources.
#[cfg(target_os = "windows")]
pub fn sample() -> Option<ProcessMemory> {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        size: u32,
        page_fault_count: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged_pool: usize,
        quota_paged_pool: usize,
        quota_peak_nonpaged_pool: usize,
        quota_nonpaged_pool: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    let size = std::mem::size_of::<Counters>() as u32;
    let mut counters = Counters {
        size,
        ..Default::default()
    };
    // -1 is the documented current-process pseudo-handle. The structure size
    // selects PROCESS_MEMORY_COUNTERS_EX, including its PrivateUsage field.
    let result = unsafe { K32GetProcessMemoryInfo(-1, &mut counters, size) };
    (result != 0).then_some(ProcessMemory {
        resident_bytes: counters.working_set,
        private_bytes: counters.private_usage,
    })
}

#[cfg(not(target_os = "windows"))]
pub fn sample() -> Option<ProcessMemory> {
    None
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    #[test]
    fn current_process_counters_are_available() {
        let memory = super::sample().expect("current process memory counters");
        assert!(memory.resident_bytes > 0);
        assert!(memory.private_bytes > 0);
    }
}
