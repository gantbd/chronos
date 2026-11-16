#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    process::{Command, Stdio},
    time::Duration,
};

struct Process(std::process::Child);
mod scheduling;
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn actual_mcp_agent_and_ui_model_share_one_durable_service() {
    let path = std::path::PathBuf::from(format!("/private/tmp/ch-mcp-{}", std::process::id()));
    let mut server = chronos_standalone::Server::bind(&path, true).unwrap();
    let service = std::thread::spawn(move || {
        for _ in 0..8 {
            server.serve_one().unwrap();
        }
    });
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_chronos-mcp"))
            .arg(&path)
            .arg("--allow-writes")
            .arg("--dependencies")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = child.0.stdin.take().unwrap();
    let output = child.0.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        while let Ok(Some(frame)) =
            chronos_wire::read_frame(&mut reader, chronos_mcp::MAX_MESSAGE_BYTES)
        {
            if send
                .send(serde_json::from_slice::<Value>(&frame).unwrap())
                .is_err()
            {
                break;
            }
        }
    });
    let mut request = |value: Value| {
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
        receive.recv_timeout(Duration::from_secs(5)).unwrap()
    };
    assert_eq!(
        request(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"harness-test","version":"1"}}})
        )["result"]["protocolVersion"],
        "2025-11-25"
    );
    // Ping is permitted before the initialized notification.
    let reply = request(json!({"jsonrpc":"2.0","id":2,"method":"ping"}));
    assert_eq!(reply["id"], 2);
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"command":{"operation":"submit_graph","expected":{"kind":"finite","value":"0"},"tasks":[{"id":"a","priority":0,"dependencies":[],"capabilities":[],"max_attempts":1,"retry_safety":"fence_unknown"}]}}}})).unwrap();
    input.flush().unwrap();
    let reply = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(reply["id"], 3);
    assert_eq!(reply["result"]["isError"], false);
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"command":{"operation":"revision"}}}})).unwrap();
    input.flush().unwrap();
    let reply = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        reply["result"]["structuredContent"]["outcome"]["revision"]["value"],
        "1"
    );
    // The same model consumed by both UIs observes the agent's original task.
    let mut human = chronos_model::Model::new(chronos_local::LocalClient::new(&path), true);
    human.apply(chronos_model::Action::Refresh);
    assert!(!human.view.mutation_blocked);
    assert_eq!(human.view.revision, "1");
    assert_eq!(human.view.rows.len(), 1);
    assert_eq!(human.view.rows[0].reference.task, "a");
    let original = human.view.rows[0].reference.incarnation.clone();
    human.apply(chronos_model::Action::Submit("human-task".into()));
    assert!(!human.view.mutation_blocked);
    assert_eq!(human.view.revision, "2");
    assert_eq!(human.view.rows.len(), 2);
    // The agent's next explicit read sees the human change at the same revision.
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"command":{"operation":"page","cursor":null,"limit":64}}}})).unwrap();
    input.flush().unwrap();
    let reply = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let outcome = &reply["result"]["structuredContent"]["outcome"];
    assert_eq!(outcome["revision"]["value"], human.view.revision);
    let entries = outcome["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["reference"]["task"], "a");
    assert_eq!(entries[0]["reference"]["incarnation"], original);
    assert_eq!(entries[1]["reference"]["task"], "human-task");
    for (revision, outcome) in [("2", "visible_neighbors"), ("1", "refused")] {
        writeln!(
            input,
            "{}",
            json!({
                "jsonrpc":"2.0","id":6,"method":"tools/call","params":{
                    "name":"chronos_dependencies","arguments":{"version":6,"command":{
                        "operation":"neighbors","expected":{"kind":"finite","value":revision},
                        "task":{"task":"a","incarnation":original},
                        "direction":"dependents","limit":4
                    }}
                }
            })
        )
        .unwrap();
        input.flush().unwrap();
        let reply = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        let value = &reply["result"]["structuredContent"]["outcome"];
        assert_eq!(value["outcome"], outcome);
        if revision == "2" {
            assert_eq!(value["neighbors"], json!([]));
            assert_eq!(value["truncated"], false);
            assert_eq!(reply["result"]["isError"], false);
        } else {
            assert_eq!(value["code"], "stale");
            assert_eq!(reply["result"]["isError"], true);
        }
    }
    assert_eq!(human.view.signals.requests, 3);
    assert_eq!(human.view.signals.backend_errors, 0);
    drop(input);
    assert!(child.0.wait().unwrap().success());
    service.join().unwrap();
    for entry in std::fs::read_dir(&path).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(path).unwrap();
}
