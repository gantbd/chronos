use super::*;
#[test]
fn actual_stdio_scheduling_tool_preserves_guards_and_observes_the_atomic_service() {
    let path = std::path::PathBuf::from(format!("/private/tmp/ch-mcp-s-{}", std::process::id()));
    let mut server = chronos_standalone::Server::bind_with_options(
        &path,
        chronos_standalone::ServerOptions {
            atomic_state: true,
            allow_writes: true,
            ..Default::default()
        },
    )
    .unwrap();
    let service = std::thread::spawn(move || {
        for _ in 0..6 {
            server.serve_one().unwrap();
        }
        server.runtime_signals().scheduling.unwrap()
    });
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_chronos-mcp"))
            .arg(&path)
            .arg("--scheduling")
            .arg("--allow-writes")
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
        if value.get("id").is_none() {
            return Value::Null;
        }
        receive.recv_timeout(Duration::from_secs(5)).unwrap()
    };
    assert_eq!(
        request(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
        )["result"]["protocolVersion"],
        "2025-11-25"
    );
    // Notification plus the next request are written in order on one stream.
    request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let list = request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    assert_eq!(list["result"]["tools"][1]["name"], "chronos_schedule");
    let mut tool = |command: Value| {
        request(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"chronos_schedule","arguments":{"version":5,"command":command}}}),
        )
    };
    assert_eq!(
        tool(json!({"operation":"enable","expected":{"kind":"finite","value":"0"}}))["result"]["isError"],
        false
    );
    let overview =
        || json!({"operation":"overview","now":{"before_epoch":false,"seconds":"0","nanos":0}});
    assert_eq!(
        tool(overview())["result"]["structuredContent"]["outcome"]["revision"]["value"],
        "1"
    );
    assert_eq!(
        tool(
            json!({"operation":"set_concurrency_limit","expected":{"kind":"finite","value":"1"},"limit":2})
        )["result"]["isError"],
        false
    );
    assert_eq!(
        tool(
            json!({"operation":"set_paused","expected":{"kind":"finite","value":"1"},"paused":true})
        )["result"]["isError"],
        true
    );
    assert_eq!(
        tool(
            json!({"operation":"set_paused","expected":{"kind":"finite","value":"2"},"paused":true})
        )["result"]["isError"],
        false
    );
    let observed = tool(overview());
    assert_eq!(
        observed["result"]["structuredContent"]["outcome"]["controls"]["paused"],
        true
    );
    assert_eq!(
        observed["result"]["structuredContent"]["outcome"]["concurrency"]["capacity"],
        2
    );
    drop(input);
    assert!(child.0.wait().unwrap().success());
    let signals = service.join().unwrap();
    assert_eq!(
        (signals.calls, signals.controls, signals.queries),
        (6, 4, 2)
    );
    assert_eq!(signals.domain_refused, 1);
    for entry in std::fs::read_dir(&path).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(path).unwrap();
}
