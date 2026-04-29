// Layout template DSL — Starship-inspired `${name}` / `${name:variant}`
// substitution over a `LayoutContext` of pre-rendered segment strings.
//
// Empty segments collapse adjacent literal whitespace so a template like
// `"${a} ${b}"` with `b == ""` renders as `"a"` instead of `"a "`.

use std::collections::HashMap;

/// All segment values are strings (which may embed ANSI escapes / OSC 8
/// hyperlinks). Resolution is a pure lookup — no I/O at evaluation time.
#[derive(Debug, Default, Clone)]
pub struct LayoutContext {
    /// Map from `(name, variant)` → pre-rendered string. Variant `""` is the
    /// default form. An entry whose value is empty is treated as a soft empty
    /// (collapses whitespace); a missing entry behaves the same.
    pub vars: HashMap<(String, String), String>,
}

impl LayoutContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert the default form of a variable.
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.vars.insert((name.into(), String::new()), value.into());
    }

    /// Insert a named variant of a variable (e.g. `model:long`).
    pub fn set_variant(&mut self, name: &str, variant: &str, value: impl Into<String>) {
        self.vars
            .insert((name.into(), variant.into()), value.into());
    }

    fn lookup(&self, name: &str, variant: &str) -> Option<&str> {
        self.vars
            .get(&(name.to_string(), variant.to_string()))
            .map(|s| s.as_str())
            .or_else(|| {
                if !variant.is_empty() {
                    // Fall back to default form if variant not provided.
                    self.vars
                        .get(&(name.to_string(), String::new()))
                        .map(|s| s.as_str())
                } else {
                    None
                }
            })
    }
}

/// Parsed token in a template string.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(String),
    Var { name: String, variant: String },
}

/// Parse `${name}` and `${name:variant}` placeholders. Unknown / malformed
/// forms are emitted as literals (best-effort, never panics).
fn parse(template: &str) -> Vec<Token> {
    let bytes = template.as_bytes();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = template[i + 2..].find('}') {
                let inner = &template[i + 2..i + 2 + end];
                let (name, variant) = match inner.split_once(':') {
                    Some((n, v)) => (n.trim().to_string(), v.trim().to_string()),
                    None => (inner.trim().to_string(), String::new()),
                };
                if !name.is_empty() {
                    if !buf.is_empty() {
                        out.push(Token::Literal(std::mem::take(&mut buf)));
                    }
                    out.push(Token::Var { name, variant });
                    i += 2 + end + 1;
                    continue;
                }
            }
        }
        // Push the byte we couldn't interpret as a placeholder. UTF-8 safe:
        // step by char rather than byte.
        let ch = template[i..].chars().next().unwrap();
        buf.push(ch);
        i += ch.len_utf8();
    }
    if !buf.is_empty() {
        out.push(Token::Literal(buf));
    }
    out
}

/// Evaluate `template` against `ctx`. Empty variable values cause adjacent
/// literal whitespace to collapse: a sequence `<ws>${empty}<ws>` between two
/// non-empty segments collapses to a single space; at the start or end of
/// the output it collapses to nothing.
pub fn eval(template: &str, ctx: &LayoutContext) -> String {
    let tokens = parse(template);
    // Two-pass approach. Pass 1 builds a flat sequence of "atoms":
    //   - Content(s)  → resolved variable or literal non-whitespace run
    //   - Ws(s)       → literal whitespace run (kept as-is, e.g. "  " or " · "
    //                    no — mixed runs are Content)
    //   - EmptyVar    → variable that resolved to empty / missing
    // Pass 2 collapses `Ws* EmptyVar Ws*` runs.
    enum Atom {
        Content(String),
        Ws(String),
        Empty,
    }
    let mut atoms: Vec<Atom> = Vec::new();
    for t in tokens {
        match t {
            Token::Literal(s) => {
                // Split literal into alternating ws / non-ws runs so we can
                // collapse whitespace adjacent to empty vars without losing
                // non-whitespace content.
                let mut cur = String::new();
                let mut cur_is_ws = true;
                let mut started = false;
                for ch in s.chars() {
                    let is_ws = ch == ' ' || ch == '\t';
                    if !started {
                        cur_is_ws = is_ws;
                        started = true;
                    }
                    if is_ws == cur_is_ws {
                        cur.push(ch);
                    } else {
                        if !cur.is_empty() {
                            atoms.push(if cur_is_ws {
                                Atom::Ws(std::mem::take(&mut cur))
                            } else {
                                Atom::Content(std::mem::take(&mut cur))
                            });
                        }
                        cur_is_ws = is_ws;
                        cur.push(ch);
                    }
                }
                if started && !cur.is_empty() {
                    atoms.push(if cur_is_ws {
                        Atom::Ws(cur)
                    } else {
                        Atom::Content(cur)
                    });
                }
            }
            Token::Var { name, variant } => {
                match ctx.lookup(&name, &variant).filter(|s| !s.is_empty()) {
                    Some(v) => atoms.push(Atom::Content(v.to_string())),
                    None => atoms.push(Atom::Empty),
                }
            }
        }
    }

    // Collapse: locate each Empty and remove all adjacent Ws runs on both
    // sides. If Content exists on both sides after collapse, insert a single
    // " " in place. If at start/end, insert nothing.
    // Simpler: process linearly emitting to a new vec, tracking whether we
    // "owe" a space due to a collapsed empty between content.
    let mut out_atoms: Vec<Atom> = Vec::new();
    let mut i = 0;
    while i < atoms.len() {
        match &atoms[i] {
            Atom::Empty => {
                // Drop trailing Ws atoms already in out_atoms.
                while matches!(out_atoms.last(), Some(Atom::Ws(_))) {
                    out_atoms.pop();
                }
                // Skip following Ws atoms.
                let mut j = i + 1;
                while matches!(atoms.get(j), Some(Atom::Ws(_))) {
                    j += 1;
                }
                // If both sides had Content, leave a single space placeholder.
                let left_has_content = matches!(out_atoms.last(), Some(Atom::Content(_)));
                let right_has_content = matches!(atoms.get(j), Some(Atom::Content(_)));
                if left_has_content && right_has_content {
                    out_atoms.push(Atom::Ws(" ".to_string()));
                }
                i = j;
            }
            _ => {
                // Move atom (clone since we still iterate by ref).
                out_atoms.push(match &atoms[i] {
                    Atom::Content(s) => Atom::Content(s.clone()),
                    Atom::Ws(s) => Atom::Ws(s.clone()),
                    Atom::Empty => unreachable!(),
                });
                i += 1;
            }
        }
    }
    // Strip leading/trailing whitespace atoms.
    while matches!(out_atoms.first(), Some(Atom::Ws(_))) {
        out_atoms.remove(0);
    }
    while matches!(out_atoms.last(), Some(Atom::Ws(_))) {
        out_atoms.pop();
    }

    let mut out = String::new();
    for a in out_atoms {
        match a {
            Atom::Content(s) | Atom::Ws(s) => out.push_str(&s),
            Atom::Empty => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_from(pairs: &[(&str, &str)]) -> LayoutContext {
        let mut c = LayoutContext::new();
        for (k, v) in pairs {
            if let Some((n, var)) = k.split_once(':') {
                c.set_variant(n, var, *v);
            } else {
                c.set(k, *v);
            }
        }
        c
    }

    #[test]
    fn template_substitutes_variables() {
        let ctx = ctx_from(&[
            ("repo", "foo/bar"),
            ("branch", "main"),
            ("model:long", "Claude Opus 4.7"),
            ("effort:icon", "⚡"),
            ("effort:short", "M"),
        ]);
        let s = eval(
            "${repo} ${branch} [${model:long} ${effort:icon}${effort:short}]",
            &ctx,
        );
        assert_eq!(s, "foo/bar main [Claude Opus 4.7 ⚡M]");
    }

    #[test]
    fn template_collapses_empty_segments() {
        let ctx = ctx_from(&[("a", "a"), ("b", "")]);
        // Trailing empty: "${a} ${b}" → "a"
        assert_eq!(eval("${a} ${b}", &ctx), "a");
        // Middle empty: "${a} ${b} ${a}" → "a a" (single space, not "a  a")
        let mut ctx2 = ctx.clone();
        ctx2.set("c", "c");
        assert_eq!(eval("${a} ${b} ${c}", &ctx2), "a c");
        // Leading empty: "${b} ${a}" → "a"
        assert_eq!(eval("${b} ${a}", &ctx), "a");
    }

    #[test]
    fn template_unknown_variable_treated_as_empty() {
        let ctx = ctx_from(&[("a", "a")]);
        assert_eq!(eval("${a} ${nope}", &ctx), "a");
        assert_eq!(eval("${nope} ${a}", &ctx), "a");
    }

    #[test]
    fn template_variant_falls_back_to_default() {
        let ctx = ctx_from(&[("model", "Opus")]);
        assert_eq!(eval("${model:long}", &ctx), "Opus");
    }

    #[test]
    fn template_preserves_literal_punctuation() {
        let ctx = ctx_from(&[("a", "x"), ("b", "y")]);
        assert_eq!(eval("${a} · ${b}", &ctx), "x · y");
        // When b is empty the " · " separator collapses too.
        let ctx2 = ctx_from(&[("a", "x"), ("b", "")]);
        assert_eq!(eval("${a} · ${b}", &ctx2), "x ·");
    }

    #[test]
    fn template_malformed_passes_through() {
        let ctx = ctx_from(&[("a", "x")]);
        assert_eq!(eval("$ {a} ${a}", &ctx), "$ {a} x");
        assert_eq!(eval("${unclosed", &ctx), "${unclosed");
    }
}
