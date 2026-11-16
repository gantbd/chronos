#![forbid(unsafe_code)]

#[cfg(unix)]
fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(directory) = args.next() else {
        eprintln!(
            "usage: chronos-mcp PRIVATE_SERVICE_DIRECTORY [--allow-writes] [--scheduling] [--dependencies]"
        );
        std::process::exit(2);
    };
    if directory == "--help" {
        println!(
            "chronos-mcp PRIVATE_SERVICE_DIRECTORY [--allow-writes] [--scheduling] [--dependencies]\nMCP 2025-11-25 stdio client of the running Chronos local service. Default connector is read-only. Service authorization still applies. --scheduling opts into the V5 scheduling tool on atomic local deployments; it grants no execution or executor-stop authority."
        );
        return;
    }
    let (mut writes, mut scheduling) = (false, false);
    let mut dependencies = false;
    for flag in args {
        if flag == "--allow-writes" && !writes {
            writes = true;
        } else if flag == "--scheduling" && !scheduling {
            scheduling = true;
        } else if flag == "--dependencies" && !dependencies {
            dependencies = true;
        } else {
            eprintln!("unknown or duplicate option");
            std::process::exit(2);
        }
    }
    let mut adapter = chronos_mcp::Adapter::new(
        chronos_local::LocalClient::new(std::path::PathBuf::from(directory)),
        writes,
    );
    if dependencies {
        adapter = adapter.with_dependencies();
    }
    if scheduling {
        adapter = adapter.with_scheduling();
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    loop {
        let frame = match chronos_wire::read_frame(&mut reader, chronos_mcp::MAX_MESSAGE_BYTES) {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(_) => {
                eprintln!("MCP framing refused");
                std::process::exit(1);
            }
        };
        if let Some(response) = adapter.handle(&frame)
            && chronos_wire::write_frame(&mut writer, &response, chronos_mcp::MAX_MESSAGE_BYTES)
                .is_err()
        {
            eprintln!("MCP delivery failed; outcome may be unknown");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("this MCP composition requires the Unix local service profile");
    std::process::exit(1);
}
