use super::*;
use chronos_wire::control::{CommandV1, LifecycleV1, MaintenanceV1, StatusV1};
use rustix::event::{PollFd, PollFlags, poll};
use std::io::{Read, Write};

enum Event {
    Data,
    Control,
    Signal,
    Worker,
    WorkerClosed,
}

fn poll_ready(fds: &mut [PollFd<'_>]) -> Result<(), &'static str> {
    loop {
        match poll(fds, None) {
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return Err("service readiness wait failed"),
        }
    }
}

fn ready_event(
    fds: &[PollFd<'_>],
    signal: Option<usize>,
    worker: Option<usize>,
) -> Result<Event, &'static str> {
    if fds
        .iter()
        .any(|fd| fd.revents().intersects(PollFlags::ERR | PollFlags::NVAL))
    {
        return Err("service descriptor failed");
    }
    if let Some(index) = signal {
        if fds[index].revents().contains(PollFlags::IN) {
            return Ok(Event::Signal);
        }
        if fds[index].revents().contains(PollFlags::HUP) {
            return Err("signal channel lost");
        }
    }
    // Operational control wins ties: no new data or worker request is admitted
    // after an already observable drain request.
    if fds[0].revents().contains(PollFlags::IN) {
        return Ok(Event::Control);
    }
    if let Some(index) = worker {
        if fds[index].revents().contains(PollFlags::IN) {
            return Ok(Event::Worker);
        }
        if fds[index].revents().contains(PollFlags::HUP) {
            return Ok(Event::WorkerClosed);
        }
    }
    if fds[1].revents().contains(PollFlags::IN) {
        return Ok(Event::Data);
    }
    Err("unexpected service readiness event")
}

impl Server {
    pub fn status(&mut self) -> Result<StatusV1, &'static str> {
        self.verify_ownership()?;
        Ok(self.status_snapshot())
    }

    fn status_snapshot(&self) -> StatusV1 {
        StatusV1 {
            lifecycle: self.lifecycle.state(),
            ready: self.lifecycle.ready(),
            obligations: self.lifecycle.active(),
            maintenance: MaintenanceV1::NotRequested,
            entries: 0,
            committed: 0,
            temporary: 0,
            other: 0,
        }
    }

    /// OS-readiness-driven local loop. No polling interval, hidden queue or worker
    /// pool. Only the composition root installs global signal handling.
    pub fn run(&mut self) -> Result<(), &'static str> {
        self.run_driver(None)
    }
    pub fn run_with_shutdown(&mut self, wake: &UnixStream) -> Result<(), &'static str> {
        self.run_driver(Some(wake))
    }

    fn run_driver(&mut self, wake: Option<&UnixStream>) -> Result<(), &'static str> {
        let result = (|| {
            loop {
                self.verify_ownership()?;
                match self.lifecycle.state() {
                    LifecycleV1::Draining => {
                        self.lifecycle
                            .stop()
                            .map_err(|_| "drain obligations remain")?;
                        return Ok(());
                    }
                    LifecycleV1::Stopped => return Ok(()),
                    LifecycleV1::Fenced => return Err("server fenced"),
                    LifecycleV1::Running => {}
                }
                match self.wait_event(wake)? {
                    Event::Data => self.serve_one()?,
                    Event::Control => self.serve_control()?,
                    Event::Worker => self.serve_worker()?,
                    Event::WorkerClosed => self.worker = None,
                    Event::Signal => {
                        let mut source = wake.ok_or("signal wake unavailable")?;
                        let mut token = [0; 1];
                        source
                            .read_exact(&mut token)
                            .map_err(|_| "signal wake failed")?;
                        if token != [1] {
                            return Err("invalid signal wake");
                        }
                        self.verify_ownership()?;
                        self.lifecycle.drain().map_err(|_| "drain refused")?;
                        increment(
                            &mut self.signals.signal_requests,
                            &mut self.signals.saturated,
                        );
                        increment(
                            &mut self.signals.drain_requests,
                            &mut self.signals.saturated,
                        );
                    }
                }
            }
        })();
        if result.is_err() {
            self.lifecycle.fence();
        }
        result
    }

    fn wait_event(&self, wake: Option<&UnixStream>) -> Result<Event, &'static str> {
        match (wake, self.worker.as_ref()) {
            (Some(signal), Some(worker)) => {
                let mut fds = [
                    PollFd::new(&self.control, PollFlags::IN),
                    PollFd::new(&self.listener, PollFlags::IN),
                    PollFd::new(signal, PollFlags::IN),
                    PollFd::new(&worker.wake, PollFlags::IN),
                ];
                poll_ready(&mut fds)?;
                ready_event(&fds, Some(2), Some(3))
            }
            (Some(signal), None) => {
                let mut fds = [
                    PollFd::new(&self.control, PollFlags::IN),
                    PollFd::new(&self.listener, PollFlags::IN),
                    PollFd::new(signal, PollFlags::IN),
                ];
                poll_ready(&mut fds)?;
                ready_event(&fds, Some(2), None)
            }
            (None, Some(worker)) => {
                let mut fds = [
                    PollFd::new(&self.control, PollFlags::IN),
                    PollFd::new(&self.listener, PollFlags::IN),
                    PollFd::new(&worker.wake, PollFlags::IN),
                ];
                poll_ready(&mut fds)?;
                ready_event(&fds, None, Some(2))
            }
            (None, None) => {
                let mut fds = [
                    PollFd::new(&self.control, PollFlags::IN),
                    PollFd::new(&self.listener, PollFlags::IN),
                ];
                poll_ready(&mut fds)?;
                ready_event(&fds, None, None)
            }
        }
    }

    fn serve_worker(&mut self) -> Result<(), &'static str> {
        self.verify_ownership()?;
        if !self.lifecycle.ready() {
            return Err("service not ready");
        }
        let mut token = [0; 1];
        let read = {
            let inbox = self.worker.as_mut().ok_or("worker unavailable")?;
            inbox
                .wake
                .read(&mut token)
                .map_err(|_| "worker wake failed")?
        };
        if read == 0 {
            self.worker = None;
            return Ok(());
        }
        if token != [1] {
            return Err("invalid worker wake");
        }
        let call = {
            let inbox = self.worker.as_mut().ok_or("worker unavailable")?;
            inbox
                .requests
                .try_recv()
                .map_err(|_| "worker request missing")?
        };
        self.lifecycle
            .admit()
            .map_err(|_| "worker admission refused")?;
        increment(
            &mut self.signals.worker_requests,
            &mut self.signals.saturated,
        );
        let started = std::time::Instant::now();
        let outcome = self.service.dispatch_worker(call.request);
        if outcome.as_ref().is_ok_and(|reply| reply.is_err()) {
            increment(
                &mut self.signals.worker_refused,
                &mut self.signals.saturated,
            );
        }
        let owner_fenced = outcome.is_err();
        let published = outcome.map_err(|_| trusted_worker::WorkerTransportErrorV1::OwnerFenced);
        if call.reply.send(published).is_err() {
            increment(
                &mut self.signals.worker_delivery_failed,
                &mut self.signals.saturated,
            );
        }
        let mut result = if owner_fenced {
            Err("persistence fenced; worker outcome unknown")
        } else {
            Ok(())
        };
        if self.lifecycle.finish().is_err() {
            result = Err("worker obligation discharge failed");
        }
        add_time(
            &mut self.signals.worker_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        self.verify_ownership()?;
        result
    }

    fn serve_control(&mut self) -> Result<(), &'static str> {
        self.verify_ownership()?;
        let (mut stream, _) = self.control.accept().map_err(|_| "control accept failed")?;
        let started = std::time::Instant::now();
        increment(
            &mut self.signals.control_requests,
            &mut self.signals.saturated,
        );
        let result = self.handle_control(&mut stream);
        add_time(
            &mut self.signals.control_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        result
    }

    fn handle_control(&mut self, stream: &mut UnixStream) -> Result<(), &'static str> {
        if stream.set_read_timeout(Some(IO_TIMEOUT)).is_err()
            || stream.set_write_timeout(Some(IO_TIMEOUT)).is_err()
        {
            increment(
                &mut self.signals.control_refused,
                &mut self.signals.saturated,
            );
            return Ok(());
        }
        // EOF is mandatory; a valid prefix plus a second command is not accepted.
        let mut bytes = [0; 5];
        let mut length = 0;
        let command = loop {
            match stream.read(&mut bytes[length..]) {
                Ok(0) => break CommandV1::decode(&bytes[..length]).ok(),
                Ok(count) => {
                    length += count;
                    if length == bytes.len() {
                        break None;
                    }
                }
                Err(_) => break None,
            }
        };
        let Some(command) = command else {
            increment(
                &mut self.signals.control_refused,
                &mut self.signals.saturated,
            );
            return Ok(());
        };
        self.verify_ownership()?;
        let mut status = self.status_snapshot();
        match command {
            CommandV1::Status => {}
            CommandV1::Drain => {
                self.lifecycle.drain().map_err(|_| "drain refused")?;
                increment(
                    &mut self.signals.drain_requests,
                    &mut self.signals.saturated,
                );
                status = self.status_snapshot();
            }
            CommandV1::Maintain => {
                let report = self
                    .service
                    .maintain()
                    .map_err(|_| "maintenance fenced; inspect storage")?;
                status.maintenance = MaintenanceV1::Inspected;
                status.entries = report.entries;
                status.committed = report.committed;
                status.temporary = report.temporary;
                status.other = report.other;
            }
        }
        self.verify_ownership()?;
        let response = status.encode().map_err(|_| "invalid operational status")?;
        if stream.write_all(&response).is_err() {
            increment(
                &mut self.signals.control_delivery_failed,
                &mut self.signals.saturated,
            );
        }
        Ok(())
    }
}
