//! Bounded JSON codec and newline framing for the closed coordination protocol.
#![forbid(unsafe_code)]
pub use chronos_protocol::*;
use serde::Serialize;
use std::io::Write;

pub fn decode_alignment_request_v1(
    bytes: &[u8],
    max_bytes: usize,
    limits: alignment_v1::Limits,
) -> Result<alignment_v1::Request, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let request: alignment_v1::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_alignment_response_v1(
    bytes: &[u8],
    max_bytes: usize,
    request: &alignment_v1::Request,
    limits: alignment_v1::Limits,
) -> Result<alignment_v1::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: alignment_v1::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate_for(request, limits)?;
    Ok(response)
}

pub fn alignment_refusal_v1(error: CodecError) -> alignment_v1::Response {
    alignment_v1::Response {
        version: 1,
        outcome: alignment_v1::Outcome::Refused {
            code: match error {
                CodecError::Limit => alignment_v1::RefusalCode::Limit,
                CodecError::Capacity => alignment_v1::RefusalCode::Capacity,
                CodecError::Version => alignment_v1::RefusalCode::UnsupportedVersion,
                CodecError::Malformed => alignment_v1::RefusalCode::InvalidRequest,
            },
        },
    }
}

pub fn decode_dual_frontier_request_v1(
    bytes: &[u8],
    max_bytes: usize,
    limits: dual_frontier_v1::Limits,
) -> Result<dual_frontier_v1::Request, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let request: dual_frontier_v1::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_dual_frontier_response_v1(
    bytes: &[u8],
    max_bytes: usize,
    request: &dual_frontier_v1::Request,
    limits: dual_frontier_v1::Limits,
) -> Result<dual_frontier_v1::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: dual_frontier_v1::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate_for(request, limits)?;
    Ok(response)
}

pub fn dual_frontier_refusal_v1(error: CodecError) -> dual_frontier_v1::Response {
    dual_frontier_v1::Response {
        version: 1,
        outcome: dual_frontier_v1::Outcome::Refused {
            code: match error {
                CodecError::Limit => dual_frontier_v1::RefusalCode::Limit,
                CodecError::Capacity => dual_frontier_v1::RefusalCode::Capacity,
                CodecError::Version => dual_frontier_v1::RefusalCode::UnsupportedVersion,
                CodecError::Malformed => dual_frontier_v1::RefusalCode::InvalidRequest,
            },
        },
    }
}

pub fn decode_request_v2(
    bytes: &[u8],
    limits: Limits,
) -> Result<coordination_v2::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: coordination_v2::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}
pub fn decode_response_v2(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<coordination_v2::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: coordination_v2::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate(max_bytes)?;
    Ok(response)
}

/// Strict version dispatch without falling back from a malformed profile.
pub enum ProfileRequest {
    ReviewV10(review_v10::Request),
    WorkflowV9(workflow_v9::Request),
    ScheduledCancellationV8(coordination_v8::Request),
    ScheduledAdmissionV7(coordination_v7::Request),
    DependenciesV6(dependencies_v6::Request),
    SchedulingV5(scheduling_v5::Request),
    V4(coordination_v4::Request),
    V1(Request),
    V2(coordination_v2::Request),
    V3(coordination_v3::Request),
}
#[derive(Serialize)]
#[serde(untagged)]
pub enum ProfileResponse {
    ReviewV10(review_v10::Response),
    WorkflowV9(workflow_v9::Response),
    ScheduledCancellationV8(coordination_v8::Response),
    ScheduledAdmissionV7(coordination_v7::Response),
    DependenciesV6(dependencies_v6::Response),
    SchedulingV5(scheduling_v5::Response),
    V4(coordination_v4::Response),
    V1(Response),
    V2(coordination_v2::Response),
    V3(coordination_v3::Response),
}
pub fn decode_profile(bytes: &[u8], limits: Limits) -> Result<ProfileRequest, CodecError> {
    match request_version(bytes, limits.max_frame_bytes)? {
        10 => decode_request_v10(bytes, limits).map(ProfileRequest::ReviewV10),
        9 => decode_request_v9(bytes, limits).map(ProfileRequest::WorkflowV9),
        8 => decode_request_v8(bytes, limits).map(ProfileRequest::ScheduledCancellationV8),
        7 => decode_request_v7(bytes, limits).map(ProfileRequest::ScheduledAdmissionV7),
        6 => {
            let request: dependencies_v6::Request =
                serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
            request.validate(limits)?;
            Ok(ProfileRequest::DependenciesV6(request))
        }
        5 => decode_scheduling_request_v5(bytes, limits).map(ProfileRequest::SchedulingV5),
        4 => decode_request_v4(bytes, limits).map(ProfileRequest::V4),
        1 => decode_request(bytes, limits).map(ProfileRequest::V1),
        2 => decode_request_v2(bytes, limits).map(ProfileRequest::V2),
        3 => decode_request_v3(bytes, limits).map(ProfileRequest::V3),
        _ => Err(CodecError::Version),
    }
}

/// Select only the error representation, never an executable command. A
/// syntactically unambiguous V2 envelope receives V2 refusal syntax even when
/// its command is invalid. Unknown/ambiguous envelopes retain the legacy V1
/// generic refusal. No input content is reflected in the response.
pub fn profile_refusal(bytes: &[u8], limits: Limits, error: CodecError) -> ProfileResponse {
    if request_version(bytes, limits.max_frame_bytes) == Ok(10) {
        return ProfileResponse::ReviewV10(review_v10::Response {
            version: 10,
            outcome: review_v10::Outcome::Refused {
                code: match error {
                    CodecError::Limit => review_v10::RefusalCode::Limit,
                    CodecError::Capacity => review_v10::RefusalCode::Capacity,
                    CodecError::Version => review_v10::RefusalCode::UnsupportedVersion,
                    CodecError::Malformed => review_v10::RefusalCode::InvalidRequest,
                },
            },
        });
    }
    if request_version(bytes, limits.max_frame_bytes) == Ok(9) {
        return ProfileResponse::WorkflowV9(workflow_v9::Response {
            version: 9,
            outcome: workflow_v9::Outcome::Refused {
                code: match error {
                    CodecError::Limit => workflow_v9::RefusalCode::Limit,
                    CodecError::Capacity => workflow_v9::RefusalCode::Capacity,
                    CodecError::Version => workflow_v9::RefusalCode::UnsupportedVersion,
                    CodecError::Malformed => workflow_v9::RefusalCode::InvalidRequest,
                },
            },
        });
    }
    if request_version(bytes, limits.max_frame_bytes) == Ok(8) {
        return ProfileResponse::ScheduledCancellationV8(coordination_v8::Response {
            version: 8,
            outcome: coordination_v8::Outcome::Refused {
                code: match error {
                    CodecError::Limit => coordination_v8::RefusalCode::Limit,
                    CodecError::Capacity => coordination_v8::RefusalCode::Capacity,
                    CodecError::Version => coordination_v8::RefusalCode::UnsupportedVersion,
                    CodecError::Malformed => coordination_v8::RefusalCode::InvalidRequest,
                },
            },
        });
    }
    if request_version(bytes, limits.max_frame_bytes) == Ok(7) {
        return ProfileResponse::ScheduledAdmissionV7(coordination_v7::Response {
            version: 7,
            outcome: coordination_v7::Outcome::Refused {
                code: match error {
                    CodecError::Limit => coordination_v7::RefusalCode::Limit,
                    CodecError::Capacity => coordination_v7::RefusalCode::Capacity,
                    CodecError::Version => coordination_v7::RefusalCode::UnsupportedVersion,
                    CodecError::Malformed => coordination_v7::RefusalCode::InvalidRequest,
                },
            },
        });
    }
    if request_version(bytes, limits.max_frame_bytes) == Ok(6) {
        return ProfileResponse::DependenciesV6(dependencies_v6::Response {
            version: 6,
            outcome: dependencies_v6::Outcome::Refused {
                code: if error == CodecError::Limit {
                    dependencies_v6::Refusal::Limit
                } else {
                    dependencies_v6::Refusal::InvalidRequest
                },
            },
        });
    }
    if request_version(bytes, limits.max_frame_bytes) == Ok(5) {
        return ProfileResponse::SchedulingV5(scheduling_v5::Response {
            version: 5,
            outcome: scheduling_v5::Outcome::Refused {
                code: match error {
                    CodecError::Limit => scheduling_v5::RefusalCode::Limit,
                    CodecError::Capacity => scheduling_v5::RefusalCode::Capacity,
                    CodecError::Version | CodecError::Malformed => {
                        scheduling_v5::RefusalCode::InvalidRequest
                    }
                },
            },
        });
    }
    if request_version(bytes, limits.max_frame_bytes) == Ok(4) {
        return ProfileResponse::V4(coordination_v4::Response {
            version: 4,
            outcome: coordination_v4::Outcome::Refused {
                code: match error {
                    CodecError::Limit => coordination_v4::RefusalCode::Limit,
                    CodecError::Capacity => coordination_v4::RefusalCode::Capacity,
                    CodecError::Version => coordination_v4::RefusalCode::UnsupportedVersion,
                    CodecError::Malformed => coordination_v4::RefusalCode::InvalidRequest,
                },
            },
        });
    }
    let failure = Failure::Refused {
        code: match error {
            CodecError::Limit => FailureCode::Limit,
            CodecError::Capacity => FailureCode::Capacity,
            CodecError::Version => FailureCode::UnsupportedVersion,
            CodecError::Malformed => FailureCode::InvalidRequest,
        },
    };
    if request_version(bytes, limits.max_frame_bytes) == Ok(3) {
        ProfileResponse::V3(coordination_v3::Response {
            version: 3,
            outcome: coordination_v3::Outcome::Refused {
                code: match error {
                    CodecError::Limit => coordination_v3::RefusalCode::Limit,
                    CodecError::Capacity => coordination_v3::RefusalCode::Capacity,
                    CodecError::Version => coordination_v3::RefusalCode::UnsupportedVersion,
                    CodecError::Malformed => coordination_v3::RefusalCode::InvalidRequest,
                },
            },
        })
    } else if request_version(bytes, limits.max_frame_bytes) == Ok(2) {
        ProfileResponse::V2(coordination_v2::Response {
            version: 2,
            outcome: coordination_v2::Outcome::Refused {
                failure: coordination_v2::RetirementFailure::Common { failure },
            },
        })
    } else {
        ProfileResponse::V1(Response {
            version: 1,
            outcome: Outcome::Refused { failure },
        })
    }
}

pub fn decode_request_v3(
    bytes: &[u8],
    limits: Limits,
) -> Result<coordination_v3::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: coordination_v3::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_request_v7(
    bytes: &[u8],
    limits: Limits,
) -> Result<coordination_v7::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: coordination_v7::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_request_v8(
    bytes: &[u8],
    limits: Limits,
) -> Result<coordination_v8::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: coordination_v8::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_request_v9(bytes: &[u8], limits: Limits) -> Result<workflow_v9::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: workflow_v9::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_request_v10(bytes: &[u8], limits: Limits) -> Result<review_v10::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: review_v10::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_response_v10(
    bytes: &[u8],
    max_bytes: usize,
    request: &review_v10::Request,
    limits: Limits,
) -> Result<review_v10::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: review_v10::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate_for(request, limits)?;
    Ok(response)
}

pub fn decode_response_v9(
    bytes: &[u8],
    max_bytes: usize,
    limits: Limits,
) -> Result<workflow_v9::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: workflow_v9::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate(limits)?;
    Ok(response)
}

pub fn decode_response_v8(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<coordination_v8::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: coordination_v8::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate()?;
    Ok(response)
}

pub fn decode_response_v7(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<coordination_v7::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: coordination_v7::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate()?;
    Ok(response)
}
pub fn decode_response_v3(
    bytes: &[u8],
    max_bytes: usize,
    limits: Limits,
) -> Result<coordination_v3::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: coordination_v3::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate(limits)?;
    Ok(response)
}

fn request_version(bytes: &[u8], max_bytes: usize) -> Result<u16, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    #[derive(serde::Deserialize)]
    struct Version {
        version: u16,
    }
    let version: Version = serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    Ok(version.version)
}

/// Frame ceiling is checked before JSON allocation. Field/count bounds are
/// checked before engine dispatch. Parser memory is bounded by the frame, not
/// by an allocator/RSS guarantee; standard allocation aborts are not recoverable.
pub fn decode_request(bytes: &[u8], limits: Limits) -> Result<Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: Request = serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}

pub fn decode_request_v4(
    bytes: &[u8],
    limits: Limits,
) -> Result<coordination_v4::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: coordination_v4::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}
pub fn decode_scheduling_request_v5(
    bytes: &[u8],
    limits: Limits,
) -> Result<scheduling_v5::Request, CodecError> {
    if bytes.len() > limits.max_frame_bytes {
        return Err(CodecError::Limit);
    }
    let request: scheduling_v5::Request =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    request.validate(limits)?;
    Ok(request)
}
pub fn decode_scheduling_response_v5(
    bytes: &[u8],
    max_bytes: usize,
    limits: Limits,
) -> Result<scheduling_v5::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: scheduling_v5::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate(limits)?;
    Ok(response)
}
pub fn decode_response_v4(
    bytes: &[u8],
    max_bytes: usize,
    limits: Limits,
) -> Result<coordination_v4::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: coordination_v4::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate(limits)?;
    Ok(response)
}
struct BoundedWriter {
    bytes: Vec<u8>,
    max: usize,
    failure: Option<CodecError>,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let error = if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            Some(CodecError::Limit)
        } else if self.bytes.try_reserve(bytes.len()).is_err() {
            Some(CodecError::Capacity)
        } else {
            None
        };
        if let Some(error) = error {
            self.failure = Some(error);
            return Err(std::io::Error::other("bounded output refused"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn encode<T: Serialize>(value: &T, max_bytes: usize) -> Result<Vec<u8>, CodecError> {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        max: max_bytes,
        failure: None,
    };
    if serde_json::to_writer(&mut writer, value).is_err() {
        return Err(writer.failure.unwrap_or(CodecError::Malformed));
    }
    Ok(writer.bytes)
}

pub fn decode_response(bytes: &[u8], max_bytes: usize) -> Result<Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: Response = serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    if response.version != 1 {
        return Err(CodecError::Version);
    }
    response.validate(max_bytes)?;
    Ok(response)
}

#[derive(Debug)]
pub enum FrameError {
    Io(std::io::ErrorKind),
    Limit,
    Truncated,
    Capacity,
}

/// Newline-delimited UTF-8 JSON. Oversized/partial frames are fatal to the
/// connection; never drain arbitrary attacker-controlled input or auto-retry.
pub fn read_frame(
    reader: &mut impl std::io::BufRead,
    max: usize,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut frame = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| FrameError::Io(error.kind()))?;
        if available.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(FrameError::Truncated)
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.unwrap_or(available.len());
        if count > max.saturating_sub(frame.len()) {
            return Err(FrameError::Limit);
        }
        frame.try_reserve(count).map_err(|_| FrameError::Capacity)?;
        frame.extend_from_slice(&available[..count]);
        reader.consume(count + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(frame));
        }
    }
}

/// Encodes fully within the ceiling before writing any response bytes. I/O
/// failure may still mean partial delivery; it never authorizes command replay.
pub fn write_frame(
    writer: &mut impl Write,
    value: &impl Serialize,
    max: usize,
) -> Result<(), FrameError> {
    let bytes = encode(value, max).map_err(|error| match error {
        CodecError::Capacity => FrameError::Capacity,
        _ => FrameError::Limit,
    })?;
    writer
        .write_all(&bytes)
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .map_err(|error| FrameError::Io(error.kind()))
}

pub fn decode_dependency_response_v6(
    bytes: &[u8],
    max_bytes: usize,
    request: &dependencies_v6::Request,
    limits: Limits,
) -> Result<dependencies_v6::Response, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::Limit);
    }
    let response: dependencies_v6::Response =
        serde_json::from_slice(bytes).map_err(|_| CodecError::Malformed)?;
    response.validate_for(request, limits)?;
    Ok(response)
}
