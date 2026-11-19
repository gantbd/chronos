#![cfg(unix)]

use chronos_client::AlignmentWireClientV1;
use chronos_local::alignment::AlignmentLocalClient;
use chronos_wire::alignment_v1::{Command, Limits, Outcome, RefusalCode, Request, Response};
use std::{
    io::BufReader,
    os::unix::{fs::PermissionsExt, net::UnixListener},
};

fn limits() -> Limits {
    Limits {
        max_field_bytes: 64,
        max_evidence: 8,
        max_next_tasks: 4,
    }
}

#[test]
fn alignment_client_uses_the_verified_independent_owner_socket() {
    let directory = std::path::PathBuf::from("/tmp").join(format!(
        "chronos-alignment-client-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = chronos_local::alignment_socket_path(&directory);
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let frame = chronos_wire::read_frame(
            &mut BufReader::new(&stream),
            chronos_local::MAX_REQUEST_BYTES,
        )
        .unwrap()
        .unwrap();
        let request = chronos_wire::decode_alignment_request_v1(
            &frame,
            chronos_local::MAX_REQUEST_BYTES,
            limits(),
        )
        .unwrap();
        assert_eq!(request.chronos_scope(), "product-delivery");
        chronos_wire::write_frame(
            &mut stream,
            &Response {
                version: 1,
                outcome: Outcome::Refused {
                    code: RefusalCode::Unavailable,
                },
            },
            chronos_local::MAX_RESPONSE_BYTES,
        )
        .unwrap();
    });

    let mut client = AlignmentLocalClient::new(&directory, limits());
    let response = client
        .request_alignment_v1(Request {
            version: 1,
            command: Command::Observe {
                chronos_scope: "product-delivery".into(),
            },
        })
        .unwrap();
    assert!(matches!(
        response.outcome,
        Outcome::Refused {
            code: RefusalCode::Unavailable
        }
    ));
    server.join().unwrap();
    std::fs::remove_file(socket).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
