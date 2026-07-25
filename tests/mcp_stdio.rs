//! End-to-end test of `lh mcp serve`: spawn the built binary, speak JSON-RPC
//! over its stdio, and verify the initialize/tools-list handshake. No network
//! is required (both methods are handled locally).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn mcp_serve_initialize_and_list_tools() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lh"))
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn lh mcp serve");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // initialize
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .unwrap();
    stdin.flush().unwrap();

    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSON response");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "litehouse");

    // tools/list
    line.clear();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n")
        .unwrap();
    stdin.flush().unwrap();
    stdout.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    let names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"deploy".to_string()));
    assert!(names.contains(&"list_apps".to_string()));

    // Closing stdin ends the serve loop (EOF).
    drop(stdin);
    let _ = child.wait();
}
