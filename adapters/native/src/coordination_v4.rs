//! Original retained references only; never a current-name reconstruction.
use crate::{EmbeddedClient, EmbeddedError, IntoWire};
use chronos_host::{
    native_allocation::{AllocationClientV1, AllocationReplyV1, AllocationResultV1},
    native_v3::{InvocationRefusalV1, InvocationV1},
};
use chronos_wire::{CodecError, Limits, coordination_v4 as wire};
pub fn invocation(request: wire::Request, limits: Limits) -> Result<InvocationV1, CodecError> {
    request.validate(limits)?;
    let wire::Command::LookupAllocation { serial, key } = request.command;
    Ok(InvocationV1 {
        serial: super::decimal(&serial)?,
        key,
    })
}
pub fn response(serial: &str, reply: AllocationReplyV1, limits: Limits) -> wire::Response {
    if matches!(&reply, Ok(AllocationResultV1::Created(receipt)) if receipt.tasks.len() > limits.max_graph_tasks)
    {
        return wire::Response {
            version: 4,
            outcome: wire::Outcome::Refused {
                code: wire::RefusalCode::Limit,
            },
        };
    }
    let outcome = match reply {
        Ok(AllocationResultV1::Created(receipt)) => wire::Outcome::Allocation {
            serial: serial.into(),
            revision: receipt.revision.into_wire(),
            tasks: receipt.tasks.into_iter().map(IntoWire::into_wire).collect(),
        },
        Ok(AllocationResultV1::Unavailable) => wire::Outcome::Unavailable {
            serial: serial.into(),
        },
        Err(code) => wire::Outcome::Refused {
            code: match code {
                InvocationRefusalV1::Unauthorized => wire::RefusalCode::Unauthorized,
                InvocationRefusalV1::Limit => wire::RefusalCode::Limit,
                InvocationRefusalV1::Invalid => wire::RefusalCode::InvalidRequest,
                InvocationRefusalV1::Conflict => wire::RefusalCode::Conflict,
                InvocationRefusalV1::Expired => wire::RefusalCode::Expired,
                InvocationRefusalV1::OutOfSequence => wire::RefusalCode::OutOfSequence,
                InvocationRefusalV1::Exhausted => wire::RefusalCode::Exhausted,
            },
        },
    };
    let response = wire::Response {
        version: 4,
        outcome,
    };
    if response.validate(limits).is_err()
        || chronos_wire::encode(&response, limits.max_frame_bytes).is_err()
    {
        return wire::Response {
            version: 4,
            outcome: wire::Outcome::Refused {
                code: wire::RefusalCode::Limit,
            },
        };
    }
    response
}
impl<C: AllocationClientV1> chronos_client::WireClientV4 for EmbeddedClient<C> {
    type Error = EmbeddedError<C::TransportError>;
    fn request_v4(&mut self, request: wire::Request) -> Result<wire::Response, Self::Error> {
        chronos_wire::encode(&request, self.limits.max_frame_bytes)
            .map_err(EmbeddedError::Request)?;
        let native = invocation(request.clone(), self.limits).map_err(EmbeddedError::Request)?;
        let serial = native.serial.to_string();
        let reply = self.host.allocation(native).map_err(EmbeddedError::Host)?;
        let response = response(&serial, reply, self.limits);
        response
            .validate_for(&request, self.limits)
            .map_err(EmbeddedError::Request)?;
        Ok(response)
    }
}
