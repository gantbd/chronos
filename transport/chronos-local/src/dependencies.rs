use super::*;
impl chronos_client::DependencyWireClientV6 for LocalClient {
    type Error = CallError;
    fn request_dependencies_v6(
        &mut self,
        request: chronos_wire::dependencies_v6::Request,
    ) -> Result<chronos_wire::dependencies_v6::Response, CallError> {
        request
            .validate(wire_limits())
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
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
        chronos_wire::decode_dependency_response_v6(
            &frame,
            MAX_RESPONSE_BYTES,
            &request,
            wire_limits(),
        )
        .map_err(|_| CallError::InvalidResponse)
    }
}
