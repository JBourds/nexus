//! Cooperative startup barrier built on `flock(2)`.
//!
//! Replaces the previous `SIGSTOP` / `SIGCONT` handshake between the parent
//! (Nexus) and each protocol process. The earlier scheme had a race: if
//! `SIGCONT` was delivered before the child actually ran `kill -STOP $$`,
//! the SIGCONT was a no-op, the subsequent SIGSTOP suspended the child,
//! and no further signal arrived to wake it. A protocol that lost the race
//! would simply never start.
//!
//! `flock(2)` is race-free because the lock state is evaluated atomically
//! in the kernel — there is no user-space "did the signal arrive before or
//! after the wait?" window for the parent to mis-time.
//!
//! ## Protocol
//!
//! 1. Parent constructs a `BarrierLock`, which creates a unique file under
//!    the system temp dir and acquires `LOCK_EX` on it.
//! 2. Each spawned protocol's startup script `exec`s into
//!    `flock -s <path> <runner...>`, which blocks in the kernel on
//!    `LOCK_SH` until the parent's exclusive lock is released.
//! 3. Once the parent has finished FUSE registration it calls
//!    [`BarrierLock::release`]: the underlying `File` is dropped, the
//!    kernel releases `LOCK_EX`, and every blocked child atomically
//!    acquires its shared lock and exec's its runner.
//! 4. The barrier file is removed when the `BarrierLock` itself is dropped
//!    (i.e. when the simulation ends), which keeps the path valid for any
//!    protocol respawned mid-run.
//!
//! Ordering between the cgroup-freeze release and the lock release does
//! not matter: a child must be both unfrozen *and* past the flock to
//! execute, and either ordering converges to the same state.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct BarrierLock {
    path: PathBuf,
    /// `Some` while the exclusive lock is held; dropped (and the underlying
    /// fd closed, releasing the lock) by `release`.
    file: Option<File>,
}

impl std::fmt::Debug for BarrierLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BarrierLock")
            .field("path", &self.path)
            .field("locked", &self.file.is_some())
            .finish()
    }
}

impl BarrierLock {
    pub fn new() -> io::Result<Self> {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("nexus-barrier-{pid}-{nanos}"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        // SAFETY: `flock(2)` is safe to call on an open file descriptor.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            path,
            file: Some(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Drop the underlying file, releasing `LOCK_EX`. Idempotent.
    pub fn release(&mut self) {
        self.file.take();
    }
}

impl Drop for BarrierLock {
    fn drop(&mut self) {
        self.release();
        // Best-effort cleanup; if removal fails the only consequence is a
        // zero-byte file in the temp dir.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::FromRawFd;

    /// A second `LOCK_EX` attempt with `LOCK_NB` while the first holder
    /// is alive must fail (EWOULDBLOCK). After release, it must succeed.
    #[test]
    fn second_exclusive_acquire_blocks_until_release() {
        let barrier = BarrierLock::new().expect("acquire LOCK_EX");
        let path = barrier.path().to_path_buf();

        // Open a separate fd to the same file and try a non-blocking
        // exclusive lock. While the first holder is alive, this must fail.
        let fd = unsafe {
            libc::open(
                std::ffi::CString::new(path.to_str().unwrap())
                    .unwrap()
                    .as_ptr(),
                libc::O_RDWR,
            )
        };
        assert!(fd >= 0, "open second fd");
        let try_rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        assert!(try_rc != 0, "second LOCK_EX must block while first is held");

        drop(barrier);

        // After release, the same non-blocking lock attempt must succeed.
        let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, 0, "second LOCK_EX must succeed after release");
        // Closing the fd via dropping a File releases the lock cleanly.
        let _ = unsafe { File::from_raw_fd(fd) };
    }

    /// `release` must be idempotent and explicit-release must drop the
    /// lock without waiting for full `Drop`.
    #[test]
    fn explicit_release_unblocks_other_acquirers() {
        let mut barrier = BarrierLock::new().expect("acquire");
        let path = barrier.path().to_path_buf();
        barrier.release();
        barrier.release(); // idempotent

        let fd = unsafe {
            libc::open(
                std::ffi::CString::new(path.to_str().unwrap())
                    .unwrap()
                    .as_ptr(),
                libc::O_RDWR,
            )
        };
        assert!(fd >= 0);
        let rc = unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) };
        assert_eq!(rc, 0, "LOCK_SH must succeed after release");
        let _ = unsafe { File::from_raw_fd(fd) };
    }
}
