use super::spawn_reader_thread;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Hands out one chunk, then blocks until released and reports EOF.
struct OneChunk {
    chunk: Option<Vec<u8>>,
    hold: mpsc::Receiver<()>,
}

impl std::io::Read for OneChunk {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(c) = self.chunk.take() {
            buf[..c.len()].copy_from_slice(&c);
            return Ok(c.len());
        }
        let _ = self.hold.recv();
        Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "released"))
    }
}

/// A control client may capture the pane as soon as it has seen %output, so
/// every byte in the %output ring must already be on the screen (tmux order).
/// The chunk is over ECHO_CHUNK_MAX, so the parser thread takes its
/// coalescing wait before it parses.
#[test]
fn output_ring_holds_only_bytes_the_screen_already_shows() {
    let mut chunk = vec![b'p'; 1100];
    chunk.extend_from_slice(b"\r\nq-42-z\r\n");
    let (release, hold) = mpsc::channel::<()>();
    let term = Arc::new(Mutex::new(vt100::Parser::new(30, 120, 0)));
    let ring = Arc::new(Mutex::new(VecDeque::new()));
    spawn_reader_thread(
        Box::new(OneChunk { chunk: Some(chunk), hold }),
        term.clone(),
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU8::new(0)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU32::new(0)),
        ring.clone(),
        8456,
        None,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while ring.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "no %output bytes within 10 s");
        std::thread::yield_now();
    }
    let text = term.lock().unwrap().screen().contents();
    drop(release);
    assert!(
        text.contains("q-42-z"),
        "%output carried bytes the screen does not show yet: {:?}",
        &text[text.len().saturating_sub(80)..]
    );
}
