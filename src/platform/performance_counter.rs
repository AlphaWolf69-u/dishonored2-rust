//! Windows performance counter shared by gameplay clocks and random seeds.
#[cfg(target_os = "windows")]
mod implementation {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceCounter(value: *mut i64) -> i32;
        fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    }
    pub fn counter() -> Result<i64, String> {
        let mut value = 0;
        if unsafe { QueryPerformanceCounter(&mut value) } == 0 {
            return Err(format!(
                "QueryPerformanceCounter failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(value)
    }
    pub fn frequency() -> Result<i64, String> {
        let mut value = 0;
        if unsafe { QueryPerformanceFrequency(&mut value) } == 0 || value <= 0 {
            return Err(format!(
                "QueryPerformanceFrequency failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(value)
    }
}

#[cfg(not(target_os = "windows"))]
mod implementation {
    pub fn counter() -> Result<i64, String> {
        Err("QPC clock requires Windows".into())
    }
    pub fn frequency() -> Result<i64, String> {
        Err("QPC clock requires Windows".into())
    }
}

pub use implementation::{counter, frequency};
