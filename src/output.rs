use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

use chrono::Local;
use regex::Regex;

/// Global verbose flag
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// ANSI color codes
static COLORS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("cyan", "\x1b[36m");
    m.insert("yellow", "\x1b[33m");
    m.insert("magenta", "\x1b[35m");
    m.insert("green", "\x1b[32m");
    m.insert("blue", "\x1b[34m");
    m.insert("orange", "\x1b[38;5;208m");
    m.insert("red", "\x1b[31m");
    m.insert("lavender", "\x1b[38;5;183m");
    m.insert("pink", "\x1b[38;5;213m");
    m.insert("teal", "\x1b[38;5;51m");
    m.insert("lime", "\x1b[38;5;154m");
    m.insert("coral", "\x1b[38;5;209m");
    m.insert("sky", "\x1b[38;5;117m");
    m.insert("gold", "\x1b[38;5;220m");
    m.insert("violet", "\x1b[38;5;135m");
    m.insert("reset", "\x1b[0m");
    m.insert("dim", "\x1b[2m");
    m.insert("bold", "\x1b[1m");
    m
});

/// Deterministic color palette for services (assigned by index)
pub const SERVICE_COLORS: &[&str] = &[
    "cyan", "yellow", "magenta", "teal", "green", "blue", "orange", "pink", "lavender", "violet",
    "lime", "coral", "sky", "gold",
];

/// Check if stdout is a TTY
pub fn is_tty() -> bool {
    io::stdout().is_terminal()
}

/// Get ANSI color code, returns empty string if not a TTY
pub fn c(color: &str) -> &str {
    if !is_tty() {
        return "";
    }
    COLORS.get(color).copied().unwrap_or("")
}

/// Get raw ANSI code (always, even when not TTY — for stderr)
pub fn c_raw(color: &str) -> &str {
    COLORS.get(color).copied().unwrap_or("")
}

pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Safe print handling broken pipe gracefully
pub fn print(msg: &str) {
    if writeln!(io::stdout(), "{}", msg).is_err() {
        // Broken pipe — exit cleanly
        std::process::exit(0);
    }
}

/// Format a timestamp for log output
fn timestamp() -> String {
    Local::now().format("%H:%M:%S%.3f").to_string()
}

/// Log a message with optional prefix and color
pub fn log(msg: &str, prefix: &str, color: &str) {
    let ts = timestamp();
    let prefix_str = if !prefix.is_empty() {
        format!("{}{:<12}{} ", c(color), prefix, c("reset"))
    } else {
        String::new()
    };
    print(&format!(
        "{}{}{} {}{}",
        c("dim"),
        ts,
        c("reset"),
        prefix_str,
        msg
    ));
}

pub fn log_system(msg: &str) {
    log(msg, "rig", "bold");
}

pub fn log_error(msg: &str) {
    let ts = timestamp();
    let is_tty = io::stderr().is_terminal();
    let dim = if is_tty { c_raw("dim") } else { "" };
    let reset = if is_tty { c_raw("reset") } else { "" };
    let red = if is_tty { c_raw("red") } else { "" };
    let _ = writeln!(
        io::stderr(),
        "{}{}{} {}{:<12}{} {}",
        dim,
        ts,
        reset,
        red,
        "rig",
        reset,
        msg
    );
}

pub fn log_verbose(msg: &str) {
    if is_verbose() {
        log(msg, "rig", "dim");
    }
}

/// Strip ANSI control codes that would mess up log prefixes.
/// Preserves color codes but removes cursor movement, line clearing, etc.
pub fn strip_control_codes(line: &str) -> String {
    static RE_CR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r").unwrap());
    static RE_CURSOR_MOVE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\[\d*[ABCD]").unwrap());
    static RE_CURSOR_POS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\[\d*;\d*[Hf]").unwrap());
    static RE_CURSOR_COL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[\d*G").unwrap());
    static RE_CLEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[\d*[JK]").unwrap());
    static RE_CURSOR_VIS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\[\?25[lh]").unwrap());

    let s = RE_CR.replace_all(line, "");
    let s = RE_CURSOR_MOVE.replace_all(&s, "");
    let s = RE_CURSOR_POS.replace_all(&s, "");
    let s = RE_CURSOR_COL.replace_all(&s, "");
    let s = RE_CLEAR.replace_all(&s, "");
    let s = RE_CURSOR_VIS.replace_all(&s, "");
    s.into_owned()
}

/// Strip all ANSI escape codes (for test assertions)
pub fn strip_ansi(s: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());
    RE.replace_all(s, "").into_owned()
}
