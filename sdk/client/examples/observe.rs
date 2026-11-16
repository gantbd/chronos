//! Read-only headless consumer of the same SDK available to UI/agent hosts.
#![forbid(unsafe_code)]
#[cfg(unix)]
fn main() -> std::process::ExitCode {
    use chronos_sdk_client::{
        client::observer::{Observer, ObserverLimits},
        transport::LocalClient,
    };
    let mut args = std::env::args_os().skip(1);
    let Some(directory) = args.next() else {
        eprintln!("usage: observe PRIVATE_DIRECTORY");
        return std::process::ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("unexpected argument");
        return std::process::ExitCode::FAILURE;
    }
    let mut client = LocalClient::new(std::path::PathBuf::from(directory));
    let mut observer = match Observer::new(ObserverLimits {
        page_size: 64,
        max_pages: 16,
        max_entries: 1024,
        max_id_bytes: 256,
    }) {
        Ok(observer) => observer,
        Err(_) => {
            eprintln!("observer limits refused");
            return std::process::ExitCode::FAILURE;
        }
    };
    match observer.resync_feed(&mut client) {
        Ok(more) => {
            println!(
                "observed_tasks={} more_changes={more}",
                observer.view().map_or(0, |view| view.entries.len())
            );
            std::process::ExitCode::SUCCESS
        }
        Err(_) => {
            eprintln!(
                "observer unavailable; inspect profile, authority and limits; no automatic retry"
            );
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(not(unix))]
fn main() {
    eprintln!("the concrete local SDK transport requires Unix");
}
