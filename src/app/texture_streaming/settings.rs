//! Installed streamer controls (14030d1e0, 14030cce0, 14030f3d0).
//! Host: cache cvar parsing by console generation, like other settings readers.
#[derive(Clone, Copy)]
pub(super) struct Settings {
    pub upload_kb: f32,
    pub max_texture_kb: i32,
    pub unload_frames: u32,
    pub add_all_mips: bool,
    pub frozen: bool,
    pub priority_multiplier: bool,
    pub memory_budget_mb: u64,
}

#[derive(Default)]
pub(crate) struct SettingsCache {
    current: Option<(u64, Settings)>,
    adapter_memory: Option<Option<u64>>,
    automatic_budget: Option<u64>,
}

impl SettingsCache {
    pub(super) fn get(&mut self, adapter: Option<&bevy::render::renderer::RenderAdapter>) -> Settings {
        let shared = crate::platform::console::shared();
        let generation = shared.generation();
        let discovered_adapter = self.adapter_memory.is_none() && adapter.is_some();
        if let Some(adapter) = adapter.filter(|_| discovered_adapter) {
            self.adapter_memory = Some(adapter.dedicated_memory_bytes());
        }
        if let Some((cached, settings)) = self.current.filter(|_| !discovered_adapter) {
            if cached == generation { return settings; }
        }
        let console = shared.read();
        let configured = console.cvars.int("r_textureStreamerMemoryBudgetMB").unwrap_or(-1);
        let miscellaneous = console.cvars.int("r_vramMiscSizeEstimationMB").unwrap_or(2560);
        // 140369eed..140369f8f: resolve automatic capacity once; failed/too
        // small hardware reports use 256 MiB, with a 4096 MiB upper limit.
        if self.automatic_budget.is_none() && self.adapter_memory.is_some() {
            self.automatic_budget = Some(self.adapter_memory.flatten().map(|bytes|
                ((bytes >> 20) as i64 - i64::from(miscellaneous)).clamp(256, 4096) as u64).unwrap_or(256));
        }
        let settings = Settings {
            upload_kb: console.cvars.float("r_textureStreamingPerFrameBudgetKB").unwrap_or(super::TEXTURE_UPLOAD_BUDGET_KB),
            max_texture_kb: console.cvars.int("r_textureStreamingMaxTextureSizeKB").unwrap_or(131_072),
            unload_frames: console.cvars.int("r_maxFramesBeforeTextureUnload").unwrap_or(super::TEXTURE_REDUCE_AFTER_FRAMES as i32) as u32,
            add_all_mips: console.cvars.int("r_addAllMips").unwrap_or(0) != 0,
            frozen: console.cvars.int("r_freezeTextureStreaming").unwrap_or(0) != 0,
            priority_multiplier: console.cvars.int("r_skipAnisoTextureStreaming").unwrap_or(0) == 0,
            memory_budget_mb: if configured < 0 { self.automatic_budget.unwrap_or(256) } else { configured as u64 },
        };
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY texture streaming settings: upload_kb={} max_texture_kb={} unload_frames={} add_all_mips={} frozen={} memory_budget_mb={} adapter_memory_bytes={:?}",
                settings.upload_kb, settings.max_texture_kb, settings.unload_frames, settings.add_all_mips, settings.frozen,
                settings.memory_budget_mb, self.adapter_memory.flatten());
        }
        self.current = Some((generation, settings));
        settings
    }
}
