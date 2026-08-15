//! Whole-tree termination for the yt-dlp child process.
//!
//! `Child::kill()` / `start_kill()` is a bare `TerminateProcess` on the
//! direct child, and Windows does not cascade-kill descendants. That is not
//! good enough for yt-dlp, which is *always* at least two processes and
//! sometimes three:
//!
//! 1. The official `yt-dlp.exe` is a PyInstaller one-file build. Its
//!    bootloader extracts the payload to `%TEMP%\_MEIxxxx` and then runs
//!    the real program as a **child process of itself** — so the PID we
//!    spawn is not the PID doing the downloading. Verified on a live run:
//!    `unduhin-app.exe` → `yt-dlp.exe` (bootloader) → `yt-dlp.exe` (the
//!    downloader).
//! 2. yt-dlp spawns **ffmpeg** for merges, remuxes, and any HLS format it
//!    routes through `FFmpegFD`.
//!
//! Terminating only the process we spawned therefore leaves a live
//! downloader behind: it keeps writing to the output path (so a pause
//! doesn't pause and a delete doesn't delete), and — because it inherited
//! duplicates of our piped stdout/stderr handles — the pipes never reach
//! EOF, so draining them blocks forever and wedges the caller.
//!
//! A job object fixes both halves. The child is assigned to a job created
//! with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; every process it spawns
//! inherits the job, so one `TerminateJobObject` reaps the whole tree, and
//! merely *dropping* the guard does the same. That last property is what
//! covers the paths we can't write explicit cleanup for: a panicking
//! worker, and the 5-second bounded shutdown in the Tauri shell that ends
//! in `std::process::exit(0)`.
//!
//! On Unix the same two problems exist for the same two reasons, and the
//! POSIX counterpart is the process group. The child is spawned with
//! `process_group(0)`, which makes it the leader of a new group whose PGID
//! equals its PID; descendants inherit that group unless they call `setsid`
//! themselves, and neither yt-dlp nor ffmpeg does. One `killpg` then reaps
//! the tree.
//!
//! One difference matters. A job object reaps when its last handle closes,
//! which happens for free at process death; a process group does not, so
//! `Drop` has to signal explicitly. That still leaves a gap the Windows
//! build does not have: `std::process::exit(0)` in the Tauri shell's
//! bounded shutdown skips destructors, so on Unix the group must be
//! terminated before that point rather than relied on to unwind.

/// RAII handle to the process tree spawned for one yt-dlp run.
///
/// Held alongside the `Child` for the lifetime of the download. Dropping it
/// terminates every process still in the tree: on Windows by closing the
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` job handle, on Unix by signalling
/// the process group.
pub(crate) struct ProcessTreeGuard {
    #[cfg(target_os = "windows")]
    job: Option<windows::Win32::Foundation::HANDLE>,
    /// Process group id, always equal to the direct child's pid because the
    /// child was spawned with `process_group(0)`. `None` means "no tree to
    /// reap" and every operation becomes a no-op.
    #[cfg(unix)]
    pgid: Option<i32>,
}

#[cfg(target_os = "windows")]
mod imp {
    use super::ProcessTreeGuard;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // SAFETY: the guard owns a Windows job-object HANDLE. Handles are
    // process-wide kernel-object references, not thread-affine — the Win32
    // calls this type makes (`AssignProcessToJobObject`,
    // `TerminateJobObject`, `CloseHandle`) are all documented as callable
    // from any thread. `windows::HANDLE` is only `!Send`/`!Sync` because
    // it wraps a raw pointer. Exclusive ownership means no handle races:
    // the guard is never cloned, and `CloseHandle` runs once, in `Drop`.
    //
    // Required because the guard is held across an `.await` inside the
    // worker future, which `tokio::spawn` needs to be `Send`.
    unsafe impl Send for ProcessTreeGuard {}
    // SAFETY: as above; `&self` only reaches `TerminateJobObject`, which
    // is idempotent and thread-safe.
    unsafe impl Sync for ProcessTreeGuard {}

    impl ProcessTreeGuard {
        /// Create a kill-on-close job and put `child` in it.
        ///
        /// Best-effort by design: every failure path yields an inert guard
        /// rather than an error, because failing to build a job object is
        /// no reason to refuse to download. The caller still calls
        /// `start_kill()` on the direct child, so a degraded run behaves
        /// exactly like the pre-job-object code did.
        ///
        /// There is a narrow race — the bootloader could in principle
        /// spawn its child between `CreateProcess` returning and the
        /// assignment below. In practice the bootloader must first extract
        /// a ~30 MB payload to disk, which takes orders of magnitude
        /// longer than the two syscalls here.
        pub(crate) fn adopt(child: &tokio::process::Child) -> Self {
            let Some(process) = child.raw_handle() else {
                // Already exited; nothing to adopt.
                return Self { job: None };
            };
            // SAFETY: `CreateJobObjectW(None, None)` takes no borrowed
            // state and returns an owned handle or an error.
            let job = match unsafe { CreateJobObjectW(None, None) } {
                Ok(job) => job,
                Err(err) => {
                    tracing::warn!(%err, "ytdlp: CreateJobObject failed; tree kill unavailable");
                    return Self { job: None };
                }
            };

            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `job` is a live handle from the call above; `info`
            // outlives the call and its size is the one the class expects.
            let configured = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if let Err(err) = configured {
                tracing::warn!(%err, "ytdlp: SetInformationJobObject failed; tree kill unavailable");
                // SAFETY: `job` is a live handle we own and stop using here.
                let _ = unsafe { CloseHandle(job) };
                return Self { job: None };
            }

            // SAFETY: both handles are live — `job` from above, `process`
            // borrowed from a `Child` that outlives this call.
            let assigned = unsafe { AssignProcessToJobObject(job, HANDLE(process)) };
            if let Err(err) = assigned {
                tracing::warn!(%err, "ytdlp: AssignProcessToJobObject failed; tree kill unavailable");
                // SAFETY: as above.
                let _ = unsafe { CloseHandle(job) };
                return Self { job: None };
            }

            tracing::debug!("ytdlp: process tree adopted into job object");
            Self { job: Some(job) }
        }

        /// Terminate every process in the job, immediately.
        ///
        /// Idempotent and safe to call on an inert guard. `Drop` would do
        /// the same thing, but cancellation needs the tree gone *before*
        /// the caller drains the pipes, not whenever the future unwinds.
        pub(crate) fn terminate(&self) {
            let Some(job) = self.job else { return };
            // SAFETY: `job` is a live handle owned by this guard.
            if let Err(err) = unsafe { TerminateJobObject(job, 1) } {
                tracing::warn!(%err, "ytdlp: TerminateJobObject failed");
            }
        }
    }

    impl Drop for ProcessTreeGuard {
        fn drop(&mut self) {
            let Some(job) = self.job.take() else { return };
            // Closing the last handle to a KILL_ON_JOB_CLOSE job kills
            // whatever is still running in it.
            // SAFETY: `job` is a live handle owned by this guard and is
            // not used again after the take above.
            let _ = unsafe { CloseHandle(job) };
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::ProcessTreeGuard;

    impl ProcessTreeGuard {
        /// Record the child's process group.
        ///
        /// The caller must have spawned `child` with `process_group(0)`, so
        /// its pid *is* the pgid. We do not re-derive the group with
        /// `getpgid`: if that call raced the child's exit it could return
        /// our own group, and signalling that would kill the app.
        pub(crate) fn adopt(child: &tokio::process::Child) -> Self {
            // A pid of 0 or a missing id means the child is already gone.
            // Storing either would be catastrophic: `killpg(0, ...)`
            // signals *the caller's own* process group, taking down the
            // whole app. Refuse both.
            let pgid = match child.id() {
                Some(id) if id > 0 && i32::try_from(id).is_ok() => Some(id as i32),
                _ => {
                    tracing::debug!("ytdlp: no live child pid; tree kill unavailable");
                    None
                }
            };
            if pgid.is_some() {
                tracing::debug!("ytdlp: process tree adopted into process group");
            }
            Self { pgid }
        }

        /// Kill every process in the group, immediately.
        ///
        /// Idempotent and safe on an inert guard. SIGKILL rather than
        /// SIGTERM to match the Windows `TerminateJobObject` semantics: a
        /// paused or cancelled download must stop writing to the output
        /// file before the caller drains the pipes, and ffmpeg installs its
        /// own SIGTERM handler that flushes rather than stopping.
        pub(crate) fn terminate(&self) {
            let Some(pgid) = self.pgid else { return };
            debug_assert!(pgid > 0, "killpg on a non-positive pgid targets our group");
            // SAFETY: `pgid` is a positive process-group id captured from a
            // child we spawned as a group leader. `killpg` has no memory
            // safety requirements; the invariant that matters is that the
            // value is never 0, which `adopt` guarantees.
            if unsafe { libc::killpg(pgid, libc::SIGKILL) } != 0 {
                let err = std::io::Error::last_os_error();
                // ESRCH just means the group already exited.
                if err.raw_os_error() != Some(libc::ESRCH) {
                    tracing::warn!(%err, pgid, "ytdlp: killpg failed");
                }
            }
        }
    }

    impl Drop for ProcessTreeGuard {
        fn drop(&mut self) {
            // A process group has no kill-on-close behavior to inherit, so
            // the signal has to be explicit here.
            self.terminate();
            self.pgid = None;
        }
    }
}
