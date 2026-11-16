//! The distributable client must not pull the authority-owning engine or host.
use std::process::Command;

#[test]
fn remote_client_has_no_engine_host_service_or_adapter_dependency() {
    assert_engine_free("chronos-client");
}

#[test]
fn protocol_codec_and_projection_have_no_authority_side_dependencies() {
    for package in [
        "chronos-protocol",
        "chronos-wire",
        "chronos-model",
        "chronos-sdk-client",
    ] {
        assert_engine_free(package);
    }
}

#[test]
fn sdk_facades_are_reexports_and_embedded_sdk_selects_no_concrete_effects() {
    for source in [
        include_str!("../../../sdk/client/src/lib.rs"),
        include_str!("../../../sdk/embedded/src/lib.rs"),
    ] {
        for line in source
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            assert!(
                line.starts_with("//!")
                    || line == "#![forbid(unsafe_code)]"
                    || line.starts_with("pub use "),
                "facade adds behavior: {line}"
            );
        }
    }
    let tree = normal_tree("chronos-sdk-embedded");
    for package in [
        "chronos-adapter-checkpoint",
        "chronos-local",
        "chronos-standalone",
        "chronos-tui",
        "chronos-gui-macos",
        "chronos-mcp",
    ] {
        assert!(
            !tree
                .lines()
                .any(|line| line.split_whitespace().next() == Some(package)),
            "embedded SDK selects {package}"
        );
    }
}

fn assert_engine_free(root: &str) {
    let tree = normal_tree(root);
    for package in tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
    {
        assert!(
            !matches!(
                package,
                "chronos-engine" | "chronos-host" | "chronos-service"
            ) && !package.starts_with("chronos-adapter-"),
            "{root} pulls authority-side package {package}:\n{tree}"
        );
    }
}

fn normal_tree(root: &str) -> String {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "-p",
            root,
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--locked",
            "--offline",
        ])
        .output()
        .expect("inspect client dependency closure");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
