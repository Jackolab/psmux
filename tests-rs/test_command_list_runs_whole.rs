// A one-shot command list (`a ; b`) runs every command and answers with
// every command's output, in order.

use super::*;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::time::Duration;

const TEST_KEY: &str = "unit-test-key";

/// Sends `line` on one authenticated one-shot connection to a handler whose
/// server loop prints each display-message format as it is.
fn one_shot(line: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let (tx, rx) = mpsc::channel::<CtrlReq>();
    let accept = listener.try_clone().unwrap();
    let handler = std::thread::spawn(move || {
        let (stream, _) = accept.accept().expect("handler: accept");
        handle_connection(stream, tx, TEST_KEY, Arc::new(RwLock::new(std::collections::HashMap::new())));
    });
    let server = std::thread::spawn(move || {
        while let Ok(req) = rx.recv() {
            if let CtrlReq::DisplayMessage(reply, fmt, ..) = req {
                let _ = reply.send(fmt);
            }
        }
    });
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).expect("connect");
    client.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    write!(client, "AUTH {TEST_KEY}\n").unwrap();
    write!(client, "{line}\n").unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).expect("read response");
    handler.join().expect("handler thread");
    server.join().expect("server loop");
    resp
}

fn printed(resp: &str) -> Vec<&str> {
    resp.lines().filter(|l| *l != "OK").collect()
}

#[test]
fn both_commands_of_a_list_answer() {
    let resp = one_shot("display-message -p A ; display-message -p B");
    assert_eq!(printed(&resp), ["A", "B"], "{resp:?}");
}

#[test]
fn no_command_of_a_longer_list_is_skipped() {
    let resp = one_shot("display-message -p A ; display-message -p B ; display-message -p C");
    assert_eq!(printed(&resp), ["A", "B", "C"], "{resp:?}");
}
