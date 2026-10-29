//! Autosave of unsaved work and its recovery after a crash: the open case is written to a
//! recovery file of its own while it has unsaved changes, removed on a clean exit, and offered
//! back at the next start when it is still there.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use dentcraft_case::Case;

/// How often unsaved changes are written.
pub const AUTOSAVE_EVERY: Duration = Duration::from_secs(30);

/// The recovery folder under the data folder.
pub fn recovery_dir(data: &Path) -> PathBuf {
    data.join("recovery")
}

/// This instance's recovery file under the data folder: one per running app, so a second open
/// window never overwrites the first one's unsaved work.
pub fn recovery_path(data: &Path) -> PathBuf {
    autosave_path(data, std::process::id())
}

/// The recovery file of the instance with this process id.
pub fn autosave_path(data: &Path, pid: u32) -> PathBuf {
    recovery_dir(data).join(format!("autosave-{pid}.dentcraft"))
}

/// The pre-instance recovery file: one shared file for every running app, still offered at
/// start-up.
const LEGACY_FILE: &str = "autosave.dentcraft";

/// A recovery file written this recently belongs to a live second window, not to a crash.
pub const LIVE_WITHIN: Duration = Duration::from_secs(60);

/// The recovery file to offer at start-up: the newest one, other than this process's own, that
/// has not been written for [`LIVE_WITHIN`] (a live second window's file is not offered). The
/// pre-instance `autosave.dentcraft` counts as a candidate too.
pub fn leftover(dir: &Path, own_pid: u32, now: SystemTime) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let pid = name
            .strip_prefix("autosave-")
            .and_then(|rest| rest.strip_suffix(".dentcraft"))
            .and_then(|pid| pid.parse::<u32>().ok());
        if pid.is_none() && name != LEGACY_FILE {
            continue; // Not a recovery file (another app's, or a half-written .tmp).
        }
        if pid == Some(own_pid) {
            continue; // Our own file.
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        // A write in the last minute means a live instance; a time ahead of ours (a clock set
        // differently) proves a fresh write too, so both are left alone.
        if !now
            .duration_since(modified)
            .is_ok_and(|age| age >= LIVE_WITHIN)
        {
            continue;
        }
        if best.as_ref().is_none_or(|(time, _)| modified > *time) {
            best = Some((modified, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// The unsaved case as of the last frame, for the panic hook (it cannot reach the app).
static REMEMBERED: Mutex<Option<Case>> = Mutex::new(None);

/// Keeps `case` for [`write_remembered`]; `None` when there is nothing unsaved.
pub fn remember(case: Option<&Case>) {
    let mut slot = REMEMBERED.lock().unwrap_or_else(|e| e.into_inner());
    match case {
        Some(case) if slot.as_ref() != Some(case) => *slot = Some(case.clone()),
        Some(_) => {}
        None => *slot = None,
    }
}

/// Writes the remembered case to `path`. Called from the panic hook, so it never blocks: a
/// lock held by the panicking thread itself means there is nothing consistent to write.
pub fn write_remembered(path: &Path) {
    let slot = match REMEMBERED.try_lock() {
        Ok(slot) => slot,
        Err(TryLockError::Poisoned(e)) => e.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if let Some(case) = slot.as_ref() {
        match write(path, case) {
            Ok(()) => eprintln!("dentcraft: unsaved case kept in {}", path.display()),
            Err(err) => eprintln!("dentcraft: {err}"),
        }
    }
}

/// Writes `case` to `path` with every scan path absolute (the recovery folder is not the case's
/// folder). Written to a temporary file first, so a crash mid-write keeps the previous copy.
pub fn write(path: &Path, case: &Case) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("cannot write {}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    }
    let mut out = case.clone();
    for scan in &mut out.scans {
        if let Ok(abs) = std::path::absolute(&scan.file) {
            scan.file = abs;
        }
    }
    let tmp = path.with_extension("tmp");
    out.save(&tmp).map_err(|e| fail(&e))?;
    std::fs::rename(&tmp, path).map_err(|e| fail(&e))
}

pub fn read(path: &Path) -> Result<Case, String> {
    crate::document::load(path).map_err(|e| e.to_string())
}

pub fn remove(path: &Path) {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("dentcraft: cannot remove {}: {e}", path.display());
        }
        _ => {}
    }
}

/// When the start-up prompt is due: a recovery file left by a crash or a kill, outside a
/// screenshot run. Returns when it was saved.
pub fn startup_prompt(path: &Path, screenshot: bool) -> Option<SystemTime> {
    if screenshot {
        return None;
    }
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    Some(meta.modified().unwrap_or_else(|_| SystemTime::now()))
}

/// The prompt's question, with the save time in local time.
pub fn prompt_text(saved: SystemTime) -> String {
    let at = DateTime::<Local>::from(saved).format("%H:%M");
    format!("Restore your last unsaved case (saved {at})?")
}

/// Writes the open case to the recovery file while it has unsaved changes.
pub struct Autosave {
    path: PathBuf,
    /// The case last written this session.
    written: Option<Case>,
    /// When the pending changes are written.
    due: Option<Instant>,
    /// Worker panics seen so far; a new one writes at once.
    panics: u64,
}

impl Autosave {
    pub fn new(path: PathBuf, panics: u64) -> Self {
        Self {
            path,
            written: None,
            due: None,
            panics,
        }
    }

    /// Runs once a frame. Writes a dirty case [`AUTOSAVE_EVERY`] after it changed, or at once
    /// after a worker panic; removes the file once the case is saved. Returns the time to the
    /// next write, so the caller can wake up for it.
    pub fn tick(
        &mut self,
        case: &Case,
        dirty: bool,
        now: Instant,
        panics: u64,
    ) -> Option<Duration> {
        let panicked = panics != self.panics;
        self.panics = panics;
        if !dirty {
            remember(None);
            self.due = None;
            if self.written.take().is_some() {
                remove(&self.path);
            }
            return None;
        }
        remember(Some(case));
        if self.written.as_ref() == Some(case) {
            self.due = None;
            return None;
        }
        let due = *self.due.get_or_insert(now + AUTOSAVE_EVERY);
        if panicked || now >= due {
            match write(&self.path, case) {
                Ok(()) => {
                    self.written = Some(case.clone());
                    self.due = None;
                    return None;
                }
                Err(err) => {
                    eprintln!("dentcraft: autosave: {err}");
                    self.due = Some(now + AUTOSAVE_EVERY);
                }
            }
        }
        self.due.map(|d| d.saturating_duration_since(now))
    }

    /// The app is closing normally: nothing needs recovering.
    pub fn clean_exit(&mut self) {
        remember(None);
        self.written = None;
        remove(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_case::{Scan, ScanRole};

    /// A fresh directory under the system temp folder, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("dentcraft-recovery-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn case_with_scan() -> Case {
        let mut case = Case::new("Mrs A");
        case.scans.push(Scan {
            file: PathBuf::from("scans/upper.stl"),
            role: ScanRole::Upper,
        });
        case
    }

    #[test]
    fn the_recovery_file_round_trips_with_absolute_scan_paths() {
        let dir = TempDir::new("round-trip");
        let path = recovery_path(&dir.0);
        let case = case_with_scan();
        write(&path, &case).unwrap();
        let back = read(&path).unwrap();

        assert_eq!(back.name, case.name);
        assert_eq!(back.design, case.design);
        assert!(back.scans[0].file.is_absolute());
        assert_eq!(
            back.scans[0].file,
            std::path::absolute("scans/upper.stl").unwrap()
        );
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn unsaved_changes_are_written_after_the_interval_and_removed_once_saved() {
        let dir = TempDir::new("tick");
        let path = recovery_path(&dir.0);
        let mut auto = Autosave::new(path.clone(), 0);
        let t0 = Instant::now();
        let case = case_with_scan();

        // A clean case is never written.
        assert_eq!(auto.tick(&case, false, t0, 0), None);
        assert!(!path.exists());
        // A dirty case waits for the interval...
        assert_eq!(auto.tick(&case, true, t0, 0), Some(AUTOSAVE_EVERY));
        assert!(!path.exists());
        // ...then is written.
        assert_eq!(auto.tick(&case, true, t0 + AUTOSAVE_EVERY, 0), None);
        assert_eq!(read(&path).unwrap().name, case.name);
        // Saving the case removes the file.
        auto.tick(&case, false, t0 + AUTOSAVE_EVERY * 2, 0);
        assert!(!path.exists());
    }

    #[test]
    fn a_worker_panic_writes_at_once() {
        let dir = TempDir::new("panic");
        let path = recovery_path(&dir.0);
        let mut auto = Autosave::new(path.clone(), 3);
        let t0 = Instant::now();
        let case = case_with_scan();
        auto.tick(&case, true, t0, 3);
        assert!(!path.exists());
        auto.tick(&case, true, t0, 4);
        assert!(path.exists());
    }

    #[test]
    fn a_clean_exit_removes_the_recovery_file() {
        let dir = TempDir::new("exit");
        let path = recovery_path(&dir.0);
        let mut auto = Autosave::new(path.clone(), 0);
        let t0 = Instant::now();
        auto.tick(&case_with_scan(), true, t0, 0);
        auto.tick(&case_with_scan(), true, t0 + AUTOSAVE_EVERY, 0);
        assert!(path.exists());
        auto.clean_exit();
        assert!(!path.exists());
    }

    #[test]
    fn a_left_over_recovery_file_prompts_except_in_screenshot_runs() {
        let dir = TempDir::new("prompt");
        let path = recovery_path(&dir.0);
        assert_eq!(startup_prompt(&path, false), None);
        write(&path, &case_with_scan()).unwrap();
        assert!(startup_prompt(&path, false).is_some());
        assert_eq!(startup_prompt(&path, true), None);
        let text = prompt_text(startup_prompt(&path, false).unwrap());
        assert!(
            text.starts_with("Restore your last unsaved case (saved "),
            "{text}"
        );
    }

    #[test]
    fn the_recovery_file_is_named_after_the_running_instance() {
        let dir = TempDir::new("naming");
        assert_eq!(
            recovery_path(&dir.0),
            recovery_dir(&dir.0).join(format!("autosave-{}.dentcraft", std::process::id()))
        );
        assert_eq!(
            autosave_path(&dir.0, 7),
            recovery_dir(&dir.0).join("autosave-7.dentcraft")
        );
        assert_eq!(LEGACY_FILE, "autosave.dentcraft");
    }

    /// Writes a recovery file of another instance, `age` old, into `dir`.
    fn leftover_file(dir: &Path, name: &str, age: Duration) -> PathBuf {
        let path = dir.join(name);
        write(&path, &case_with_scan()).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::now() - age))
            .unwrap();
        path
    }

    #[test]
    fn the_newest_stale_recovery_file_of_another_instance_is_chosen() {
        let dir = TempDir::new("leftover");
        let rec = recovery_dir(&dir.0);
        std::fs::create_dir_all(&rec).unwrap();
        let old = Duration::from_secs(3600);
        let now = SystemTime::now();

        // Nothing there: nothing offered.
        assert_eq!(leftover(&rec, 1, now), None);
        // Our own file never is, and a fresh file of another instance is a live window's.
        leftover_file(&rec, "autosave-1.dentcraft", old);
        leftover_file(&rec, "autosave-2.dentcraft", Duration::ZERO);
        assert_eq!(leftover(&rec, 1, SystemTime::now()), None);
        // A stale file of another instance is...
        let stale = leftover_file(&rec, "autosave-3.dentcraft", old);
        assert_eq!(leftover(&rec, 1, SystemTime::now()), Some(stale.clone()));
        // ...and a newer one wins.
        let newest = leftover_file(&rec, "autosave-4.dentcraft", old - Duration::from_secs(60));
        assert_eq!(leftover(&rec, 1, SystemTime::now()), Some(newest.clone()));
        // Other files in the folder are none of our business.
        std::fs::write(rec.join("notes.txt"), "").unwrap();
        assert_eq!(leftover(&rec, 1, SystemTime::now()), Some(newest));
    }

    #[test]
    fn an_old_style_recovery_file_is_still_chosen() {
        let dir = TempDir::new("legacy");
        let rec = recovery_dir(&dir.0);
        std::fs::create_dir_all(&rec).unwrap();
        assert_eq!(leftover(&rec, 1, SystemTime::now()), None);
        let legacy = leftover_file(&rec, "autosave.dentcraft", Duration::from_secs(600));
        assert_eq!(leftover(&rec, 1, SystemTime::now()), Some(legacy));
    }
}
