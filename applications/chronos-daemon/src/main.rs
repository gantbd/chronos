#![forbid(unsafe_code)]

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            log(message);
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(unix)]
fn log(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{message}");
}

#[cfg(unix)]
fn run() -> Result<(), &'static str> {
    let mut args = std::env::args_os().skip(1);
    let Some(directory) = args.next() else {
        return Err("usage: chronos-service PRIVATE_DIRECTORY [--allow-writes]");
    };
    if directory == "--help" {
        println!(
            "chronos-service PRIVATE_DIRECTORY [--allow-writes] [--atomic-state] [--observer-history] [--allocation-receipts]\nchronos-service --migrate-envelope PRIVATE_DIRECTORY\nchronos-service --status|--shutdown|--maintain PRIVATE_DIRECTORY\nOwner-only local Unix sockets; read-only task policy unless the process owner enables writes. Allocation receipts require --atomic-state and explicitly upgrade the existing service envelope. Migration is explicit and preserves legacy checkpoints. Shutdown requests drain, not external-effect cancellation. No network authentication or automatic command retry."
        );
        return Ok(());
    }
    if directory == "--migrate-envelope" {
        let path = args.next().ok_or("migration directory required")?;
        if args.next().is_some() {
            return Err("unexpected migration argument");
        }
        chronos_standalone::migrate_to_envelope(std::path::Path::new(&path))?;
        log("Chronos service envelope migration committed; legacy checkpoint preserved");
        return Ok(());
    }
    if directory == "--status" || directory == "--shutdown" || directory == "--maintain" {
        use chronos_local::control::{CommandV1, request};
        let command = if directory == "--status" {
            CommandV1::Status
        } else if directory == "--shutdown" {
            CommandV1::Drain
        } else {
            CommandV1::Maintain
        };
        let path = args.next().ok_or("control directory required")?;
        if args.next().is_some() {
            return Err("unexpected control argument");
        }
        let status = request(std::path::Path::new(&path), command)
            .map_err(|_| "control outcome unavailable; inspect service, do not infer completion")?;
        println!(
            "lifecycle={:?} ready={} obligations={} maintenance={:?} entries={} committed={} temporary={} other={}",
            status.lifecycle,
            status.ready,
            status.obligations,
            status.maintenance,
            status.entries,
            status.committed,
            status.temporary,
            status.other
        );
        return Ok(());
    }
    let mut options = chronos_standalone::ServerOptions::default();
    for flag in args {
        if flag == "--allow-writes" && !options.allow_writes {
            options.allow_writes = true;
        } else if flag == "--atomic-state" && !options.atomic_state {
            options.atomic_state = true;
        } else if flag == "--observer-history" && !options.observer_history {
            options.observer_history = true;
        } else if flag == "--allocation-receipts" && !options.allocation_receipts {
            options.allocation_receipts = true;
        } else {
            return Err("unknown or duplicate option");
        }
    }
    let mut server =
        chronos_standalone::Server::bind_with_options(std::path::Path::new(&directory), options)?;
    let (wake, writer) =
        std::os::unix::net::UnixStream::pair().map_err(|_| "signal wake creation failed")?;
    wake.set_read_timeout(Some(chronos_local::IO_TIMEOUT))
        .map_err(|_| "signal timeout failed")?;
    writer
        .set_write_timeout(Some(chronos_local::IO_TIMEOUT))
        .map_err(|_| "signal timeout failed")?;
    let _signals = chronos_standalone::signals::SignalSupervisor::install(writer)
        .map_err(|_| "signal supervision unavailable")?;
    log("Chronos local service ready");
    server.run_with_shutdown(&wake)?;
    log("Chronos local service stopped");
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("this local service profile requires Unix");
    std::process::exit(1);
}
