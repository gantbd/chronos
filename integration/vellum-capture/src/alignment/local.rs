//! Private local listener lifecycle and one bounded alignment request per
//! connection. Protocol semantics remain in the alignment host.

use super::{
    AlignmentJournalV1,
    host::{AlignmentFrontierV1, AlignmentHostV1},
    local_endpoint::{LocalReadEndpointProfileV1, LocalReadEndpointV1},
};
use std::{io::BufReader, os::unix::net::UnixStream, path::Path, time::Duration};

pub const IO_TIMEOUT: Duration = Duration::from_secs(2);
pub use super::local_endpoint::LocalReadEndpointSignalsV1 as AlignmentListenerSignalsV1;

const PROFILE: LocalReadEndpointProfileV1 = LocalReadEndpointProfileV1 {
    socket_name: "chronos.alignment.sock",
    lock_name: "alignment-owner.lock",
};

pub struct AlignmentListenerV1 {
    endpoint: LocalReadEndpointV1,
}

impl AlignmentListenerV1 {
    pub fn bind(directory: impl AsRef<Path>) -> Result<Self, &'static str> {
        LocalReadEndpointV1::bind(directory, PROFILE)
            .map(|endpoint| Self { endpoint })
            .map_err(map_alignment_endpoint_error)
    }

    pub fn signals(&self) -> AlignmentListenerSignalsV1 {
        self.endpoint.signals()
    }

    pub fn serve_one<J, F, D>(
        &mut self,
        host: &mut AlignmentHostV1<J, F, D>,
        max_request_bytes: usize,
        max_response_bytes: usize,
        limits: chronos_protocol::alignment_v1::Limits,
    ) -> Result<(), &'static str>
    where
        J: AlignmentJournalV1,
        F: AlignmentFrontierV1,
        D: super::AlignmentDisclosureV1,
    {
        map_endpoint_error(self.endpoint.serve_one(&mut |stream| {
            serve_connection(host, stream, max_request_bytes, max_response_bytes, limits)
        }))
    }

    pub fn run_with_shutdown<J, F, D>(
        &mut self,
        host: &mut AlignmentHostV1<J, F, D>,
        wake: &UnixStream,
        max_request_bytes: usize,
        max_response_bytes: usize,
        limits: chronos_protocol::alignment_v1::Limits,
    ) -> Result<(), &'static str>
    where
        J: AlignmentJournalV1,
        F: AlignmentFrontierV1,
        D: super::AlignmentDisclosureV1,
    {
        map_endpoint_error(self.endpoint.run_with_shutdown(wake, &mut |stream| {
            serve_connection(host, stream, max_request_bytes, max_response_bytes, limits)
        }))
    }
}

fn map_endpoint_error(result: Result<(), &'static str>) -> Result<(), &'static str> {
    result.map_err(map_alignment_endpoint_error)
}

fn map_alignment_endpoint_error(error: &'static str) -> &'static str {
    match error {
        "private read endpoint directory refused" => "private alignment directory refused",
        "read endpoint directory identity unavailable" => {
            "alignment directory identity unavailable"
        }
        "read endpoint directory canonicalization failed" => {
            "alignment directory canonicalization failed"
        }
        "read endpoint lock flags unsupported" => "alignment lock flags unsupported",
        "read endpoint owner lock refused" => "alignment owner lock refused",
        "read endpoint owner lock identity unavailable" => {
            "alignment owner lock identity unavailable"
        }
        "private read endpoint owner lock required" => "private alignment owner lock required",
        "read endpoint already owned" => "alignment endpoint already owned",
        "read endpoint observation failed" => "alignment endpoint observation failed",
        "private read endpoint required" => "private alignment endpoint required",
        "unattributed read endpoint refused" => "unattributed alignment endpoint refused",
        "stale read endpoint removal failed" => "stale alignment endpoint removal failed",
        "read endpoint bind failed" => "alignment endpoint bind failed",
        "read endpoint permissions failed" => "alignment endpoint permissions failed",
        "read endpoint identity unavailable" => "alignment endpoint identity unavailable",
        "read endpoint ownership refused" => "alignment endpoint ownership refused",
        "read endpoint accept failed" => "alignment accept failed",
        "read endpoint readiness wait failed" => "alignment readiness wait failed",
        "read endpoint descriptor failed" => "alignment descriptor failed",
        "read endpoint shutdown wake failed" => "alignment shutdown wake failed",
        "invalid read endpoint shutdown wake" => "invalid alignment shutdown wake",
        "read endpoint shutdown channel lost" => "alignment shutdown channel lost",
        "unexpected read endpoint readiness event" => "unexpected alignment readiness event",
        "read endpoint ownership changed" => "alignment endpoint ownership changed",
        other => other,
    }
}

pub fn serve_connection<J, F, D>(
    host: &mut AlignmentHostV1<J, F, D>,
    mut stream: UnixStream,
    max_request_bytes: usize,
    max_response_bytes: usize,
    limits: chronos_protocol::alignment_v1::Limits,
) -> Result<(), &'static str>
where
    J: AlignmentJournalV1,
    F: AlignmentFrontierV1,
    D: super::AlignmentDisclosureV1,
{
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|_| "alignment connection timeout setup failed")?;
    let frame = match chronos_wire::read_frame(&mut BufReader::new(&stream), max_request_bytes) {
        Ok(Some(frame)) => frame,
        Ok(None) => return Ok(()),
        Err(error) => {
            let response = chronos_wire::alignment_refusal_v1(match error {
                chronos_wire::FrameError::Limit => chronos_protocol::CodecError::Limit,
                chronos_wire::FrameError::Capacity => chronos_protocol::CodecError::Capacity,
                chronos_wire::FrameError::Io(_) | chronos_wire::FrameError::Truncated => {
                    chronos_protocol::CodecError::Malformed
                }
            });
            return chronos_wire::write_frame(&mut stream, &response, max_response_bytes)
                .map_err(|_| "alignment refusal delivery failed");
        }
    };
    let response =
        match chronos_wire::decode_alignment_request_v1(&frame, max_request_bytes, limits) {
            Ok(request) => host.dispatch(request),
            Err(error) => chronos_wire::alignment_refusal_v1(error),
        };
    chronos_wire::write_frame(&mut stream, &response, max_response_bytes)
        .map_err(|_| "alignment response delivery failed")
}
