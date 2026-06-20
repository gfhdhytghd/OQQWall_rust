use std::fs::{OpenOptions, create_dir_all};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_ENV_VAR: &str = "OQQWALL_DEBUG_LOG";
const DATA_DIR_ENV_VAR: &str = "OQQWALL_DATA_DIR";
const DEFAULT_LOG_REL_PATH: &str = "logs/debug.log";

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const MAGENTA: &str = "\x1b[35m";

static LOG_FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

pub fn log(args: std::fmt::Arguments) {
    let ts = timestamp();
    let body = format!("{}", args);
    let message = if is_color_term() {
        format!("{DIM}{ts}{RESET} {body}")
    } else {
        format!("{ts} {body}")
    };
    write_output(&message);
}

pub fn info(args: std::fmt::Arguments) {
    write_output(&format_log(CYAN, "INFO", args));
}

pub fn success(args: std::fmt::Arguments) {
    write_output(&format_log(GREEN, " OK ", args));
}

pub fn warn(args: std::fmt::Arguments) {
    write_output(&format_log(YELLOW, "WARN", args));
}

pub fn error(args: std::fmt::Arguments) {
    write_output(&format_log(RED, "ERR ", args));
}

pub fn debug(args: std::fmt::Arguments) {
    write_output(&format_log(MAGENTA, "DBG ", args));
}

pub fn trace(args: std::fmt::Arguments) {
    let ts = timestamp();
    let body = format!("{}", args);
    let message = if is_color_term() {
        format!("{DIM}{ts} TRC {body}{RESET}")
    } else {
        format!("{ts} TRC {body}")
    };
    write_output(&message);
}

fn format_log(color: &str, label: &str, args: std::fmt::Arguments) -> String {
    let ts = timestamp();
    let body = format!("{}", args);
    if is_color_term() {
        format!("{DIM}{ts}{RESET} {color}{label}{RESET} {body}")
    } else {
        format!("{ts} {label} {body}")
    }
}

fn write_output(message: &str) {
    let mut stderr = std::io::stderr();
    let _ = stderr.write_all(message.as_bytes());
    let _ = stderr.write_all(b"\n");
    let _ = stderr.flush();

    let Some(lock) = LOG_FILE.get_or_init(init_log_file).as_ref() else {
        return;
    };
    if let Ok(mut file) = lock.lock() {
        let plain = strip_ansi(message);
        let _ = file.write_all(plain.as_bytes());
        let _ = file.write_all(b"\n");
        let _ = file.flush();
    }
}

fn is_color_term() -> bool {
    std::env::var("NO_COLOR").is_err() && std::io::stderr().is_terminal()
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs() + 8 * 3600;
    let h = (total_secs / 3600) % 24;
    let m = (total_secs / 60) % 60;
    let s = total_secs % 60;
    let ms = now.subsec_millis();
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            while let Some(&c) = chars.peek() {
                chars.next();
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn init_log_file() -> Option<Mutex<std::fs::File>> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        if let Err(err) = create_dir_all(parent) {
            eprintln!("debug log: create dir failed {}: {}", parent.display(), err);
            return None;
        }
    }
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some(Mutex::new(file)),
        Err(err) => {
            eprintln!("debug log: open failed {}: {}", path.display(), err);
            None
        }
    }
}

fn log_path() -> PathBuf {
    if let Ok(path) = std::env::var(LOG_ENV_VAR) {
        return PathBuf::from(path);
    }
    let data_dir = std::env::var(DATA_DIR_ENV_VAR).unwrap_or_else(|_| "data".to_string());
    PathBuf::from(data_dir).join(DEFAULT_LOG_REL_PATH)
}
