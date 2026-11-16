//! Explicit composition-root signal supervision. Never installed by binding or
//! embedding a Server. Signals request drain; they cannot cancel external work.
use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    thread::JoinHandle,
};

pub struct SignalSupervisor {
    handle: signal_hook::iterator::Handle,
    worker: Option<JoinHandle<()>>,
}
impl SignalSupervisor {
    pub fn install(mut wake: UnixStream) -> io::Result<Self> {
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
        ])?;
        let handle = signals.handle();
        let worker = std::thread::Builder::new()
            .name("chronos-signals".into())
            .spawn(move || {
                // One bounded monotone request. No repeated writes, path lookup,
                // logging or persistence in a signal handler.
                let mut requested = false;
                for _ in signals.forever() {
                    if !requested {
                        if wake.write_all(&[1]).is_err() {
                            break;
                        }
                        requested = true;
                    }
                }
            })?;
        Ok(Self {
            handle,
            worker: Some(worker),
        })
    }
}
impl Drop for SignalSupervisor {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
