//! What a launch leaves behind when it goes wrong: a log file, a panic hook, and an
//! alert for an error that stops baa before it has a window. Without them a missing
//! engine or a locked database is a Dock icon that bounces once and goes.

use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use core_foundation::{
    base::TCFType as _,
    string::{CFString, CFStringRef},
};

const TITLE: &str = "baa could not start";

/// Set once the first window is open: a panic after that is logged, not shown.
static WINDOW_UP: AtomicBool = AtomicBool::new(false);
static LOGGING: AtomicBool = AtomicBool::new(false);

pub fn window_up() {
    WINDOW_UP.store(true, Ordering::SeqCst);
}

/// The GUI launch's way out: stderr, the log, and an alert, then status 1.
pub fn fail(what: &str, err: impl std::fmt::Display) -> ! {
    let message = format!("{what}: {err}");
    // The logger tees to stderr; before it starts, stderr is all there is.
    if LOGGING.load(Ordering::SeqCst) {
        log::error!("{message}");
    } else {
        eprintln!("baa: {message}");
    }
    alert(&message, &crate::data_dir());
    std::process::exit(1);
}

/// For `baa mcp`: a stdio bridge another program runs, so stderr only.
pub fn fail_quietly(what: &str, err: impl std::fmt::Display) -> ! {
    eprintln!("baa: {what}: {err}");
    std::process::exit(1);
}

fn alert_body(message: &str, data_dir: &Path) -> String {
    format!("{message}\n\nData folder: {}", data_dir.display())
}

/// Blocks until the user dismisses it. CFUserNotification needs no NSApplication and
/// works from any thread, so it serves a failure before gpui starts and a panic hook.
fn alert(message: &str, data_dir: &Path) {
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFUserNotificationDisplayAlert(
            timeout: f64,
            flags: usize,
            icon_url: *const std::ffi::c_void,
            sound_url: *const std::ffi::c_void,
            localization_url: *const std::ffi::c_void,
            header: CFStringRef,
            message: CFStringRef,
            default_button: CFStringRef,
            alternate_button: CFStringRef,
            other_button: CFStringRef,
            response: *mut usize,
        ) -> i32;
    }
    let (title, body) = (CFString::new(TITLE), CFString::new(&alert_body(message, data_dir)));
    let null = std::ptr::null();
    let mut response = 0;
    // kCFUserNotificationStopAlertLevel is 0; a null default button is "OK".
    // SAFETY: the strings outlive the call, which does not keep them; null is allowed
    // for every URL and button; `response` is a valid out-param.
    unsafe {
        CFUserNotificationDisplayAlert(
            0.,
            0,
            null,
            null,
            null,
            title.as_concrete_TypeRef(),
            body.as_concrete_TypeRef(),
            null.cast(),
            null.cast(),
            null.cast(),
            &mut response,
        );
    }
}

/// `~/Library/Logs/baa/baa.log`; a dev build or a `BAA_DATA_DIR` run keeps it in its
/// data directory, so tests and dev runs stay out of the user's Logs.
fn log_path(data_dir: &Path, home: Option<&Path>, dev: bool) -> PathBuf {
    match home {
        Some(home) if !dev => home.join("Library/Logs/baa/baa.log"),
        _ => data_dir.join("baa.log"),
    }
}

/// The previous run's log becomes `baa.log.1`, replacing the one before it.
fn open_log(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut old = path.as_os_str().to_owned();
    old.push(".1");
    match std::fs::rename(path, &old) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    File::create(path)
}

struct Tee(File);

impl io::Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Logging to stderr and the log file, `info` unless `RUST_LOG` says otherwise. A log
/// file that cannot be opened leaves stderr only.
pub fn init_log(data_dir: &Path) {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dev = cfg!(debug_assertions) || std::env::var_os("BAA_DATA_DIR").is_some();
    let path = log_path(data_dir, home.as_deref(), dev);
    let opened = open_log(&path).map(|file| {
        builder.target(env_logger::Target::Pipe(Box::new(Tee(file))));
        builder.write_style(env_logger::WriteStyle::Never);
    });
    builder.init();
    LOGGING.store(true, Ordering::SeqCst);
    match opened {
        Ok(_) => log::info!("baa {} logging to {}", env!("CARGO_PKG_VERSION"), path.display()),
        Err(e) => log::warn!("log file {}: {e}", path.display()),
    }
}

/// Puts a panic and its backtrace in the log. One before the window is up is a launch
/// that failed (no or the wrong BaaCore.framework), so it is also shown.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("{info}\n{}", std::backtrace::Backtrace::force_capture());
        previous(info);
        if !WINDOW_UP.load(Ordering::SeqCst) {
            let message = info.payload_as_str().unwrap_or("baa stopped unexpectedly");
            alert(message, &crate::data_dir());
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::{alert_body, log_path, open_log};
    use std::path::Path;

    #[test]
    fn the_log_goes_to_the_users_logs_only_for_an_installed_baa() {
        let (data, home) = (Path::new("/d/baa"), Path::new("/Users/u"));
        assert_eq!(log_path(data, Some(home), false), Path::new("/Users/u/Library/Logs/baa/baa.log"));
        // A debug build or BAA_DATA_DIR.
        assert_eq!(log_path(data, Some(home), true), Path::new("/d/baa/baa.log"));
        assert_eq!(log_path(data, None, false), Path::new("/d/baa/baa.log"));
    }

    #[test]
    fn a_new_run_keeps_the_previous_log_as_dot_1() {
        let dir = std::env::temp_dir().join(format!("baa-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("logs/baa.log");
        open_log(&path).unwrap();
        std::fs::write(&path, "run 1").unwrap();
        open_log(&path).unwrap();
        std::fs::write(&path, "run 2").unwrap();
        open_log(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert_eq!(std::fs::read_to_string(dir.join("logs/baa.log.1")).unwrap(), "run 2");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_alert_names_the_error_and_the_data_folder() {
        let body = alert_body("opening baa.db: database is locked", Path::new("/d/baa"));
        assert_eq!(body, "opening baa.db: database is locked\n\nData folder: /d/baa");
    }
}
