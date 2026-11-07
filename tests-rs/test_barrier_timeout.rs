//! `send_control` acks a command only once the server's barrier reply came
//! back: a read that times out is not a confirmation.

use std::net::{TcpListener, TcpStream};
use std::time::Duration;

fn silent_server() -> (TcpListener, TcpStream) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
    c.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
    (l, c)
}

#[test]
fn a_barrier_that_never_answers_is_an_error() {
    let (_server, mut c) = silent_server();
    let e = crate::session::drain_barrier(&mut c).expect_err("a timeout is not an ack");
    assert!(matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock), "{e:?}");
}

#[test]
fn a_connection_the_server_closed_is_an_ack() {
    let (server, mut c) = silent_server();
    drop(server.accept().unwrap().0);
    crate::session::drain_barrier(&mut c).expect("EOF after the barrier");
}

#[test]
fn an_error_line_before_the_barrier_is_an_error() {
    use std::io::Write;
    let (server, mut c) = silent_server();
    let (mut s, _) = server.accept().unwrap();
    s.write_all(b"ERROR: send-keys: pane input failed\n{\"session\":\"x\"}\n").unwrap();
    drop(s);
    let e = crate::session::drain_barrier(&mut c).expect_err("an ERROR line is not an ack");
    assert_eq!(e.to_string(), "send-keys: pane input failed");
}
