//! Shared helpers for integration tests in `tests/`.

use aikv::command::CommandExecutor;
use aikv::protocol::parser::RespParser;
use aikv::protocol::RespValue;
use aikv::Result;
use bytes::Bytes;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

/// `CommandExecutor::execute` takes an extra `allow_importing_slot_once` flag when built with
/// `--features cluster`; keep integration tests working for both configurations.
pub fn exec_cmd(
    executor: &CommandExecutor,
    command: &str,
    args: &[Bytes],
    current_db: &mut usize,
    client_id: usize,
) -> Result<RespValue> {
    #[cfg(feature = "cluster")]
    {
        executor.execute(command, args, current_db, client_id, false)
    }
    #[cfg(not(feature = "cluster"))]
    {
        executor.execute(command, args, current_db, client_id)
    }
}

/// Minimal Redis-compatible TCP target for `MIGRATE` tests: replies `+OK` to each RESP command
/// (`ASKING`, `SELECT`, `RESTORE`, …). Binds `127.0.0.1:0`; returns the chosen port.
///
/// The listener thread runs for the rest of the process (fine for `cargo test`).
#[allow(dead_code)] // Only used by `basic_commands_test`; other integration crates still compile `common`.
pub fn migrate_target_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind migrate mock");
    let port = listener.local_addr().expect("local_addr").port();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else {
                continue;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            handle_migrate_mock_session(&mut stream);
        }
    });
    // Listener thread may not accept() immediately; avoid flaky connects in CI.
    thread::sleep(Duration::from_millis(50));
    port
}

#[allow(dead_code)]
fn handle_migrate_mock_session(stream: &mut TcpStream) {
    let mut parser = RespParser::new(65536);
    let mut buf = [0u8; 8192];
    loop {
        loop {
            match parser.parse() {
                Ok(Some(_)) => {
                    if stream.write_all(b"+OK\r\n").is_err() || stream.flush().is_err() {
                        return;
                    }
                }
                Ok(None) => break,
                Err(_) => return,
            }
        }
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => parser.feed(&buf[..n]),
            Err(_) => return,
        }
    }
}
