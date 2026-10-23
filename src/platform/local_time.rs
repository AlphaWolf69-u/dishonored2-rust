//! Local calendar time of a Unix timestamp through the C runtime's
//! `_localtime64_s`, the conversion the game's date formatting uses
//! (`FUN_140154990` calls `_localtime64`).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalTime {
    pub year: i32,
    /// 1..=12.
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

#[repr(C)]
#[derive(Default)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
}

#[cfg(target_os = "windows")]
unsafe extern "C" {
    fn _localtime64_s(tm: *mut Tm, time: *const i64) -> i32;
}

#[cfg(target_os = "windows")]
pub fn local_time(time: i64) -> LocalTime {
    let mut tm = Tm::default();
    if unsafe { _localtime64_s(&mut tm, &time) } != 0 {
        return LocalTime::default();
    }
    LocalTime { year: tm.year + 1900, month: tm.mon + 1, day: tm.mday, hour: tm.hour, minute: tm.min, second: tm.sec }
}

#[cfg(not(target_os = "windows"))]
pub fn local_time(time: i64) -> LocalTime {
    // The game targets Windows; elsewhere UTC is the only zone available here.
    let days = time.div_euclid(86400);
    let secs = time.rem_euclid(86400);
    let (y, m, d) = civil(days);
    LocalTime { year: y, month: m, day: d, hour: (secs / 3600) as i32, minute: (secs / 60 % 60) as i32, second: (secs % 60) as i32 }
}

#[cfg(not(target_os = "windows"))]
fn civil(days: i64) -> (i32, i32, i32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((yoe + era * 400 + (m <= 2) as i64) as i32, m as i32, d as i32)
}
