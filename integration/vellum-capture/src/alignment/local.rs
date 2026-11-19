//! One bounded request per already-owned local Unix connection.
//!
//! The deployment owns listener creation, private-directory verification,
//! endpoint identity, accept-loop lifetime, and orderly shutdown.

use super::{
    AlignmentJournalV1,
    host::{AlignmentFrontierV1, AlignmentHostV1},
};
use std::{io::BufReader, os::unix::net::UnixStream, time::Duration};

pub const IO_TIMEOUT: Duration = Duration::from_secs(2);

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
