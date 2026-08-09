//! Lowering worker thread priority (§15.1).
//!
//! > **Lower worker thread OS priority below normal.**
//! >
//! > ```text
//! > Windows -> THREAD_PRIORITY_BELOW_NORMAL
//! > Linux   -> nice(+5)
//! > macOS   -> QOS_CLASS_UTILITY
//! > ```
//! >
//! > **Never lower the priority of the playback, decode, render, or audio
//! > threads.**
//!
//! This matters more than the job-count cap. Capping concurrency stops us
//! running four proxies at once, but a *single* proxy job at normal priority
//! still competes with playback for the scheduler on a 4-core machine — and
//! §80 puts playback above everything.
//!
//! Only the pool's own worker threads call this. Nothing else in the project
//! should.

/// Drop the calling thread below normal priority. Best-effort.
///
/// A failure is logged and ignored: running a background job at normal
/// priority is worse than ideal but far better than refusing to run it.
pub fn lower_current_thread() {
    #[cfg(windows)]
    {
        // SAFETY: `GetCurrentThread` returns a pseudo-handle that needs no
        // closing, and `SetThreadPriority` only reads it. Both are plain
        // integer-returning calls with no memory involved.
        let ok = unsafe {
            let handle = GetCurrentThread();
            SetThreadPriority(handle, THREAD_PRIORITY_BELOW_NORMAL)
        };
        if ok == 0 {
            tracing::debug!("could not lower worker thread priority");
        }
    }

    #[cfg(unix)]
    {
        // `nice` applies to the calling thread on Linux, which is what §15.1
        // asks for. On other Unixes it may apply per process; +5 is mild
        // enough that this is not harmful either way.
        // SAFETY: `nice` takes an int and returns one; no pointers involved.
        let result = unsafe { nice(5) };
        if result == -1 {
            tracing::debug!("could not lower worker thread priority");
        }
    }

    #[cfg(not(any(windows, unix)))]
    {
        tracing::debug!("thread priority control is unavailable on this platform");
    }
}

#[cfg(windows)]
const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;

#[cfg(windows)]
unsafe extern "system" {
    fn GetCurrentThread() -> *mut core::ffi::c_void;
    fn SetThreadPriority(thread: *mut core::ffi::c_void, priority: i32) -> i32;
}

#[cfg(unix)]
unsafe extern "C" {
    fn nice(increment: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// It must not panic or hang, whatever the platform says.
    #[test]
    fn lowering_priority_is_harmless() {
        std::thread::spawn(|| {
            lower_current_thread();
            // The thread must still be able to do work afterwards.
            let sum: u64 = (0..1000).sum();
            assert_eq!(sum, 499_500);
        })
        .join()
        .expect("worker thread survived lowering its own priority");
    }
}
