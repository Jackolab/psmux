// Regression tests for the transient-write-error fix (60a4650).
//
// The queued pane writer used to end its thread on the FIRST failed write,
// which dropped the ConPTY master writer. Closing that handle closes the
// child's input pipe, and a shell whose stdin hits EOF exits — so one
// transient write failure while a full-screen TUI child was tearing down
// could take down the whole pane, close the window, and (for the last
// window) end the session.
//
// The fix: a failed write only stops further writes; the thread — and with
// it the inner writer — goes away only when the queue side is dropped with
// the pane. These tests inject failures into a dummy inner writer and
// assert the thread survives and the inner writer is NOT released while the
// queue side is alive.

use super::*;

use parking_lot::Mutex;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Poll `cond` until it is true or the deadline passes.
fn wait_until(what: &str, mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if cond() {
        return true;
    }
    eprintln!("wait_until timed out: {what}");
    false
}

/// An inner writer that fails exactly once, on its `fail_on`-th call (like
/// a PTY pipe erroring while a TUI child tears down), then records every
/// later write, and flags its own drop — the drop is the regression signal:
/// pre-fix the thread broke on the failure and the inner writer was dropped
/// while the pane was still alive.
struct FailOnNthWriter {
    fail_on: usize,
    attempts: Arc<AtomicUsize>,
    bytes: Arc<Mutex<Vec<u8>>>,
    dropped: Arc<AtomicBool>,
}

impl Write for FailOnNthWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt == self.fail_on {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "injected transient PTY write failure",
            ));
        }
        self.bytes.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for FailOnNthWriter {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// A transient write error must NOT end the writer thread: the inner writer
/// (the ConPTY master handle) stays alive while the queue side is alive,
/// and later writes are consumed without panicking.
#[test]
fn transient_write_error_keeps_the_writer_alive() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let inner = FailOnNthWriter {
        fail_on: 1,
        attempts: attempts.clone(),
        bytes: bytes.clone(),
        dropped: dropped.clone(),
    };

    let mut queue = spawn_pane_write_queue(Box::new(inner));

    // The first write hits the injected failure inside the writer thread.
    assert_eq!(queue.write(b"first").unwrap(), 5, "enqueue must still succeed");
    // A later write is consumed by the still-alive thread.
    assert_eq!(queue.write(b"second").unwrap(), 6);

    assert!(
        wait_until("failing write attempted", || attempts.load(Ordering::SeqCst) >= 1, Duration::from_secs(5)),
        "the writer thread must attempt the queued write"
    );
    // Exactly one attempt: the failure stops further writes instead of
    // starting a retry storm.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "the failed write must not be retried"
    );
    assert!(
        !dropped.load(Ordering::SeqCst),
        "a transient failure must NOT release the inner writer (releasing it EOFs the child's stdin)"
    );

    // Pane close: dropping the queue side is what ends the thread.
    drop(queue);
    assert!(
        wait_until("thread exits on queue drop", || dropped.load(Ordering::SeqCst), Duration::from_secs(5)),
        "the inner writer must only be released when the pane's queue is dropped"
    );
}

/// After a failure the thread stays alive but sheds further writes (the
/// documented "a failed write only stops further writes" contract): no
/// panic, no delivery, no release.
#[test]
fn writes_after_a_failure_are_consumed_without_delivery_or_release() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let inner = FailOnNthWriter {
        fail_on: 1,
        attempts: attempts.clone(),
        bytes: bytes.clone(),
        dropped: dropped.clone(),
    };

    let mut queue = spawn_pane_write_queue(Box::new(inner));
    assert_eq!(queue.write(b"boom").unwrap(), 4); // fails inside the thread
    // Queued or refused, depending on whether the thread has failed yet;
    // never delivered either way.
    let _ = queue.write(b"after");

    assert!(
        wait_until("failure attempted", || attempts.load(Ordering::SeqCst) >= 1, Duration::from_secs(5))
    );
    // Give the thread time to process the second write, then check state.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "writes after a failure must not be attempted again"
    );
    assert!(
        bytes.lock().is_empty(),
        "writes after a failure must not reach the broken inner writer"
    );
    assert!(
        !dropped.load(Ordering::SeqCst),
        "the writer must remain alive (and own the PTY handle) after a failure"
    );

    drop(queue);
    assert!(
        wait_until("thread exits on queue drop", || dropped.load(Ordering::SeqCst), Duration::from_secs(5))
    );
}

/// Writes delivered BEFORE a failure are not lost, and the failure itself
/// does not take the whole queue down.
#[test]
fn writes_before_the_failure_are_delivered() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let inner = FailOnNthWriter {
        fail_on: 2, // first call succeeds, second fails
        attempts: attempts.clone(),
        bytes: bytes.clone(),
        dropped: dropped.clone(),
    };

    let mut queue = spawn_pane_write_queue(Box::new(inner));
    assert_eq!(queue.write(b"ok").unwrap(), 2);
    assert!(
        wait_until("first write delivered", || !bytes.lock().is_empty(), Duration::from_secs(5)),
        "the write before the failure must be delivered"
    );

    assert_eq!(queue.write(b"boom").unwrap(), 4); // fails inside the thread
    // Queued or refused, depending on whether the thread has failed yet;
    // never delivered either way.
    let _ = queue.write(b"after");

    assert!(
        wait_until("failure attempted", || attempts.load(Ordering::SeqCst) >= 2, Duration::from_secs(5))
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "only the pre-failure write and the failing write may be attempted"
    );
    assert_eq!(*bytes.lock(), b"ok".to_vec(), "only the pre-failure write may be delivered");
    assert!(!dropped.load(Ordering::SeqCst), "writer must stay alive after the failure");

    drop(queue);
    assert!(wait_until("thread exits on queue drop", || dropped.load(Ordering::SeqCst), Duration::from_secs(5)));
}

/// Once the writer thread has marked the pane broken, a later write or paste
/// is refused (BrokenPipe, the job handed back) instead of being acked and
/// then dropped, so the caller can tell the input never reached the child.
#[test]
fn a_write_after_the_writer_broke_fails_instead_of_acking() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let inner = FailOnNthWriter {
        fail_on: 1,
        attempts: attempts.clone(),
        bytes: bytes.clone(),
        dropped: dropped.clone(),
    };

    let mut queue = spawn_pane_write_queue(Box::new(inner));
    assert_eq!(queue.write(b"boom").unwrap(), 4, "the failing write was queued before the failure");
    assert!(
        wait_until("failure attempted", || attempts.load(Ordering::SeqCst) >= 1, Duration::from_secs(5))
    );

    let refused = wait_until(
        "a later write is refused",
        || queue.write(b"after").is_err(),
        Duration::from_secs(5),
    );
    assert!(refused, "a write to a broken pane must fail, not ack");
    let err = queue.write(b"again").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    let job = InjectPaste { pid: 0, text: "paste".to_string(), normalize: false };
    assert!(queue.queue_inject_paste(job).is_err(), "a paste to a broken pane must be handed back");
    assert!(bytes.lock().is_empty(), "nothing reaches the broken inner writer");
    assert!(!dropped.load(Ordering::SeqCst), "the writer still owns the PTY handle");

    drop(queue);
    assert!(wait_until("thread exits on queue drop", || dropped.load(Ordering::SeqCst), Duration::from_secs(5)));
}

/// A mark queued behind writes answers whether every one of them reached
/// the pane: false once a write failed, true otherwise.
#[test]
fn a_mark_answers_whether_the_writes_before_it_reached_the_pane() {
    for (fail_on, took) in [(usize::MAX, true), (1, false)] {
        let inner = FailOnNthWriter {
            fail_on,
            attempts: Arc::new(AtomicUsize::new(0)),
            bytes: Arc::new(Mutex::new(Vec::new())),
            dropped: Arc::new(AtomicBool::new(false)),
        };
        let mut queue = spawn_pane_write_queue(Box::new(inner));
        assert_eq!(queue.write(b"keys").unwrap(), 4);
        let (reply, answer) = std::sync::mpsc::channel();
        queue.queue_mark(reply).expect("the queue takes a mark");
        assert_eq!(answer.recv_timeout(Duration::from_secs(5)), Ok(took), "fail_on {fail_on}");
    }
}
