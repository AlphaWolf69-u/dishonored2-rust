//! The renderer's monitor and display-mode queries the visuals options read
//! (notes: `artifacts/ui-pause/notes-visuals-runtime.md`).

/// One monitor-table entry (`DAT_1432faf90`, 0x40 bytes).
#[derive(Clone, Debug, Default)]
pub struct Monitor {
    pub handle: isize,
    pub device: String,
    pub size: (i32, i32),
    pub origin: (i32, i32),
    pub primary: bool,
    /// `w << 16 | h`, ascending.
    pub modes: Vec<u32>,
    pub max_refresh: Vec<u32>,
}

/// `FUN_140345b40`'s table, plus the list helpers that read it.
#[derive(Clone, Debug, Default)]
pub struct Monitors {
    pub monitors: Vec<Monitor>,
}

impl Monitors {
    /// `FUN_140345a70` (vfunc +0x278): the mode `index` of the joined
    /// per-monitor lists as (w, h, monitor).
    pub fn mode(&self, index: i32) -> Option<(i32, i32, i32)> {
        if index < 0 || self.monitors.is_empty() {
            return None;
        }
        let mut index = index as usize;
        let mut mon = 0;
        while index >= self.monitors[mon].modes.len() {
            index -= self.monitors[mon].modes.len();
            mon += 1;
            if mon >= self.monitors.len() {
                return None;
            }
        }
        let p = self.monitors[mon].modes[index];
        Some(((p >> 16) as i32, (p & 0xffff) as i32, mon as i32))
    }

    /// `FUN_140348a80` (vfunc +0x190): the `r_mode` of (w, h) on `mon`, or
    /// that monitor's largest mode. Returns (new r_mode, w, h, changed).
    pub fn select(&self, old: i32, mon: i32, w: i32, h: i32) -> (i32, i32, i32, bool) {
        let (mut bw, mut bh) = (w, h);
        let mut r_mode = -1;
        let mut i = 0;
        while let Some((mw, mh, mm)) = self.mode(i) {
            if mm == mon {
                r_mode = i;
                if mw == w && mh == h {
                    return (r_mode, w, h, true);
                }
                (bw, bh) = (mw, mh);
            }
            i += 1;
        }
        (r_mode, bw, bh, r_mode != -1 && r_mode != old)
    }

    /// `FUN_140345890`: the window geometry cvars, centred on the
    /// monitor's largest mode. `None` when nothing is written.
    pub fn window(&self, mon: i32, w: i32, h: i32) -> Option<(i32, i32, i32, i32)> {
        if w == -1 || h == -1 {
            return None;
        }
        let m = self.monitors.get(mon.max(0) as usize)?;
        let p = *m.modes.last()?;
        let mut x = ((p >> 16).wrapping_sub(w as u32) >> 1) as i32;
        let mut y = ((p & 0xffff).wrapping_sub(h as u32) >> 1) as i32;
        if (0..self.monitors.len() as i32).contains(&mon) {
            x = x.wrapping_add(m.origin.0);
            y = y.wrapping_add(m.origin.1);
        }
        Some((w, h, x, y))
    }

    /// `FUN_1403460c0` (vfunc +0x2f0): the menu's modes. `dxgi` is the
    /// swapchain output's `GetDisplayModeList` (w, h) in its order; when
    /// empty the monitor's sorted list is used.
    pub fn menu_modes(&self, current: i32, dxgi: &[(u32, u32)]) -> Vec<(i32, i32)> {
        let Some(m) = self.monitors.get(current.max(0) as usize).filter(|_| current >= 0) else { return Vec::new() };
        let mut out = Vec::new();
        let mut push = |w: u32, h: u32| {
            let e = (w as i32, h as i32);
            if !out.contains(&e) {
                out.push(e);
            }
        };
        if dxgi.is_empty() {
            for &p in &m.modes {
                push(p >> 16, p & 0xffff);
            }
        } else {
            for &(w, h) in dxgi {
                if m.modes.contains(&(w << 16 | h)) {
                    push(w, h);
                }
            }
        }
        out
    }

    /// The borderless branch of `FUN_140345f40`: `r_mode`'s maximum
    /// refresh, walked with `>` as the engine does.
    pub fn borderless_refresh(&self, r_mode: i32) -> f32 {
        let mut i = r_mode.max(0) as usize;
        let mut mon = 0;
        while mon < self.monitors.len() && i > self.monitors[mon].modes.len() {
            i -= self.monitors[mon].modes.len();
            mon += 1;
        }
        // Reading one past a monitor's list reaches the next allocation in
        // the engine; here it reads nothing.
        self.monitors.get(mon).and_then(|m| m.max_refresh.get(i)).map_or(-1.0, |&r| r as f32)
    }
}

#[cfg(target_os = "windows")]
mod os {
    use super::{Monitor, Monitors};
    use std::ffi::c_void;

    const DEVMODE_SIZE: usize = 0x9c;
    const BITS_PER_PEL: usize = 104;
    const PELS_WIDTH: usize = 108;
    const PELS_HEIGHT: usize = 112;
    const DISPLAY_FREQUENCY: usize = 120;
    const ENUM_REGISTRY_SETTINGS: u32 = 0xffff_fffe;

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MonitorInfoEx {
        size: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
        device: [u8; 32],
    }

    #[repr(C)]
    struct DisplayDevice {
        cb: u32,
        name: [u8; 32],
        string: [u8; 128],
        state: u32,
        id: [u8; 128],
        key: [u8; 128],
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumDisplayMonitors(hdc: isize, clip: *const Rect, callback: unsafe extern "system" fn(isize, isize, *mut Rect, isize) -> i32, data: isize) -> i32;
        fn GetMonitorInfoA(monitor: isize, info: *mut MonitorInfoEx) -> i32;
        fn EnumDisplaySettingsExA(device: *const u8, mode: u32, devmode: *mut u8, flags: u32) -> i32;
        fn EnumDisplaySettingsA(device: *const u8, mode: u32, devmode: *mut u8) -> i32;
        fn EnumDisplayDevicesA(device: *const u8, index: u32, out: *mut DisplayDevice, flags: u32) -> i32;
    }

    unsafe extern "system" fn collect(monitor: isize, _: isize, _: *mut Rect, data: isize) -> i32 {
        unsafe { (*(data as *mut Vec<isize>)).push(monitor) };
        1
    }

    fn field(dm: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(dm[at..at + 4].try_into().unwrap())
    }

    fn c_str(bytes: &[u8]) -> String {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }

    fn info(handle: isize) -> Option<MonitorInfoEx> {
        let mut mi = MonitorInfoEx { size: 0x48, monitor: Rect { left: 0, top: 0, right: 0, bottom: 0 }, work: Rect { left: 0, top: 0, right: 0, bottom: 0 }, flags: 0, device: [0; 32] };
        (unsafe { GetMonitorInfoA(handle, &mut mi) } != 0).then_some(mi)
    }

    /// The 32-bit, at least 480-line modes of a device as (packed, refresh).
    fn device_modes(device: &[u8; 32]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut dm = [0u8; DEVMODE_SIZE + 0x400];
        dm[36..38].copy_from_slice(&(DEVMODE_SIZE as u16).to_le_bytes());
        let mut n = 0;
        while unsafe { EnumDisplaySettingsExA(device.as_ptr(), n, dm.as_mut_ptr(), 0) } != 0 {
            if field(&dm, BITS_PER_PEL) == 32 && field(&dm, PELS_HEIGHT) >= 480 {
                out.push((field(&dm, PELS_WIDTH) << 16 | field(&dm, PELS_HEIGHT), field(&dm, DISPLAY_FREQUENCY)));
            }
            n += 1;
        }
        out
    }

    /// `FUN_140345b40`.
    pub fn monitors() -> Monitors {
        let mut handles: Vec<isize> = Vec::new();
        unsafe { EnumDisplayMonitors(0, std::ptr::null(), collect, &mut handles as *mut _ as isize) };
        let mut monitors: Vec<Monitor> = handles.into_iter().map(|handle| Monitor { handle, ..Monitor::default() }).collect();
        let mut i = 0;
        while i < monitors.len() {
            if let Some(mi) = info(monitors[i].handle) {
                let m = &mut monitors[i];
                m.device = c_str(&mi.device);
                m.size = (mi.monitor.right - mi.monitor.left, mi.monitor.bottom - mi.monitor.top);
                m.origin = (mi.monitor.left, mi.monitor.top);
                m.primary = mi.monitor.left == 0 && mi.monitor.top == 0;
                let all = device_modes(&mi.device);
                for &(p, _) in &all {
                    if !m.modes.contains(&p) {
                        m.modes.push(p);
                    }
                }
                m.modes.sort_unstable();
                m.max_refresh = m.modes.iter().map(|p| all.iter().filter(|e| e.0 == *p).map(|e| e.1).max().unwrap_or(0)).collect();
            }
            if monitors[i].modes.is_empty() {
                // `FUN_1403479e0`, then i++: the monitor shifted into slot i
                // is skipped and keeps an empty mode list.
                monitors.remove(i);
            }
            i += 1;
        }
        Monitors { monitors }
    }

    /// `FUN_1403463a0` (vfunc +0x188).
    pub fn names(table: &Monitors) -> Vec<String> {
        let mut out = Vec::new();
        for m in &table.monitors {
            let device = info(m.handle).map(|mi| c_str(&mi.device)).unwrap_or_default();
            let mut name = None;
            let mut a = 0;
            loop {
                let mut adapter = DisplayDevice { cb: 0x1a8, name: [0; 32], string: [0; 128], state: 0, id: [0; 128], key: [0; 128] };
                if unsafe { EnumDisplayDevicesA(std::ptr::null(), a, &mut adapter, 0) } == 0 {
                    break;
                }
                let mut mon = DisplayDevice { cb: 0x1a8, name: [0; 32], string: [0; 128], state: 0, id: [0; 128], key: [0; 128] };
                if unsafe { EnumDisplayDevicesA(adapter.name.as_ptr(), 0, &mut mon, 0) } != 0 {
                    let string = c_str(&mon.string);
                    if c_str(&mon.name).contains(&device) && !string.is_empty() {
                        name = Some(string);
                        break;
                    }
                }
                a += 1;
            }
            out.push(name.unwrap_or_else(|| "Generic Plug'n'Play".into()));
        }
        out
    }

    /// The windowed branch of `FUN_140345f40`: the registry refresh rate.
    pub fn registry_refresh(m: &Monitor) -> f32 {
        let Some(mi) = info(m.handle) else { return -1.0 };
        let mut dm = [0u8; DEVMODE_SIZE + 0x400];
        dm[36..38].copy_from_slice(&(DEVMODE_SIZE as u16).to_le_bytes());
        if unsafe { EnumDisplaySettingsA(mi.device.as_ptr(), ENUM_REGISTRY_SETTINGS, dm.as_mut_ptr()) } != 0 {
            field(&dm, DISPLAY_FREQUENCY) as f32
        } else {
            -1.0
        }
    }

    type Hresult = i32;

    #[link(name = "dxgi")]
    unsafe extern "system" {
        fn CreateDXGIFactory1(riid: *const [u8; 16], factory: *mut *mut c_void) -> Hresult;
    }

    /// IID_IDXGIFactory1 {770aae78-f26f-4dba-a829-253c83d1b387}.
    const IID_FACTORY1: [u8; 16] = [0x78, 0xae, 0x0a, 0x77, 0x6f, 0xf2, 0xba, 0x4d, 0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87];

    unsafe fn method(object: *mut c_void, slot: usize) -> *const c_void {
        unsafe { *(*(object as *const *const *const c_void)).add(slot) }
    }

    unsafe fn release(object: *mut c_void) {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { std::mem::transmute(method(object, 2)) };
        unsafe { f(object) };
    }

    /// `IDXGIOutput::GetDisplayModeList(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, 0)`
    /// on the output of `monitor` (the swapchain's containing output when
    /// the window is on it), as (w, h) in DXGI order.
    pub fn dxgi_modes(monitor: isize) -> Vec<(u32, u32)> {
        unsafe {
            let mut factory = std::ptr::null_mut();
            if CreateDXGIFactory1(&IID_FACTORY1, &mut factory) < 0 {
                return Vec::new();
            }
            let enum_adapters1: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> Hresult = std::mem::transmute(method(factory, 12));
            let mut out = Vec::new();
            let mut a = 0;
            loop {
                let mut adapter = std::ptr::null_mut();
                if enum_adapters1(factory, a, &mut adapter) < 0 {
                    break;
                }
                let enum_outputs: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> Hresult = std::mem::transmute(method(adapter, 7));
                let mut o = 0;
                loop {
                    let mut output = std::ptr::null_mut();
                    if enum_outputs(adapter, o, &mut output) < 0 {
                        break;
                    }
                    // DXGI_OUTPUT_DESC: DeviceName[32] wchar, RECT, BOOL, rotation, HMONITOR.
                    let mut desc = [0u8; 64 + 16 + 4 + 4 + 8];
                    let get_desc: unsafe extern "system" fn(*mut c_void, *mut u8) -> Hresult = std::mem::transmute(method(output, 7));
                    if get_desc(output, desc.as_mut_ptr()) >= 0 && isize::from_le_bytes(desc[88..96].try_into().unwrap()) == monitor {
                        let list: unsafe extern "system" fn(*mut c_void, u32, u32, *mut u32, *mut u8) -> Hresult = std::mem::transmute(method(output, 8));
                        let mut count = 0u32;
                        if list(output, 0x1d, 0, &mut count, std::ptr::null_mut()) >= 0 && count > 0 {
                            let mut modes = vec![0u8; count as usize * 0x1c];
                            if list(output, 0x1d, 0, &mut count, modes.as_mut_ptr()) >= 0 {
                                for m in modes.chunks_exact(0x1c).take(count as usize) {
                                    out.push((u32::from_le_bytes(m[0..4].try_into().unwrap()), u32::from_le_bytes(m[4..8].try_into().unwrap())));
                                }
                            }
                        }
                    }
                    release(output);
                    o += 1;
                }
                release(adapter);
                a += 1;
            }
            release(factory);
            out
        }
    }
}

#[cfg(target_os = "windows")]
pub use os::{dxgi_modes, monitors, names, registry_refresh};

#[cfg(not(target_os = "windows"))]
pub fn monitors() -> Monitors {
    Monitors::default()
}
#[cfg(not(target_os = "windows"))]
pub fn names(_: &Monitors) -> Vec<String> {
    Vec::new()
}
#[cfg(not(target_os = "windows"))]
pub fn registry_refresh(_: &Monitor) -> f32 {
    -1.0
}
#[cfg(not(target_os = "windows"))]
pub fn dxgi_modes(_: isize) -> Vec<(u32, u32)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_join_monitors_and_select_like_the_engine() {
        let table = Monitors {
            monitors: vec![
                Monitor { modes: vec![1280 << 16 | 720, 1920 << 16 | 1080], max_refresh: vec![60, 144], ..Monitor::default() },
                Monitor { origin: (1920, 0), modes: vec![1024 << 16 | 768], max_refresh: vec![60], ..Monitor::default() },
            ],
        };
        assert_eq!(table.mode(1), Some((1920, 1080, 0)));
        assert_eq!(table.mode(2), Some((1024, 768, 1)));
        assert_eq!(table.mode(3), None);
        // No exact match falls back to the monitor's largest mode.
        assert_eq!(table.select(0, 0, 800, 600), (1, 1920, 1080, true));
        assert_eq!(table.window(1, 800, 600), Some((800, 600, 1920 + 112, 84)));
        assert_eq!(table.menu_modes(0, &[(1920, 1080), (640, 480), (1280, 720)]), vec![(1920, 1080), (1280, 720)]);
        // `>` walk: r_mode 2 reads past monitor 0's list.
        assert_eq!(table.borderless_refresh(1), 144.0);
    }

    #[test]
    #[ignore = "reads this machine's displays"]
    fn debug_list_displays() {
        let table = monitors();
        for (m, name) in table.monitors.iter().zip(names(&table)) {
            println!("{name} {} {:?} {:?} modes {}", m.device, m.size, m.origin, m.modes.len());
            println!("  dxgi {:?}", dxgi_modes(m.handle));
        }
    }
}
