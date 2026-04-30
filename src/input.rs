// Stdin JSON parsing for the Claude Code statusline contract.

use serde_json::Value;

#[derive(Debug, Default)]
pub struct Session {
    pub model: String,
    pub cwd: String,
    pub session_id: String,
    pub ctx_pct: u32,
    pub cost: f64,
    pub wt_branch: String,
    pub r5h: Option<u32>,
    pub r7d: Option<u32>,
    pub r5h_reset: Option<i64>,
    pub r7d_reset: Option<i64>,
    pub transcript: String,
    pub effort: String,
    pub fast_mode: bool,
    pub output_style: String,
    pub duration_ms: i64,
    pub cols: u32,
}

fn s(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for k in path {
        cur = match cur.get(*k) {
            Some(c) => c,
            None => return String::new(),
        };
    }
    cur.as_str().unwrap_or("").to_string()
}

fn n_u32(v: &Value, path: &[&str]) -> Option<u32> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k)?;
    }
    cur.as_f64().map(|x| x as u32)
}

fn n_i64(v: &Value, path: &[&str]) -> i64 {
    let mut cur = v;
    for k in path {
        match cur.get(*k) {
            Some(c) => cur = c,
            None => return 0,
        }
    }
    cur.as_f64().map(|x| x as i64).unwrap_or(0)
}

pub fn ts_to_epoch(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    // RFC3339 / ISO8601 — best-effort. Format: YYYY-MM-DDTHH:MM:SS(...).
    // We only need approximate seconds for the quota math; missing fractional
    // seconds and timezones default to UTC.
    parse_iso8601_to_epoch(s)
}

fn parse_iso8601_to_epoch(s: &str) -> Option<i64> {
    // Hand-rolled tiny parser to avoid pulling chrono. Accepts strings like
    // 2026-04-28T15:00:00Z or 2026-04-28T15:00:00+00:00.
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let year: i64 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let month: u32 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let day: u32 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    let hour: i64 = std::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
    let min: i64 = std::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
    let sec: i64 = std::str::from_utf8(&bytes[17..19]).ok()?.parse().ok()?;
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    // Howard Hinnant's date algorithm — days since 1970-01-01.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as i64;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

impl Session {
    pub fn parse(buf: &str) -> Self {
        let v: Value = serde_json::from_str(buf).unwrap_or(Value::Null);
        let mut model = s(&v, &["model", "display_name"]);
        if model.is_empty() {
            model = "?".into();
        }
        let mut session_id = s(&v, &["session_id"]);
        if session_id.is_empty() {
            session_id = "nosession".into();
        }
        let cols = ["terminal", "width"]
            .iter()
            .find_map(|_| n_u32(&v, &["terminal", "width"]))
            .or_else(|| n_u32(&v, &["terminal", "columns"]))
            .or_else(|| n_u32(&v, &["window", "columns"]))
            .or_else(|| n_u32(&v, &["cols"]))
            .unwrap_or(0);

        let mut effort = s(&v, &["effort_level"]);
        if effort.is_empty() {
            effort = s(&v, &["effortLevel"]);
        }
        if effort.is_empty() {
            effort = s(&v, &["output_style", "effort"]);
        }
        if effort.is_empty() {
            // Settings file fallback.
            if let Ok(home) = std::env::var("HOME") {
                let p = format!("{home}/.claude/settings.json");
                if let Ok(txt) = std::fs::read_to_string(&p) {
                    if let Ok(sj) = serde_json::from_str::<Value>(&txt) {
                        effort = sj
                            .get("effortLevel")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                }
            }
        }

        Self {
            model,
            cwd: s(&v, &["workspace", "current_dir"]),
            session_id,
            ctx_pct: n_u32(&v, &["context_window", "used_percentage"]).unwrap_or(0),
            cost: v
                .get("cost")
                .and_then(|c| c.get("total_cost_usd"))
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            wt_branch: s(&v, &["workspace", "git_worktree"]),
            r5h: n_u32(&v, &["rate_limits", "five_hour", "used_percentage"]),
            r7d: n_u32(&v, &["rate_limits", "seven_day", "used_percentage"]),
            r5h_reset: ts_to_epoch(&s(&v, &["rate_limits", "five_hour", "resets_at"])),
            r7d_reset: ts_to_epoch(&s(&v, &["rate_limits", "seven_day", "resets_at"])),
            transcript: s(&v, &["transcript_path"]),
            effort,
            fast_mode: v
                .get("fast_mode")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            output_style: s(&v, &["output_style", "name"]),
            duration_ms: n_i64(&v, &["cost", "total_duration_ms"]),
            cols,
        }
    }
}
