//! Source-profile checks, not publication authorization or release certification.
use serde_json::Value;
use std::{path::Path, process::Command};

fn metadata() -> Value {
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
        .unwrap();
    assert!(output.status.success(), "profile metadata unavailable");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn validate(metadata: &Value) -> Result<(), &'static str> {
    let root = Path::new(
        metadata["workspace_root"]
            .as_str()
            .ok_or("workspace root")?,
    )
    .canonicalize()
    .map_err(|_| "workspace root")?;
    let packages = metadata["packages"].as_array().ok_or("packages")?;
    if packages.len() != 14 {
        return Err("package count");
    }
    for package in packages {
        let engine = package["name"] == "chronos-engine";
        if package["version"] != if engine { "0.3.0" } else { "0.1.0" } {
            return Err("profile version");
        }
        if package["rust_version"] != "1.95" {
            return Err("MSRV");
        }
        if package["license"] != "MIT OR Apache-2.0" {
            return Err("license declaration");
        }
        // Preserve the current distinction. Cargo registry eligibility is NOT
        // approval to publish; changing it requires an explicit policy review.
        if (engine && !package["publish"].is_null())
            || (!engine && package["publish"] != serde_json::json!([]))
        {
            return Err("publication setting");
        }
        for dependency in package["dependencies"].as_array().ok_or("dependencies")? {
            if let Some(path) = dependency["path"].as_str()
                && !Path::new(path)
                    .canonicalize()
                    .is_ok_and(|path| path.starts_with(&root))
            {
                return Err("outward path dependency");
            }
        }
    }
    Ok(())
}

#[test]
fn current_v03_source_profile_has_explicit_versions_msrv_policy_and_local_closure() {
    assert_eq!(validate(&metadata()), Ok(()));
}

#[test]
fn vendored_keystone_mechanism_is_an_exact_recorded_source_snapshot_not_a_claimed_commit_release() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let origin: Value = serde_json::from_str(include_str!(
        "../../../third_party/keystone-storage-0.0.1/UPSTREAM.json"
    ))
    .unwrap();
    assert_eq!(origin["working_tree_changes_included"], true);
    assert_eq!(origin["not_an_immutable_upstream_release"], true);
    let digest = Command::new("shasum")
        .args(["-a", "256", "third_party/keystone-storage-0.0.1/src/lib.rs"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(digest.status.success());
    assert_eq!(
        String::from_utf8(digest.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap(),
        origin["source_lib_sha256"].as_str().unwrap()
    );
    let manifest_digest = Command::new("shasum")
        .args(["-a", "256", "third_party/keystone-storage-0.0.1/Cargo.toml"])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap();
    assert!(manifest_digest.status.success());
    assert_eq!(
        String::from_utf8(manifest_digest.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap(),
        origin["manifest_sha256"].as_str().unwrap()
    );
}

#[test]
fn changed_release_inputs_are_not_silently_credited_to_the_current_profile() {
    let baseline = metadata();
    let engine = baseline["packages"]
        .as_array()
        .unwrap()
        .iter()
        .position(|package| package["name"] == "chronos-engine")
        .unwrap();
    let nonengine = baseline["packages"]
        .as_array()
        .unwrap()
        .iter()
        .position(|package| package["name"] != "chronos-engine")
        .unwrap();
    for (index, field, value, expected) in [
        (
            engine,
            "version",
            serde_json::json!("0.4.0"),
            "profile version",
        ),
        (nonengine, "rust_version", serde_json::json!("1.96"), "MSRV"),
        (
            nonengine,
            "license",
            serde_json::json!("UNLICENSED"),
            "license declaration",
        ),
        (nonengine, "publish", Value::Null, "publication setting"),
        (
            engine,
            "publish",
            serde_json::json!([]),
            "publication setting",
        ),
    ] {
        let mut changed = baseline.clone();
        changed["packages"][index][field] = value;
        assert_eq!(validate(&changed), Err(expected));
    }
    let mut changed = baseline.clone();
    changed["packages"][engine]["dependencies"] = serde_json::json!([{"path": "/outside-profile"}]);
    assert_eq!(validate(&changed), Err("outward path dependency"));
    let mut changed = baseline.clone();
    changed["packages"][engine]["dependencies"] = serde_json::json!([{
        "path": Path::new(baseline["workspace_root"].as_str().unwrap()).join("..")
    }]);
    assert_eq!(validate(&changed), Err("outward path dependency"));
    let mut changed = baseline;
    changed["packages"].as_array_mut().unwrap().pop();
    assert_eq!(validate(&changed), Err("package count"));
}

#[test]
fn local_profile_gate_is_separate_from_ui_and_optional_external_tools() {
    let makefile = include_str!("../../../Makefile");
    let profile = makefile
        .lines()
        .find(|line| line.starts_with("check-local-profile:"));
    assert_eq!(
        profile,
        Some(
            "check-local-profile: check package portability mutation-smoke qualify-local-runtime qualify-local-performance"
        )
    );
}
