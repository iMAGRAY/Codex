// Handles setting parent-death signal for child processes across platforms.
// On Linux, sets PR_SET_PDEATHSIG to SIGTERM and exits child if the parent has already died.

#[cfg(target_os = "linux")]
use libc;

/// Sets a death signal for the child process so it receives SIGTERM when the parent exits.
///
/// # Arguments
///
/// * `parent_pid` - PID of the parent process captured before fork.
#[cfg(target_os = "linux")]
pub fn set_parent_death(parent_pid: libc::pid_t) {
    unsafe {
        // Set the parent-death signal to SIGTERM. Ignore errors for portability.
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        // If the current parent PID differs from the captured one, the original parent died
        // between fork and this call. In that case, terminate this process immediately.
        if libc::getppid() != parent_pid {
            libc::kill(libc::getpid(), libc::SIGTERM);
        }
    }
}

/// On non-Linux platforms, setting a parent-death signal is a no-op.
#[cfg(not(target_os = "linux"))]
pub fn set_parent_death(_parent_pid: u32) {
    // No-op on non-Linux platforms.
}
