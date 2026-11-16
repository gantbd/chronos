//! Native enforcement of package placement and dependency direction (no Node).
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

#[test]
fn categorical_workspace_has_exact_owned_layers_and_no_upward_edges() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
        ])
        .output()
        .expect("inspect source-distributed packages");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let root = Path::new(metadata["workspace_root"].as_str().unwrap());
    let expected: BTreeMap<&str, (&str, &[&str])> = BTreeMap::from([
        ("chronos-engine", ("engine/chronos-engine", &[] as &[&str])),
        (
            "chronos-host",
            ("host/chronos-host", &["chronos-engine"] as &[&str]),
        ),
        (
            "chronos-model",
            (
                "host/chronos-model",
                &["chronos-client", "chronos-protocol"] as &[&str],
            ),
        ),
        (
            "chronos-protocol",
            ("protocol/chronos-protocol", &["serde"] as &[&str]),
        ),
        (
            "chronos-wire",
            (
                "protocol/chronos-wire",
                &["chronos-protocol", "serde", "serde_json"] as &[&str],
            ),
        ),
        (
            "chronos-client",
            ("client/chronos-client", &["chronos-protocol"] as &[&str]),
        ),
        (
            "chronos-service",
            (
                "service/chronos-service",
                &["chronos-engine", "chronos-host", "keystone-storage"] as &[&str],
            ),
        ),
        (
            "chronos-local",
            (
                "transport/chronos-local",
                &["chronos-client", "chronos-wire", "rustix"] as &[&str],
            ),
        ),
        (
            "chronos-adapter-checkpoint",
            (
                "adapters/checkpoint",
                &["chronos-engine", "chronos-host"] as &[&str],
            ),
        ),
        (
            "chronos-adapter-native",
            (
                "adapters/native",
                &[
                    "chronos-engine",
                    "chronos-host",
                    "chronos-client",
                    "chronos-wire",
                ] as &[&str],
            ),
        ),
        (
            "chronos-mcp",
            (
                "connectors/chronos-mcp",
                &[
                    "chronos-client",
                    "chronos-local",
                    "chronos-wire",
                    "serde",
                    "serde_json",
                ] as &[&str],
            ),
        ),
        (
            "chronos-standalone",
            (
                "applications/chronos-daemon",
                &[
                    "chronos-adapter-native",
                    "chronos-engine",
                    "chronos-host",
                    "chronos-service",
                    "chronos-wire",
                    "chronos-local",
                    "chronos-adapter-checkpoint",
                    "rustix",
                    "signal-hook",
                ] as &[&str],
            ),
        ),
        (
            "chronos-sdk-client",
            (
                "sdk/client",
                &[
                    "chronos-client",
                    "chronos-protocol",
                    "chronos-wire",
                    "chronos-local",
                ] as &[&str],
            ),
        ),
        (
            "chronos-sdk-embedded",
            (
                "sdk/embedded",
                &[
                    "chronos-engine",
                    "chronos-host",
                    "chronos-service",
                    "chronos-adapter-native",
                    "chronos-client",
                    "chronos-protocol",
                ] as &[&str],
            ),
        ),
    ]);
    let packages = metadata["packages"].as_array().unwrap();
    assert_eq!(
        metadata["workspace_members"].as_array().unwrap().len(),
        expected.len()
    );
    assert_eq!(packages.len(), expected.len());
    for package in packages {
        let name = package["name"].as_str().unwrap();
        let (directory, allowed) = expected
            .get(name)
            .expect("package requires an explicit architectural owner");
        assert_eq!(
            Path::new(package["manifest_path"].as_str().unwrap()),
            root.join(directory).join("Cargo.toml"),
            "incorrect category for {name}"
        );
        let normal: BTreeSet<_> = package["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|dependency| dependency["kind"].is_null())
            .map(|dependency| dependency["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            normal,
            allowed.iter().copied().collect(),
            "dependency direction/selection changed for {name}"
        );
        if name != "chronos-engine" {
            assert_eq!(
                package["publish"],
                serde_json::json!([]),
                "unpublished source-distribution baseline changed for {name}"
            );
        }
    }
}
