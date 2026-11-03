//! Host diagnostic for first-use preparation stalls. Never changes scheduling.
pub struct LoadingSpan(&'static str, Option<std::time::Instant>);

impl LoadingSpan {
    pub fn new(label: &'static str) -> Self {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        Self(label, ENABLED.get_or_init(|| {
            std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1")
        }).then(std::time::Instant::now))
    }
}

impl Drop for LoadingSpan {
    fn drop(&mut self) {
        if let Some(start) = self.1 {
            let elapsed = start.elapsed();
            // Log stalls, not every draw or asset frame.
            if elapsed.as_millis() >= 10 {
                eprintln!("LOAD backend {}: {:.3} ms", self.0, elapsed.as_secs_f64() * 1000.0);
            }
        }
    }
}
