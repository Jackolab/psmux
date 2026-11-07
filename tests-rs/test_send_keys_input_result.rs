// send-keys on the one-shot CLI path acks only input the pane writer took.
// The pane writer runs on its own thread, so the handler asks the server
// loop for the writer's result behind the keys (CtrlReq::InputResult) and
// reports a failed write as an error instead of an ack.

use super::*;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

const TEST_KEY: &str = "unit-test-key";

fn spawn_handler(listener: TcpListener, tx: mpsc::Sender<CtrlReq>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("handler: accept");
        handle_connection(stream, tx, TEST_KEY, Arc::new(RwLock::new(std::collections::HashMap::new())));
    })
}

/// A server loop that answers every InputResult with `took`, or never.
fn spawn_loop(rx: mpsc::Receiver<CtrlReq>, took: Option<bool>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(req) = rx.recv() {
            if let CtrlReq::InputResult(reply) = req {
                if let Some(took) = took {
                    let _ = reply.send(took);
                } else {
                    std::mem::forget(reply);
                }
            }
        }
    })
}

fn send_keys(took: Option<bool>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let (tx, rx) = mpsc::channel::<CtrlReq>();
    let handler = spawn_handler(listener.try_clone().unwrap(), tx);
    let server = spawn_loop(rx, took);
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).expect("connect");
    client.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    write!(client, "AUTH {TEST_KEY}\n").unwrap();
    write!(client, "send-keys hello Enter\n").unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).expect("read response");
    handler.join().expect("handler thread");
    server.join().expect("server loop");
    resp
}

#[test]
fn send_keys_whose_write_failed_is_an_error() {
    assert_eq!(send_keys(Some(false)), "OK\nERROR: send-keys: pane input failed\n");
}

#[test]
fn send_keys_the_writer_took_is_an_ack() {
    assert_eq!(send_keys(Some(true)), "OK\n");
}

#[test]
fn send_keys_with_no_writer_result_in_time_is_still_an_ack() {
    assert_eq!(send_keys(None), "OK\n");
}
