//! Tenax issue #2804: a spawned standby that exits early must be reaped,
//! and it must not litter the server-marker registry.
//!
//! Observed on a Linux rig: a live server accumulated ~190 `<defunct>`
//! children over ~3 h — one per `WARM_RESPAWN_MIN_INTERVAL` respawn —
//! plus ~190 `servers\<pid>` markers with `pid:0` bodies.
//!
//! * Zombie: the Unix spawn branch dropped the `Child`; a dropped handle
//!   is never waitpid'd, and standby early-exits are by design (duplicate
//!   `__warm__` mutex, orphan watchdog). The spawned handle now goes to a
//!   parked thread whose `wait()` collects the exit the moment it lands.
//! * Marker: every dying standby wrote `servers\<pid>` at startup, but the
//!   only consumers (the orphan reaper's select/terminate path) are
//!   Windows — off Windows the write is litter no reader can honour.

#[cfg(not(windows))]
use super::*;

/// A standby-shaped child that exits at once is collected by the reaper
/// thread — it must leave the process table entirely, never sit in `Z`.
#[cfg(not(windows))]
#[test]
fn dead_spawn_is_reaped_not_zombied() {
    let pid = spawn_standby_unix(
        std::path::Path::new("/bin/sh"),
        &["-c".into(), "exit 0".into()],
    )
    .expect("spawn sh");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid));
        match stat {
            // Reaped: the pid is gone from the table.
            Err(_) => return,
            Ok(stat) => {
                // Field 3 is the state letter; comm may contain spaces, so
                // read the char after the last ')'.
                let state = stat
                    .rfind(')')
                    .and_then(|i| stat[i + 2..].chars().next())
                    .unwrap_or('?');
                assert_ne!(state, 'Z', "spawn {} zombied instead of reaping", pid);
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "spawn {} still present after 5s — never reaped",
            pid
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// Off Windows the ownership marker is never written — its only readers
/// are the Windows reaper stubs, so writing was pure litter.
#[cfg(not(windows))]
#[test]
fn server_marker_is_not_written_off_windows() {
    let pid = u32::MAX - 7;
    crate::session::write_server_marker(pid);
    assert!(!std::path::Path::new(&crate::paths::server_marker_file(pid)).exists());
}
