use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn init_color(no_color_flag: bool) {
    let no_color_env = std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty());
    let is_term = std::io::stdout().is_terminal();

    let enabled = !no_color_flag && !no_color_env && is_term;
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

pub fn bold(s: impl AsRef<str>) -> String {
    if is_color_enabled() {
        format!("\x1b[1m{}\x1b[0m", s.as_ref())
    } else {
        s.as_ref().to_string()
    }
}

pub fn dim(s: impl AsRef<str>) -> String {
    if is_color_enabled() {
        format!("\x1b[90m{}\x1b[0m", s.as_ref())
    } else {
        s.as_ref().to_string()
    }
}

pub fn accent(s: impl AsRef<str>) -> String {
    if is_color_enabled() {
        format!("\x1b[38;5;75m{}\x1b[0m", s.as_ref())
    } else {
        s.as_ref().to_string()
    }
}

pub fn green(s: impl AsRef<str>) -> String {
    if is_color_enabled() {
        format!("\x1b[32m{}\x1b[0m", s.as_ref())
    } else {
        s.as_ref().to_string()
    }
}

pub fn yellow(s: impl AsRef<str>) -> String {
    if is_color_enabled() {
        format!("\x1b[33m{}\x1b[0m", s.as_ref())
    } else {
        s.as_ref().to_string()
    }
}

pub fn red(s: impl AsRef<str>) -> String {
    if is_color_enabled() {
        format!("\x1b[31m{}\x1b[0m", s.as_ref())
    } else {
        s.as_ref().to_string()
    }
}
