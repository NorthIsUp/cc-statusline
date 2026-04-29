// Visible length calculation — strips ANSI/OSC-8 escapes, then counts cells
// using the same width rules as the bash perl helper.

pub fn nf_width() -> u32 {
    // Default to 2: most modern Nerd Font Mono fonts in iTerm2/Alacritty
    // render PUA glyphs as 2 cells wide. Override to 1 via env or config if
    // your font renders them single-wide.
    std::env::var("CC_STATUSLINE_NF_WIDTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|w: &u32| *w == 1 || *w == 2)
        .unwrap_or_else(|| crate::config::config().nerd_font_width())
}

pub fn strip(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for n in chars.by_ref() {
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    // OSC: terminated by ESC \ or BEL.
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' {
                            chars.next(); // consume the trailing \
                            break;
                        }
                    }
                }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn cell_width(c: char) -> u32 {
    let cp = c as u32;
    let nfw = nf_width();
    // Nerd Font PUA ranges → configurable width
    if (0xE000..=0xF8FF).contains(&cp) || (0xF0000..=0xFFFFD).contains(&cp) {
        return nfw;
    }
    // Wide ranges: emoji, CJK, Hangul, etc.
    if (0x1F300..=0x1FAFF).contains(&cp)
        || (0x2300..=0x23FF).contains(&cp)
        || (0x2600..=0x27BF).contains(&cp)
        || (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0x9FFF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
    {
        return 2;
    }
    1
}

pub fn vlen(s: &str) -> u32 {
    strip(s).chars().map(cell_width).sum()
}

pub fn truncate_to_width(s: &str, max: u32) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = cell_width(c);
        if w + cw > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}
