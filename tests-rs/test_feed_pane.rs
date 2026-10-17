// feed-pane (tenax #2710): bytes fed through `feed_pane_output` are pane
// OUTPUT — they enter the live vt100 parser, bump the data version, and
// run the reader loop's byte scans, exactly like the PTY read path. The
// host's stuck-alt Fix writes RMCUP this way: `#{pane_tty}` is a compat
// label with no file behind it, so open(2) on it can never clear
// `#{alternate_on}`.

use super::*;
use std::io::Read;

fn tcp_pair() -> (std::net::TcpStream, std::net::TcpStream) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let accept = std::thread::spawn(move || listener.accept().expect("accept").0);
    let client = std::net::TcpStream::connect(addr).expect("connect");
    let server = accept.join().expect("join accept thread");
    (client, server)
}

/// A proxy pane plus the peer end of its input pipe: `pane.writer` writes
/// land on `child_input`, so a test can prove feed bytes never reach the
/// child.
fn proxy_pane_with_input_peer() -> (crate::types::Pane, std::net::TcpStream) {
    let (reader, _r) = tcp_pair();
    let (writer, child_input) = tcp_pair();
    let pane = crate::proxy_pane::create_proxy_pane(
        reader,
        writer,
        "127.0.0.1:1".to_string(),
        "test-key".to_string(),
        "test-session".to_string(),
        0,
        None,
        "test-pane".to_string(),
        24,
        80,
        7,
        None,
    )
    .expect("create proxy pane");
    (pane, child_input)
}

fn alternate_on(pane: &crate::types::Pane) -> bool {
    pane.term
        .lock()
        .map(|p| p.screen().alternate_screen())
        .unwrap_or(false)
}

#[test]
fn feed_pane_rmcup_leaves_the_alternate_screen() {
    let (pane, _child_input) = proxy_pane_with_input_peer();
    // The child's own output puts the pane in alt, like a TUI that crashed.
    feed_pane_output(&pane, b"\x1b[?1049h");
    assert!(alternate_on(&pane), "SMGR setup: pane is on the alt screen");

    let version_before = pane.data_version.load(std::sync::atomic::Ordering::Acquire);
    feed_pane_output(&pane, b"\x1b[?1049l");

    assert!(
        !alternate_on(&pane),
        "fed RMCUP leaves the alternate screen — the Fix the unix tty write gives"
    );
    assert!(
        pane.data_version.load(std::sync::atomic::Ordering::Acquire) > version_before,
        "a feed bumps data_version so the render pass notices"
    );
}

#[test]
fn feed_pane_rmcup_resets_cursor_shape_tracking() {
    let (pane, _child_input) = proxy_pane_with_input_peer();
    pane.cursor_shape.store(5, std::sync::atomic::Ordering::Release);

    feed_pane_output(&pane, b"\x1b[?1049l");

    assert_eq!(
        pane.cursor_shape.load(std::sync::atomic::Ordering::Acquire),
        0,
        "RMCUP resets cursor shape, same as the reader loop's scan"
    );
}

#[test]
fn feed_pane_never_reaches_the_child() {
    let (pane, mut child_input) = proxy_pane_with_input_peer();
    child_input
        .set_read_timeout(Some(std::time::Duration::from_millis(200)))
        .expect("read timeout");

    feed_pane_output(&pane, b"\x1b[?1049l");

    let mut buf = [0u8; 16];
    match child_input.read(&mut buf) {
        Err(e)
            if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut => {}
        other => panic!("output feed wrote to the child's input pipe: {:?}", other),
    }
}
