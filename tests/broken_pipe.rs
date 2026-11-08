//! A closed stdout pipe must end the process quietly, not with a panic.
//!
//! Regression test for the crash the release smoke test found on macOS in
//! #131: `nvm --help | grep -q "Usage: nvm"` panicked with
//! `failed printing to stdout: Broken pipe (os error 32)`. `grep -q` exits the
//! moment it matches, closing the pipe while nvm is still writing help text.
//!
//! Windows is not immune — the same race there reports `os error 232`, with a
//! *localized* error string (a Chinese Windows prints `管道正在被关闭。`), which
//! is why the fix matches on numeric codes rather than the message text.

mod common;

use std::process::{Command, Stdio};

use common::{isolated_nvm_dir, nvm_bin};

#[test]
fn help_with_closed_stdout_does_not_panic() {
    let (dir, nvm_dir) = isolated_nvm_dir();

    // stderr goes to a file rather than a pipe: draining a pipe only after
    // `wait()` can deadlock once the buffer fills, and we only inspect stderr
    // once the child is gone.
    let stderr_path = dir.path().join("stderr.txt");
    let stderr_sink = std::fs::File::create(&stderr_path).expect("create stderr sink");

    let mut cmd = Command::new(nvm_bin());
    cmd.arg("--help")
        .env("NVM_DIR", &nvm_dir)
        .env("HOME", &nvm_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr_sink));
    // Windows: get_home_dir() falls back to USERPROFILE, so isolate it too —
    // a code path reading it directly would otherwise see the real profile.
    #[cfg(windows)]
    cmd.env("USERPROFILE", &nvm_dir);

    let mut child = cmd.spawn().expect("spawn nvm --help");

    // Close the read end before the child gets as far as writing its help
    // text. This is what `| grep -q` and `| head -1` do from the consumer side,
    // and what `| true` does immediately.
    drop(child.stdout.take().expect("piped stdout"));

    let status = child.wait().expect("wait for nvm");

    let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
    assert!(
        !stderr.contains("panicked"),
        "nvm must not panic when stdout closes early; stderr was:\n{stderr}"
    );
    assert_ne!(
        status.code(),
        Some(101),
        "exit code 101 means the process panicked; stderr was:\n{stderr}"
    );

    // On Unix the fix restores the default SIGPIPE disposition, so the child
    // dies by signal exactly like `ls | head -1` does. Asserting that also
    // keeps the test honest: without it, a child that simply won the race and
    // finished writing would pass vacuously without exercising the bug.
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(13), // SIGPIPE
            "expected the child to die from SIGPIPE; stderr was:\n{stderr}"
        );
    }
}
