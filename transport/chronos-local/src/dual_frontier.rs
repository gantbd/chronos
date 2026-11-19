//! Explicit client for the independent read-only dual-frontier PM endpoint.

use super::*;

pub struct DualFrontierLocalClient {
    directory: PathBuf,
    limits: chronos_wire::dual_frontier_v1::Limits,
}

impl DualFrontierLocalClient {
    pub fn new(
        directory: impl Into<PathBuf>,
        limits: chronos_wire::dual_frontier_v1::Limits,
    ) -> Self {
        Self {
            directory: directory.into(),
            limits,
        }
    }
}

impl chronos_client::DualFrontierWireClientV1 for DualFrontierLocalClient {
    type Error = CallError;

    fn request_dual_frontier_v1(
        &mut self,
        request: chronos_wire::dual_frontier_v1::Request,
    ) -> Result<chronos_wire::dual_frontier_v1::Response, Self::Error> {
        request
            .validate(self.limits)
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_dual_frontier_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        chronos_wire::decode_dual_frontier_response_v1(
            &frame,
            MAX_RESPONSE_BYTES,
            &request,
            self.limits,
        )
        .map_err(|_| CallError::InvalidResponse)
    }
}
