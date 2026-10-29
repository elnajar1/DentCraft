//! The app's safety net: background jobs that survive their own panics, and a panic hook that
//! writes a crash log (and, for a panic on the UI thread, the recovery file) before the process
//! goes down.

use std::backtrace::Backtrace;
use std::fs::OpenOptions;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Local};

/// The app id: names the window class and the per-user data folder.
pub const APP_ID: &str = "dentcraft";

/// Stack of a background job. Deep work on big scans (recursive mesh walks) needs far more than
/// the 2 MiB default, and a stack overflow cannot be caught.
pub const WORKER_STACK: usize = 64 << 20;

/// What a job that panicked reports in its step.
pub const PANIC_MESSAGE: &str = "Something went wrong while building the crown \u{2014} the \
                                 design is unchanged. Details were saved to the crash log.";

/// Hidden switch for checking the safety net by hand: `DENTCRAFT_TEST_PANIC=worker` makes the
/// next design job (not a scan load) panic.
const TEST_PANIC_VAR: &str = "DENTCRAFT_TEST_PANIC";

/// Worker names starting with this load scans; the test switch leaves them alone.
pub const LOAD_PREFIX: &str = "load ";

static WORKER_PANICS: AtomicU64 = AtomicU64::new(0);
static TEST_PANIC_USED: AtomicBool = AtomicBool::new(false);

/// How many background jobs have panicked so far; the app saves the recovery file when it grows.
pub fn worker_panics() -> u64 {
    WORKER_PANICS.load(Ordering::SeqCst)
}

/// `true` once, for the first design job after start, when the test switch asks for a panic.
fn take_test_panic(name: &str) -> bool {
    !name.starts_with(LOAD_PREFIX)
        && std::env::var(TEST_PANIC_VAR).is_ok_and(|v| v == "worker")
        && !TEST_PANIC_USED.swap(true, Ordering::SeqCst)
}

/// Runs `work` on a named background thread with a [`WORKER_STACK`] stack and hands its outcome
/// to `deliver` (on that thread). A panic in `work` becomes `Err(PANIC_MESSAGE)` instead of
/// taking the thread's channel down with it; a thread that cannot start is reported the same way.
pub fn spawn_worker<T, W, D>(name: &str, work: W, deliver: D)
where
    T: 'static,
    W: FnOnce() -> T + Send + 'static,
    D: FnOnce(Result<T, String>) + Send + 'static,
{
    // Shared so the caller can still deliver when the thread never starts.
    let slot = Arc::new(Mutex::new(Some(deliver)));
    let in_thread = Arc::clone(&slot);
    let inject = take_test_panic(name);
    let label = name.to_owned();
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                if inject {
                    panic!("{TEST_PANIC_VAR}: test panic in the {label} job");
                }
                work()
            }))
            .map_err(|_| {
                WORKER_PANICS.fetch_add(1, Ordering::SeqCst);
                PANIC_MESSAGE.to_owned()
            });
            if let Some(deliver) = take(&in_thread) {
                deliver(result);
            }
        });
    if let Err(err) = spawned {
        eprintln!("dentcraft: cannot start the {name} job: {err}");
        if let Some(deliver) = take(&slot) {
            deliver(Err(format!(
                "DentCraft could not start a background job ({err}). Close other programs and \
                 try again."
            )));
        }
    }
}

fn take<D>(slot: &Mutex<Option<D>>) -> Option<D> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// The per-user folder DentCraft keeps its files in: eframe's storage folder for the app id, one
/// level up on Windows where eframe adds `data` (so `%APPDATA%\dentcraft`).
pub fn data_dir() -> Option<PathBuf> {
    let dir = eframe::storage_dir(APP_ID)?;
    if cfg!(windows) && dir.file_name().is_some_and(|n| n == "data") {
        return dir.parent().map(Path::to_path_buf);
    }
    Some(dir)
}

/// Folder of the crash logs under the data folder.
pub fn crash_log_dir(data: &Path) -> PathBuf {
    data.join("crash-logs")
}

/// One panic, as written to the crash log.
pub struct CrashRecord {
    pub time: DateTime<Local>,
    pub thread: String,
    pub message: String,
    pub location: Option<String>,
    pub backtrace: String,
}

impl CrashRecord {
    pub fn text(&self) -> String {
        format!(
            "DentCraft {version} crash report\n\
             time: {time}\n\
             thread: {thread}\n\
             message: {message}\n\
             location: {location}\n\
             backtrace:\n{backtrace}\n\
             ----\n",
            version = env!("CARGO_PKG_VERSION"),
            time = self.time.format("%Y-%m-%d %H:%M:%S %:z"),
            thread = self.thread,
            message = self.message,
            location = self.location.as_deref().unwrap_or("unknown"),
            backtrace = self.backtrace.trim_end(),
        )
    }
}

/// Appends `record` to `dir/YYYYMMDD-HHMMSS.txt` (its local time) and returns the file.
pub fn write_crash_log(dir: &Path, record: &CrashRecord) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.txt", record.time.format("%Y%m%d-%H%M%S")));
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(record.text().as_bytes())?;
    Ok(path)
}

/// Installs the panic hook: every panic (caught in a worker or not) is logged to `logs`; a panic
/// on the UI thread also writes the case kept by [`crate::recovery::remember`] to `recovery`.
/// The previous hook still runs, so the message reaches the terminal too.
pub fn install_panic_hook(logs: Option<PathBuf>, recovery: Option<PathBuf>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed").to_owned();
        let on_ui_thread = thread == "main";
        let record = CrashRecord {
            time: Local::now(),
            thread,
            message: panic_message(info.payload()),
            location: info.location().map(ToString::to_string),
            backtrace: Backtrace::force_capture().to_string(),
        };
        let written = logs.as_deref().map(|dir| write_crash_log(dir, &record));
        if on_ui_thread && let Some(path) = &recovery {
            crate::recovery::write_remembered(path);
        }
        previous(info);
        match written {
            Some(Ok(path)) => eprintln!("dentcraft: crash details saved to {}", path.display()),
            Some(Err(err)) => eprintln!("dentcraft: cannot write the crash log: {err}"),
            None => {}
        }
    }));
}

/// The text a panic was raised with.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn run<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
        let (tx, rx) = channel();
        spawn_worker("test job", work, move |r| {
            let _ = tx.send(r);
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the worker always answers")
    }

    #[test]
    fn a_panicking_job_reports_an_error_instead_of_dropping_its_channel() {
        let before = worker_panics();
        let result = run(|| -> u32 { panic!("boom") });
        assert_eq!(result, Err(PANIC_MESSAGE.to_owned()));
        assert!(worker_panics() > before);
    }

    #[test]
    fn a_job_that_finishes_delivers_its_value() {
        assert_eq!(run(|| 6 * 7), Ok(42));
    }

    #[test]
    fn a_job_has_room_for_deep_recursion() {
        // About 24 MiB of stack: overflows a default 2 MiB thread, fits the worker's.
        fn depth(n: u32) -> u32 {
            let pad = [n as u8; 4096];
            std::hint::black_box(&pad);
            if n == 0 { 0 } else { depth(n - 1) + 1 }
        }
        assert_eq!(run(|| depth(6000)), Ok(6000));
    }

    #[test]
    fn the_crash_log_holds_the_message_and_where_it_happened() {
        let dir = std::env::temp_dir().join(format!("dentcraft-crash-{}", std::process::id()));
        let record = CrashRecord {
            time: Local::now(),
            thread: "margin".to_owned(),
            message: "index out of bounds".to_owned(),
            location: Some("src/margin.rs:12:5".to_owned()),
            backtrace: "0: somewhere".to_owned(),
        };
        let path = write_crash_log(&dir, &record).unwrap();
        // A second panic in the same second appends to the same file.
        let again = write_crash_log(&dir, &record).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(path, again);
        assert!(path.starts_with(&dir));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name.len(), "YYYYMMDD-HHMMSS.txt".len(), "{name}");
        assert!(text.contains("message: index out of bounds"), "{text}");
        assert!(text.contains("thread: margin"));
        assert!(text.contains("location: src/margin.rs:12:5"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert_eq!(text.matches("crash report").count(), 2);
    }
}
