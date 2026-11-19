//! Explicit client for the independent read-only alignment endpoint.

use super::*;

pub struct AlignmentLocalClient {
    directory: PathBuf,
    limits: chronos_wire::alignment_v1::Limits,
}

impl AlignmentLocalClient {
    pub fn new(directory: impl Into<PathBuf>, limits: chronos_wire::alignment_v1::Limits) -> Self {
        Self {
            directory: directory.into(),
            limits,
        }
    }
}

impl chronos_client::AlignmentWireClientV1 for AlignmentLocalClient {
    type Error = CallError;

    fn request_alignment_v1(
        &mut self,
        request: chronos_wire::alignment_v1::Request,
    ) -> Result<chronos_wire::alignment_v1::Response, Self::Error> {
        request
            .validate(self.limits)
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_alignment_socket(&self.directory).map_err(|_| CallError::NotSent)?;
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
        chronos_wire::decode_alignment_response_v1(
            &frame,
            MAX_RESPONSE_BYTES,
            &request,
            self.limits,
        )
        .map_err(|_| CallError::InvalidResponse)
    }
}
