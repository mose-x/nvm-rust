use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::i18n::T;

// `colored::Colorize` is needed for the cyan() call in `acquire_nvm_lock`'s
// blocking path. It's already a dependency; bring it into scope here so the
// macro resolves without forcing every caller of utils to import it.
use colored::Colorize as _;

// ---------------------------------------------------------------------------
// Concurrency: nvm-wide advisory lock
// ---------------------------------------------------------------------------

/// Process-local flag for re-entrancy. `nvm use --install` calls `install`
/// internally, and both want to hold the nvm lock; a second `flock(LOCK_EX)`
/// on the same file from the *same* process can self-deadlock on some
/// platforms, so we track ownership per-process and hand out a no-op guard
/// when the lock is already held. Cross-process contention is still
/// serialised by the OS lock itself.
pub(crate) static NVM_LOCK_HELD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// RAII guard holding an exclusive OS advisory lock on the nvm directory.
///
/// Prevents two `nvm` processes from racing on mutating operations
/// (install / uninstall / use) against the same `NVM_DIR`. The lock is an
/// OS-level advisory lock (`flock` on Unix, `LockFileEx` on Windows) on the
/// `.nvm.lock` file, which the kernel releases automatically when the
/// holding process exits — so a crashed/killed `nvm` never leaves a stale
/// lock behind (the previous PID-based lock-file approach had both a
/// reclaim race and stale-lock leaks on crash).
///
/// The `Option<File>` is `None` for a re-entrant acquire (the current
/// process already holds the lock via an outer call); dropping a re-entrant
/// guard does not release the lock.
///
/// Acquired via [`acquire_nvm_lock`]; released on `Drop`.
pub struct NvmLock(Option<std::fs::File>);

impl Drop for NvmLock {
    fn drop(&mut self) {
        if let Some(file) = self.0.take() {
            // `fs4::fs_std::FileExt::unlock` is brought into scope by the
            // `use` inside `acquire_nvm_lock` for the acquire path; for the
            // drop path we reference it fully-qualified to avoid a stale
            // module-level import.
            //
            // ORDER MATTERS: clear the process-local flag BEFORE releasing the
            // OS lock. If we unlock first, there is a window where the OS lock
            // is free but `NVM_LOCK_HELD` is still `true` — another thread in
            // THIS process calling `acquire_nvm_lock` would then see `swap`
            // return `true`, take the re-entrant no-op branch, and execute its
            // critical section with NO OS lock while a different process has
            // already grabbed the now-free OS lock. That breaks mutual
            // exclusion silently.
            //
            // Clearing the flag first means a same-process contender sees
            // `false`, tries `swap(true)` → `false`, and goes for the OS lock
            // (still held by us) → blocks until we unlock. Correct.
            //
            // If `unlock` itself fails (extremely rare: invalid fd, kernel
            // error), the OS lock stays held with `NVM_LOCK_HELD=false`. The
            // next same-process acquire will then block on `lock_exclusive`
            // rather than silently bypass — a safer failure mode than the
            // silent-bypass window above. We still surface the failure as a
            // warning so it is diagnosable.
            NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
            if let Err(e) = fs4::fs_std::FileExt::unlock(&file) {
                eprintln!(
                    "{} {}: {}",
                    "⚠".yellow().bold(),
                    T("lock_release_failed"),
                    e
                );
            }
        }
        // Re-entrant guard (None): nothing to release; the outer guard still
        // owns the OS lock and the `NVM_LOCK_HELD` flag.
    }
}

/// RAII guard that removes a file when dropped, unless disarmed.
///
/// Used by `download_prebuilt_npm` to ensure the npm tarball is cleaned up
/// on EVERY exit path (download `io::copy` failure, truncation, integrity
/// mismatch, tar extraction failure, symlink failure), not just the success
/// path. Previously only the truncation/integrity branches and the final
/// success line cleaned up; an `io::copy` `?` left a half-written
/// `npm-v*.tgz` that the next run's `exists()` cache-hit check treated as
/// complete, silently skipping re-download and then failing at extraction
/// with a confusing "unexpected EOF".
///
/// On the success path the caller removes the file explicitly (so a failure
/// between staging and disarm still triggers cleanup via `Drop`) and then
/// calls `disarm()` so `Drop` does not issue a redundant `remove_file`.
pub struct FileGuard {
    path: std::path::PathBuf,
    armed: bool,
}

impl FileGuard {
    /// Create an armed guard for `path`. The file at `path` need not exist
    /// yet (e.g. it is about to be created by a download); `Drop` will
    /// silently tolerate a missing file via `remove_file`'s `Err` being
    /// ignored.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            armed: true,
        }
    }

    /// Disarm the guard so `Drop` does not remove the file. Call this only
    /// after the file has been successfully consumed (extracted + wired up)
    /// AND explicitly removed by the caller, so a failure between staging
    /// and disarm still triggers cleanup.
    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        if self.armed {
            // Best-effort: a missing file (e.g. download never started, or
            // caller already removed it) is not an error. Any other I/O
            // error (permission denied, etc.) is swallowed because we are
            // on an unwind/early-return path where surfacing it would mask
            // the real error in flight.
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Acquire an exclusive lock on the nvm directory, blocking until any other
/// `nvm` process releases it.
///
/// The lock file lives at `<nvm_dir>/.nvm.lock`. We open it `create(true)`
/// so the first-ever invocation creates it; subsequent invocations reuse the
/// same file and contend on the OS lock, not on file creation (which would
/// be racy).
///
/// **Re-entrant within a process**: if the current process already holds the
/// lock (e.g. `nvm use --install` → `install`), this returns a no-op guard
/// instead of deadlocking on a second `flock(LOCK_EX)` on the same file.
///
/// Returns an [`NvmLock`] whose `Drop` releases the lock. Hold it for the
/// duration of the mutating operation (install / uninstall / use).
pub fn acquire_nvm_lock(nvm_dir: &Path) -> Result<NvmLock> {
    use fs4::fs_std::FileExt;

    // Re-entrancy: already held in this process → hand out a no-op guard.
    // `swap` returns the previous value; if it was already `true`, another
    // frame in this process owns the real lock, so we don't touch the OS
    // lock and we must NOT flip the flag back (the outer owner will).
    if NVM_LOCK_HELD.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Ok(NvmLock(None));
    }

    // Ensure the EXACT dir we were passed exists -- not a re-derived
    // `get_nvm_dir()` path. If `NVM_DIR` changed between the caller's
    // capture and here (e.g. a parallel test doing `set_var("NVM_DIR", ..)`),
    // `ensure_nvm_dir()` would create a *different* dir and the
    // `open(nvm_dir.join(".nvm.lock"))` below would hit ENOENT. Using the
    // parameter closes that race and is more correct: we lock the dir the
    // caller asked us to lock.
    //
    // The flag was already flipped to `true` above, so on failure we MUST
    // roll it back — otherwise every subsequent `acquire_nvm_lock` in this
    // process takes the re-entrant branch and returns a no-op guard,
    // silently bypassing the OS lock and breaking mutual exclusion.
    fs::create_dir_all(nvm_dir).inspect_err(|_| {
        NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
    })?;
    let lock_path = nvm_dir.join(".nvm.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| {
            // Roll back the flag so a later retry can actually acquire.
            NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
            anyhow::anyhow!("{}: {e}", T("lock_open_failed"))
        })?;

    // Fast path: try a non-blocking acquire so the common single-invocation
    // case returns instantly. If another nvm holds the lock, fall back to a
    // blocking acquire (with a notice) so we wait for it instead of erroring
    // out — concurrent `nvm install` should serialize, not fail.
    match file.try_lock_exclusive() {
        Ok(()) => Ok(NvmLock(Some(file))),
        Err(_) => {
            eprintln!("  {} {}", "⏳".cyan(), T("lock_wait_another"));
            match file.lock_exclusive() {
                Ok(()) => Ok(NvmLock(Some(file))),
                Err(e) => {
                    NVM_LOCK_HELD.store(false, std::sync::atomic::Ordering::Release);
                    Err(anyhow::anyhow!("{}: {e}", T("lock_acquire_failed")))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Both lock tests mutate the process-global `NVM_LOCK_HELD` flag and
    // acquire the OS lock on the real nvm dir (resolved via `get_nvm_dir`).
    // Running them in parallel (cargo test's default) would let one test's
    // acquire race with the other's drop, producing flaky flag assertions
    // and self-deadlock on `flock(LOCK_EX)`. They also READ NVM_DIR via
    // `get_nvm_dir()`, so a parallel test in another module doing
    // `set_var("NVM_DIR", ..)` could point them at a temp dir that gets
    // restored mid-acquire. The process-global `ENV_TESTS_MUTEX` closes both
    // gaps: it serializes the lock tests against each other AND against
    // every other NVM_DIR-touching test across the crate.
    use crate::system::ENV_TESTS_MUTEX;

    #[test]
    fn acquire_nvm_lock_is_reentrant_in_same_process() {
        let _guard = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // Two nested acquires in the SAME process must not deadlock: the
        // inner one returns a no-op guard (re-entrant) because the outer
        // already holds the OS lock. This is the `nvm use --install` →
        // `install` path.
        let nvm_dir = crate::system::get_nvm_dir();
        let outer = acquire_nvm_lock(&nvm_dir).expect("outer acquire");
        // Inner acquire should succeed instantly (no-op guard), not block.
        let inner = acquire_nvm_lock(&nvm_dir).expect("inner re-entrant acquire");
        drop(inner);
        drop(outer);
        // After both drop, the flag must be cleared so a subsequent real
        // acquire works again.
        assert!(!NVM_LOCK_HELD.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn acquire_nvm_lock_can_be_reacquired_after_drop() {
        let _guard = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // Regression for the drop-order bug: previously `drop` released the
        // OS lock FIRST and only then cleared `NVM_LOCK_HELD`. If anything
        // went wrong between those two steps (panic, reentrant re-acquire
        // from another thread), the flag could be left `true` with the OS
        // lock free, making every subsequent same-process acquire take the
        // re-entrant no-op branch and silently bypass mutual exclusion.
        //
        // Here we exercise the acquire → drop → acquire → drop cycle several
        // times. After each drop the flag MUST be `false` (proving the flag
        // was cleared, not left dangling), and the next acquire MUST succeed
        // as a REAL acquire (proving the OS lock was actually released, not
        // leaked). If the OS lock were leaked, the second acquire would
        // self-deadlock on `flock(LOCK_EX)` and hang the test.
        let nvm_dir = crate::system::get_nvm_dir();
        for i in 0..5 {
            let guard = acquire_nvm_lock(&nvm_dir)
                .unwrap_or_else(|e| panic!("iteration {i}: acquire failed: {e}"));
            // While held, the flag must be true.
            assert!(
                NVM_LOCK_HELD.load(std::sync::atomic::Ordering::Acquire),
                "iteration {i}: flag not set after acquire"
            );
            drop(guard);
            // After drop, the flag must be cleared before any further acquire.
            assert!(
                !NVM_LOCK_HELD.load(std::sync::atomic::Ordering::Acquire),
                "iteration {i}: flag not cleared after drop — drop order regression"
            );
        }
    }
}
