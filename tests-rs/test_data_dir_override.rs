// Regression tests for the PSMUX_DATA_DIR override (a41720f).
//
// Every data-directory consumer used to resolve the root on its own,
// mixing USERPROFILE/HOME; the fix routes them all through
// `paths::psmux_dir()` and lets an absolute PSMUX_DATA_DIR take precedence.
//
// Contracts locked here:
//
//   - with PSMUX_DATA_DIR set, every derived path (.port/.key/.sid/.pid/
//     .spawnlock, fixed-name files, server markers) lands under the
//     override root
//   - trailing separators are trimmed
//   - a relative or empty value is rejected (assertion panic)
//   - with the variable unset, the default home-based root is restored
//
// Env is process-global, so every test in this file serializes on the
// CRATE-WIDE crate::util::lock_test_env() and restores the previous value
// via a Drop guard. A file-private mutex is NOT enough: these tests mutate
// USERPROFILE/HOME/PSMUX_DATA_DIR, which tests_issue474_home_resolution
// (shared lock) and the inline paths::tests readers (same vars) also touch,
// so a private lock serialized this file against itself while still racing
// its siblings — the source of one-off failures of
// unset_data_dir_falls_back_to_home_root and
// psmux_dir_is_home_relative_dot_psmux in full parallel runs.

use super::*;

/// An absolute path for this platform (is_absolute() semantics differ).
#[cfg(windows)]
fn abs_root() -> &'static str {
    "C:\\psmux\\data\\root"
}

#[cfg(not(windows))]
fn abs_root() -> &'static str {
    "/psmux/data/root"
}

/// Restores a mutated env var on drop, so a failure mid-test cannot leak
/// the value into other tests.
struct EnvGuard {
    var: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(var: &'static str, value: &str) -> EnvGuard {
        let prev = std::env::var_os(var);
        std::env::set_var(var, value);
        EnvGuard { var, prev }
    }

    fn remove(var: &'static str) -> EnvGuard {
        let prev = std::env::var_os(var);
        std::env::remove_var(var);
        EnvGuard { var, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => std::env::set_var(self.var, value),
            None => std::env::remove_var(self.var),
        }
    }
}

/// With PSMUX_DATA_DIR set, every derived path must live under the override
/// root; removing the variable restores the default root.
#[test]
fn data_dir_env_reroutes_every_derived_path() {
    let _lock = crate::util::lock_test_env();
    let _clean = EnvGuard::remove("PSMUX_DATA_DIR");
    let default_dir = psmux_dir();

    let _override = EnvGuard::set("PSMUX_DATA_DIR", abs_root());
    assert_eq!(psmux_dir(), abs_root());
    assert_eq!(psmux_dir_opt(), Some(abs_root().to_string()));
    assert_eq!(port_file("sess"), sep_join(abs_root(), "sess.port"));
    assert_eq!(key_file("sess"), sep_join(abs_root(), "sess.key"));
    assert_eq!(sid_file("sess"), sep_join(abs_root(), "sess.sid"));
    assert_eq!(pid_file("sess"), sep_join(abs_root(), "sess.pid"));
    assert_eq!(
        spawnlock_file("sess"),
        sep_join(abs_root(), "sess.spawnlock")
    );
    assert_eq!(
        port_file_opt("sess"),
        Some(sep_join(abs_root(), "sess.port"))
    );
    assert_eq!(
        psmux_dir_file("latency.log"),
        sep_join(abs_root(), "latency.log")
    );
    assert_eq!(server_marker_dir(), sep_join(abs_root(), "servers"));
    assert_eq!(
        server_marker_file(42),
        join_parts(&[abs_root(), "servers", "42"])
    );

    drop(_override);
    assert_eq!(
        psmux_dir(),
        default_dir,
        "removing PSMUX_DATA_DIR must restore the home-based root"
    );
}

/// Trailing '/' and '\\' are trimmed from the override root.
#[test]
fn data_dir_env_trims_trailing_separators() {
    let _lock = crate::util::lock_test_env();
    for raw in [format!("{}/", abs_root()), format!("{}\\", abs_root())] {
        let _g = EnvGuard::set("PSMUX_DATA_DIR", &raw);
        assert_eq!(psmux_dir(), abs_root(), "raw value {raw:?}");
    }
}

/// A relative or empty PSMUX_DATA_DIR is a configuration error and must be
/// rejected loudly, never silently resolved relative to the CWD.
///
/// Exercises the validator directly instead of writing the bad value into the
/// process-global environment: an invalid PSMUX_DATA_DIR in the env, even
/// briefly and under lock_test_env, panics any concurrently running test
/// whose call chain reads psmux_dir_opt without that lock (observed as a
/// one-off failure of resize_window tests in full parallel runs).
#[test]
fn relative_or_empty_data_dir_is_rejected() {
    for bad in ["", ".", "relative/path", "C:relative"] {
        let raw = std::ffi::OsString::from(bad);
        let result = std::panic::catch_unwind(|| normalize_data_dir_override(&raw));
        assert!(
            result.is_err(),
            "PSMUX_DATA_DIR={bad:?} must panic instead of resolving"
        );
    }
}

/// Without PSMUX_DATA_DIR the data root falls back to the home directory,
/// and the fallback is unaffected by the override code path.
#[test]
fn unset_data_dir_falls_back_to_home_root() {
    let _lock = crate::util::lock_test_env();
    let _clean = EnvGuard::remove("PSMUX_DATA_DIR");
    // Pin USERPROFILE so the fallback is deterministic on every platform
    // (home_dir() prefers USERPROFILE over HOME).
    let _profile = EnvGuard::set("USERPROFILE", "/unit/test_home");
    let _home = EnvGuard::remove("HOME");
    let _drive = EnvGuard::remove("HOMEDRIVE");
    let _path = EnvGuard::remove("HOMEPATH");

    assert_eq!(psmux_dir(), join_parts(&["/unit/test_home", ".psmux"]));
    assert_eq!(
        psmux_dir_opt(),
        Some(join_parts(&["/unit/test_home", ".psmux"]))
    );
    assert_eq!(
        port_file("sess"),
        join_parts(&["/unit/test_home", ".psmux", "sess.port"])
    );
}
