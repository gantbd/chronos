//! Private local listener for one bounded, read-only dual-frontier PM
//! observation per connection. Protocol and disclosure remain owner policy.

use super::{DualFrontierDisclosureV1, DualFrontierJournalV1, host::DualFrontierHostV1};
use crate::alignment::{
    host::AlignmentFrontierV1,
    local_endpoint::{LocalReadEndpointProfileV1, LocalReadEndpointV1},
};
use std::{io::BufReader, os::unix::net::UnixStream, path::Path, time::Duration};

pub const IO_TIMEOUT: Duration = Duration::from_secs(2);
pub use crate::alignment::local_endpoint::LocalReadEndpointSignalsV1 as DualFrontierListenerSignalsV1;

const PROFILE: LocalReadEndpointProfileV1 = LocalReadEndpointProfileV1 {
    socket_name: "chronos.dual-frontier.sock",
    lock_name: "dual-frontier-owner.lock",
};

pub struct DualFrontierListenerV1 {
    endpoint: LocalReadEndpointV1,
}

impl DualFrontierListenerV1 {
    pub fn bind(directory: impl AsRef<Path>) -> Result<Self, &'static str> {
        LocalReadEndpointV1::bind(directory, PROFILE).map(|endpoint| Self { endpoint })
    }

    pub fn signals(&self) -> DualFrontierListenerSignalsV1 {
        self.endpoint.signals()
    }

    pub fn serve_one<J, F, D>(
        &mut self,
        host: &mut DualFrontierHostV1<J, F, D>,
        max_request_bytes: usize,
        max_response_bytes: usize,
        limits: chronos_protocol::dual_frontier_v1::Limits,
    ) -> Result<(), &'static str>
    where
        J: DualFrontierJournalV1,
        F: AlignmentFrontierV1,
        D: DualFrontierDisclosureV1,
    {
        map_endpoint_error(self.endpoint.serve_one(&mut |stream| {
            serve_connection(host, stream, max_request_bytes, max_response_bytes, limits)
        }))
    }

    pub fn run_with_shutdown<J, F, D>(
        &mut self,
        host: &mut DualFrontierHostV1<J, F, D>,
        wake: &UnixStream,
        max_request_bytes: usize,
        max_response_bytes: usize,
        limits: chronos_protocol::dual_frontier_v1::Limits,
    ) -> Result<(), &'static str>
    where
        J: DualFrontierJournalV1,
        F: AlignmentFrontierV1,
        D: DualFrontierDisclosureV1,
    {
        map_endpoint_error(self.endpoint.run_with_shutdown(wake, &mut |stream| {
            serve_connection(host, stream, max_request_bytes, max_response_bytes, limits)
        }))
    }
}

pub fn serve_connection<J, F, D>(
    host: &mut DualFrontierHostV1<J, F, D>,
    mut stream: UnixStream,
    max_request_bytes: usize,
    max_response_bytes: usize,
    limits: chronos_protocol::dual_frontier_v1::Limits,
) -> Result<(), &'static str>
where
    J: DualFrontierJournalV1,
    F: AlignmentFrontierV1,
    D: DualFrontierDisclosureV1,
{
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|_| "dual-frontier connection timeout setup failed")?;
    let frame = match chronos_wire::read_frame(&mut BufReader::new(&stream), max_request_bytes) {
        Ok(Some(frame)) => frame,
        Ok(None) => return Ok(()),
        Err(error) => {
            let response = chronos_wire::dual_frontier_refusal_v1(match error {
                chronos_wire::FrameError::Limit => chronos_protocol::CodecError::Limit,
                chronos_wire::FrameError::Capacity => chronos_protocol::CodecError::Capacity,
                chronos_wire::FrameError::Io(_) | chronos_wire::FrameError::Truncated => {
                    chronos_protocol::CodecError::Malformed
                }
            });
            return chronos_wire::write_frame(&mut stream, &response, max_response_bytes)
                .map_err(|_| "dual-frontier refusal delivery failed");
        }
    };
    let response = chronos_wire::decode_dual_frontier_request_v1(&frame, max_request_bytes, limits)
        .map_or_else(chronos_wire::dual_frontier_refusal_v1, |request| {
            host.dispatch(request)
        });
    chronos_wire::write_frame(&mut stream, &response, max_response_bytes)
        .map_err(|_| "dual-frontier response delivery failed")
}

fn map_endpoint_error(result: Result<(), &'static str>) -> Result<(), &'static str> {
    result.map_err(|error| match error {
        "read endpoint ownership changed" => "dual-frontier endpoint ownership changed",
        other => other,
    })
}
