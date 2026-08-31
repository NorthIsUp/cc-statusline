// Per-session cache directory + freshness helpers.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn session_dir(session_id: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/cc-statusline-{session_id}"))
}

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Broken-down local time for a unix `epoch` via `localtime_r` (reads the
/// system timezone). Replaces shelling out to `date -r`.
pub fn local_tm(epoch: i64) -> Option<libc::tm> {
    let t = epoch as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        None
    } else {
        Some(tm)
    }
}

pub fn mtime(p: &Path) -> Option<i64> {
    let md = std::fs::metadata(p).ok()?;
    md.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

pub fn fresh(p: &Path, ttl: i64) -> bool {
    match mtime(p) {
        Some(m) => now_epoch() - m < ttl,
        None => false,
    }
}
