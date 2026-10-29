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

/// The lock file of the instance with this process id, in the recovery folder `dir`: held
/// locked for as long as that instance runs.
fn lock_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("autosave-{pid}.lock"))
}

/// This instance's sign of life to the others: an exclusive lock on its lock file, held until
/// the process ends. The OS lets go of it when the process dies too (a crash or a kill), so a
/// recovery file whose owner's lock can be taken was left behind.
pub struct InstanceLock {
    file: std::fs::File,
    path: PathBuf,
}

impl InstanceLock {
    /// Locks this instance's lock file in the recovery folder `dir`.
    pub fn take(dir: &Path, pid: u32) -> Result<Self, String> {
        let path = lock_path(dir, pid);
        let fail = |e: &dyn std::fmt::Display| format!("cannot lock {}: {e}", path.display());
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(false).write(true);
        // Others may open it to probe the lock, but nobody can delete it while it is held.
        #[cfg(windows)]
        {
            const FILE_SHARE_READ: u32 = 0x1;
            const FILE_SHARE_WRITE: u32 = 0x2;
            std::os::windows::fs::OpenOptionsExt::share_mode(
                &mut options,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
            );
        }
        let file = options.open(&path).map_err(|e| fail(&e))?;
        file.try_lock().map_err(|e| fail(&e))?;
        Ok(Self { file, path })
    }

    /// Lets go of the lock and removes its file: the instance is closing normally.
    pub fn release(self) {
        drop(self.file);
        remove(&self.path);
    }
}

/// Starts this instance's recovery in the folder `dir`: takes its lock, then sets aside a
/// recovery file already at its own path. That one was left by a dead instance with the same
/// process id (Windows reuses them); renamed to `autosave-<pid>-<unix secs>.dentcraft` it has no
/// owner, so it is offered as a leftover instead of being overwritten.
pub fn start(dir: &Path, pid: u32) -> Result<InstanceLock, String> {
    // Another instance probing a dead owner's lock file holds its lock for an instant.
    let lock = InstanceLock::take(dir, pid).or_else(|_| {
        std::thread::sleep(Duration::from_millis(100));
        InstanceLock::take(dir, pid)
    })?;
    let own = dir.join(format!("autosave-{pid}.dentcraft"));
    if let Ok(meta) = std::fs::metadata(&own) {
        let secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let aside = dir.join(format!("autosave-{pid}-{secs}.dentcraft"));
        std::fs::rename(&own, &aside)
            .map_err(|e| format!("cannot move {} aside: {e}", own.display()))?;
    }
    Ok(lock)
}

/// Who wrote the recovery file called `name`: `Some(Some(pid))` for a running instance's own
/// file, `Some(None)` for one without a live owner (the pre-instance file, or one set aside by
/// [`start`]), `None` when it is not a recovery file (another app's, or a half-written .tmp).
fn owner(name: &str) -> Option<Option<u32>> {
    if name == LEGACY_FILE {
        return Some(None);
    }
    let rest = name
        .strip_prefix("autosave-")
        .and_then(|rest| rest.strip_suffix(".dentcraft"))?;
    if let Ok(pid) = rest.parse::<u32>() {
        return Some(Some(pid));
    }
    let (pid, secs) = rest.split_once('-')?;
    (pid.parse::<u32>().is_ok() && secs.parse::<u64>().is_ok()).then_some(None)
}

/// Whether the instance with this process id may still be running. It is gone only when its
/// lock file is not there or its lock can be taken; a lock file that cannot be opened or probed
/// (another program holding it, a refused access) counts as live, so its files are left alone.
fn is_live(dir: &Path, pid: u32) -> bool {
    match std::fs::File::open(lock_path(dir, pid)) {
        Ok(file) => file.try_lock().is_err(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// The process id of the lock file called `name`, `autosave-<pid>.lock`.
fn lock_owner(name: &str) -> Option<u32> {
    name.strip_prefix("autosave-")?
        .strip_suffix(".lock")?
        .parse()
        .ok()
}

/// What [`Autosave::restore`] and [`discard`] say when the leftover's owner runs again.
const IN_USE: &str =
    "Another DentCraft window is using this unsaved case now, so it was left as it is.";

/// Refuses the leftover at `path` once an instance with its owner's process id runs: that one
/// writes to the file now (Windows reuses process ids while a prompt can stay open).
fn unowned(path: &Path) -> Result<(), String> {
    let owner = path.file_name().and_then(|n| n.to_str()).and_then(owner);
    match (owner, path.parent()) {
        (Some(Some(pid)), Some(dir)) if is_live(dir, pid) => Err(IN_USE.to_owned()),
        _ => Ok(()),
    }
}

/// The recovery file to offer at start-up: the newest one in the recovery folder `dir`, other
/// than this process's own, whose owner is not running (a live second window's file is never
/// offered). Set-aside files and the pre-instance `autosave.dentcraft` have no owner, so they
/// are offered whatever their age. Lock files of dead instances that left no recovery file are
/// swept on the way.
pub fn leftover(dir: &Path, own_pid: u32) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if let Some(pid) = entry.file_name().to_str().and_then(lock_owner) {
            if pid != own_pid
                && !dir.join(format!("autosave-{pid}.dentcraft")).exists()
                && !is_live(dir, pid)
            {
                remove(&entry.path());
            }
            continue;
        }
        let Some(owner) = entry.file_name().to_str().and_then(owner) else {
            continue;
        };
        if let Some(pid) = owner
            && (pid == own_pid || is_live(dir, pid))
        {
            continue; // Our own file, or a live second window's.
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(time, _)| modified > *time) {
            best = Some((modified, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// Removes a leftover recovery file the user restored or discarded, with its dead owner's lock
/// file. Leaves both alone when the owner runs again.
pub fn discard(path: &Path) -> Result<(), String> {
    unowned(path)?;
    remove(path);
    let owner = path.file_name().and_then(|n| n.to_str()).and_then(owner);
    if let (Some(Some(pid)), Some(dir)) = (owner, path.parent())
        && !is_live(dir, pid)
    {
        remove(&lock_path(dir, pid));
    }
    Ok(())
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
    /// This instance's lock, held until the clean exit.
    lock: Option<InstanceLock>,
    /// The case last written this session.
    written: Option<Case>,
    /// When the pending changes are written.
    due: Option<Instant>,
    /// Worker panics seen so far; a new one writes at once.
    panics: u64,
}

impl Autosave {
    pub fn new(path: PathBuf, lock: Option<InstanceLock>, panics: u64) -> Self {
        Self {
            path,
            lock,
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

    /// Takes over the leftover recovery file at `leftover`: reads its case and keeps it in this
    /// instance's own file before the leftover goes, so a crash right after restoring loses
    /// nothing. When the own file cannot be written the leftover stays for the next start. Refused
    /// when the leftover's owner runs again.
    pub fn restore(&mut self, leftover: &Path) -> Result<Case, String> {
        unowned(leftover)?;
        let case = read(leftover)?;
        match write(&self.path, &case) {
            Ok(()) => {
                self.written = Some(case.clone());
                self.due = None;
                if let Err(err) = discard(leftover) {
                    eprintln!("dentcraft: autosave: {err}");
                }
            }
            Err(err) => eprintln!("dentcraft: autosave: {err}"),
        }
        Ok(case)
    }

    /// The app is closing normally: nothing needs recovering, and the instance is gone.
    pub fn clean_exit(&mut self) {
        remember(None);
        self.written = None;
        remove(&self.path);
        if let Some(lock) = self.lock.take() {
            lock.release();
        }
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
        let mut auto = Autosave::new(path.clone(), None, 0);
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
        let mut auto = Autosave::new(path.clone(), None, 3);
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
        let mut auto = Autosave::new(path.clone(), None, 0);
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

    /// The recovery folder of a fresh temp data folder.
    fn rec_dir(dir: &TempDir) -> PathBuf {
        let rec = recovery_dir(&dir.0);
        std::fs::create_dir_all(&rec).unwrap();
        rec
    }

    #[test]
    fn a_live_instances_recovery_file_is_offered_only_once_its_lock_is_free() {
        let dir = TempDir::new("live");
        let rec = rec_dir(&dir);
        // Written long ago: age says nothing about whether its window is still open.
        let file = leftover_file(&rec, "autosave-2.dentcraft", Duration::from_secs(3600));
        let lock = InstanceLock::take(&rec, 2).unwrap();
        assert_eq!(leftover(&rec, 1), None);
        // The instance dies: the OS lets go of its lock, the lock file stays.
        drop(lock);
        assert!(lock_path(&rec, 2).exists());
        assert_eq!(leftover(&rec, 1), Some(file));
    }

    #[test]
    fn a_recovery_file_without_a_live_owner_is_offered_at_once() {
        let dir = TempDir::new("leftover");
        let rec = rec_dir(&dir);

        // Nothing there: nothing offered.
        assert_eq!(leftover(&rec, 1), None);
        // Our own file never is; a set-aside or old-style one is, and the newer one wins.
        leftover_file(&rec, "autosave-1.dentcraft", Duration::from_secs(3600));
        assert_eq!(leftover(&rec, 1), None);
        let aside = leftover_file(
            &rec,
            "autosave-1-1700000000.dentcraft",
            Duration::from_secs(120),
        );
        assert_eq!(leftover(&rec, 1), Some(aside));
        let legacy = leftover_file(&rec, "autosave.dentcraft", Duration::from_secs(60));
        assert_eq!(leftover(&rec, 1), Some(legacy));
        // A crash's (no lock file at all) is offered however fresh: written this very moment.
        let crashed = leftover_file(&rec, "autosave-3.dentcraft", Duration::ZERO);
        assert_eq!(leftover(&rec, 1), Some(crashed.clone()));
        // Other files in the folder are none of our business.
        std::fs::write(rec.join("notes.txt"), "").unwrap();
        std::fs::write(rec.join("autosave-x.dentcraft"), "").unwrap();
        assert_eq!(leftover(&rec, 1), Some(crashed));
    }

    #[test]
    fn a_dead_instances_file_at_our_own_path_is_set_aside_and_offered() {
        let dir = TempDir::new("pid-reuse");
        let rec = rec_dir(&dir);
        let own = rec.join("autosave-5.dentcraft");
        let mut case = case_with_scan();
        case.name = "Left by pid 5's crash".into();
        write(&own, &case).unwrap();

        let lock = start(&rec, 5).unwrap();
        assert!(!own.exists());
        let aside = leftover(&rec, 5).expect("the set-aside file is offered");
        assert_ne!(aside, own);
        assert_eq!(read(&aside).unwrap().name, case.name);
        lock.release();
        assert!(!lock_path(&rec, 5).exists());
    }

    #[test]
    fn restoring_moves_the_case_into_our_own_file_and_drops_the_leftover() {
        let dir = TempDir::new("restore");
        let rec = rec_dir(&dir);
        let file = leftover_file(&rec, "autosave-2.dentcraft", Duration::ZERO);
        drop(InstanceLock::take(&rec, 2).unwrap()); // The crashed owner's lock file.
        let own = rec.join("autosave-1.dentcraft");
        let mut auto = Autosave::new(own.clone(), None, 0);

        let case = auto.restore(&file).unwrap();
        assert_eq!(case.name, case_with_scan().name);
        assert!(!file.exists());
        assert!(!lock_path(&rec, 2).exists());
        assert_eq!(read(&own).unwrap().name, case.name);
        assert_eq!(leftover(&rec, 1), None);
        // Already kept: the tick has nothing to write.
        assert_eq!(auto.tick(&case, true, Instant::now(), 0), None);
    }

    #[test]
    fn a_leftover_stays_when_our_own_file_cannot_be_written() {
        let dir = TempDir::new("restore-fail");
        let rec = rec_dir(&dir);
        let file = leftover_file(&rec, "autosave-2.dentcraft", Duration::ZERO);
        // A file where the own file's folder should be: the write fails.
        std::fs::write(dir.0.join("blocker"), "").unwrap();
        let mut auto = Autosave::new(dir.0.join("blocker").join("autosave-1.dentcraft"), None, 0);

        assert!(auto.restore(&file).is_ok());
        assert!(file.exists());
        assert_eq!(leftover(&rec, 1), Some(file));
    }

    #[test]
    fn discarding_removes_only_the_chosen_leftover() {
        let dir = TempDir::new("discard");
        let rec = rec_dir(&dir);
        let chosen = leftover_file(&rec, "autosave-2.dentcraft", Duration::ZERO);
        drop(InstanceLock::take(&rec, 2).unwrap());
        let other = leftover_file(&rec, "autosave-3.dentcraft", Duration::from_secs(60));
        let live = leftover_file(&rec, "autosave-4.dentcraft", Duration::from_secs(60));
        let _lock = InstanceLock::take(&rec, 4).unwrap();

        discard(&chosen).unwrap();
        assert!(!chosen.exists());
        assert!(!lock_path(&rec, 2).exists());
        assert!(other.exists());
        assert!(live.exists());
        assert!(lock_path(&rec, 4).exists());
        assert_eq!(leftover(&rec, 1), Some(other));
    }

    /// Holds the lock file of `pid` open the way another program might: nobody else can open
    /// it, so its lock cannot be probed.
    #[cfg(windows)]
    fn unprobeable_lock(dir: &Path, pid: u32) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .share_mode(0)
            .open(lock_path(dir, pid))
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn a_lock_file_that_cannot_be_probed_counts_as_live() {
        let dir = TempDir::new("unprobeable");
        let rec = rec_dir(&dir);
        let file = leftover_file(&rec, "autosave-2.dentcraft", Duration::from_secs(3600));
        let held = unprobeable_lock(&rec, 2);
        assert!(std::fs::File::open(lock_path(&rec, 2)).is_err());
        assert!(is_live(&rec, 2));
        assert_eq!(leftover(&rec, 1), None);
        assert!(file.exists());
        drop(held);
        assert!(!is_live(&rec, 2));
        assert_eq!(leftover(&rec, 1), Some(file));
    }

    #[test]
    fn restore_and_discard_leave_a_live_owners_file_alone() {
        let dir = TempDir::new("owner-back");
        let rec = rec_dir(&dir);
        let file = leftover_file(&rec, "autosave-2.dentcraft", Duration::ZERO);
        // Offered while pid 2 was dead; a new instance with that pid starts before the choice.
        assert_eq!(leftover(&rec, 1), Some(file.clone()));
        let lock = InstanceLock::take(&rec, 2).unwrap();
        let own = rec.join("autosave-1.dentcraft");
        let mut auto = Autosave::new(own.clone(), None, 0);

        assert_eq!(auto.restore(&file), Err(IN_USE.to_owned()));
        assert!(!own.exists());
        assert_eq!(discard(&file), Err(IN_USE.to_owned()));
        assert!(file.exists());
        assert!(lock_path(&rec, 2).exists());
        drop(lock);
    }

    #[test]
    fn dead_lock_files_without_a_recovery_file_are_swept() {
        let dir = TempDir::new("sweep");
        let rec = rec_dir(&dir);
        drop(InstanceLock::take(&rec, 2).unwrap()); // Dead, nothing left: swept.
        drop(InstanceLock::take(&rec, 3).unwrap()); // Dead, its file left: kept.
        let file = leftover_file(&rec, "autosave-3.dentcraft", Duration::ZERO);
        let _live = InstanceLock::take(&rec, 4).unwrap(); // Live, no file yet: kept.
        let _own = InstanceLock::take(&rec, 1).unwrap();
        std::fs::write(rec.join("autosave-x.lock"), "").unwrap(); // Not ours: kept.

        assert_eq!(leftover(&rec, 1), Some(file));
        assert!(!lock_path(&rec, 2).exists());
        assert!(lock_path(&rec, 3).exists());
        assert!(lock_path(&rec, 4).exists());
        assert!(lock_path(&rec, 1).exists());
        assert!(rec.join("autosave-x.lock").exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_held_lock_file_cannot_be_deleted() {
        let dir = TempDir::new("held");
        let rec = rec_dir(&dir);
        let lock = InstanceLock::take(&rec, 2).unwrap();
        assert!(std::fs::remove_file(lock_path(&rec, 2)).is_err());
        // Other instances can still probe it, and the clean exit removes it.
        assert!(is_live(&rec, 2));
        lock.release();
        assert!(!lock_path(&rec, 2).exists());
    }
}
