//! Bounded in-process access to Chronos's native worker profile.
//!
//! The handle carries native authority inside one process. It is deliberately
//! not a socket, codec, credential, or cloneable bearer. The server remains the
//! sole durable owner and serializes each call with ordinary coordination.

use chronos_host::worker::{WorkerClientV1, WorkerReplyV1, WorkerRequestV1};
use std::{
    io::Write,
    os::unix::net::UnixStream,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
};

pub(crate) struct WorkerCall {
    pub(crate) request: WorkerRequestV1,
    pub(crate) reply: SyncSender<Result<WorkerReplyV1, WorkerTransportErrorV1>>,
}

pub(crate) struct WorkerInbox {
    pub(crate) wake: UnixStream,
    pub(crate) requests: Receiver<WorkerCall>,
}

/// One non-cloneable authority-bearing handle to the owning Chronos service.
pub struct TrustedWorkerHandle {
    wake: UnixStream,
    requests: SyncSender<WorkerCall>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerTransportErrorV1 {
    OwnerUnavailable,
    OwnerFenced,
}

pub(crate) fn channel() -> Result<(TrustedWorkerHandle, WorkerInbox), std::io::Error> {
    let (client_wake, owner_wake) = UnixStream::pair()?;
    let (requests, receiver) = sync_channel(1);
    Ok((
        TrustedWorkerHandle {
            wake: client_wake,
            requests,
        },
        WorkerInbox {
            wake: owner_wake,
            requests: receiver,
        },
    ))
}

impl WorkerClientV1 for TrustedWorkerHandle {
    type TransportError = WorkerTransportErrorV1;

    fn call_worker(
        &mut self,
        request: WorkerRequestV1,
    ) -> Result<WorkerReplyV1, Self::TransportError> {
        let (reply, result) = sync_channel(0);
        self.requests
            .send(WorkerCall { request, reply })
            .map_err(|_| WorkerTransportErrorV1::OwnerUnavailable)?;
        self.wake
            .write_all(&[1])
            .map_err(|_| WorkerTransportErrorV1::OwnerUnavailable)?;
        result
            .recv()
            .map_err(|_| WorkerTransportErrorV1::OwnerUnavailable)?
    }
}
