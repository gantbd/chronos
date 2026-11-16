use super::*;
use chronos_wire::control::{CommandV1, LifecycleV1, MaintenanceV1, StatusV1};
use rustix::event::{PollFd, PollFlags, poll};
use std::{
    io::{Read, Write},
    os::fd::AsFd,
};

enum Event {
    Data,
    Control,
    Signal,
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
        // Without a signal descriptor the third slot aliases data and is ignored.
        let third = wake
            .map(AsFd::as_fd)
            .unwrap_or_else(|| self.listener.as_fd());
        let mut fds = [
            PollFd::new(&self.control, PollFlags::IN),
            PollFd::new(&self.listener, PollFlags::IN),
            PollFd::from_borrowed_fd(third, PollFlags::IN),
        ];
        loop {
            match poll(&mut fds, None) {
                Ok(_) => break,
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => return Err("service readiness wait failed"),
            }
        }
        if fds
            .iter()
            .any(|fd| fd.revents().intersects(PollFlags::ERR | PollFlags::NVAL))
        {
            return Err("service descriptor failed");
        }
        if wake.is_some() {
            if fds[2].revents().contains(PollFlags::IN) {
                return Ok(Event::Signal);
            }
            if fds[2].revents().contains(PollFlags::HUP) {
                return Err("signal channel lost");
            }
        }
        // Operational control wins ties: no new data request is admitted after
        // an already observable drain request.
        if fds[0].revents().contains(PollFlags::IN) {
            return Ok(Event::Control);
        }
        if fds[1].revents().contains(PollFlags::IN) {
            return Ok(Event::Data);
        }
        Err("unexpected service readiness event")
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
