//! Development-only source capture tool. Uses existing dev dependencies, not
//! client runtime dependencies. No publication, installs, Git writes or signing.
#![forbid(unsafe_code)]
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ROOT_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "Makefile",
    "README.md",
    "COMPONENT_ARCHITECTURE.md",
    "ROADMAP.production.md",
    "requirements.md",
    "specification.md",
    "model.md",
];
const ROOT_LICENSE_FILES: &[&str] = &["LICENSE-MIT", "LICENSE-APACHE", "NOTICE"];
const ROOT_DIRS: &[&str] = &[
    "third_party",
    "engine",
    "host",
    "protocol",
    "client",
    "service",
    "transport",
    "adapters",
    "connectors",
    "applications",
    "sdk",
    "scripts",
    "docs",
    "formal",
    "examples",
];
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

fn fail(message: &str) -> Box<dyn std::error::Error> {
    io::Error::other(message).into()
}
fn command(program: &str, args: &[&str], cwd: &Path) -> Result<Vec<u8>> {
    let output = Command::new(program).args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        return Err(fail(&format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}
fn compiler(cwd: &Path) -> Result<String> {
    require_installed(cwd, "1.95.0")?;
    let text = String::from_utf8(command("rustc", &["+1.95.0", "-vV"], cwd)?)?;
    if !text.starts_with("rustc 1.95.0 ") {
        return Err(fail("unqualified compiler"));
    }
    Ok(text)
}
fn require_installed(cwd: &Path, name: &str) -> Result<()> {
    let installed = String::from_utf8(command("rustup", &["toolchain", "list"], cwd)?)?;
    if !installed
        .lines()
        .any(|line| line.starts_with(&format!("{name}-")))
    {
        return Err(fail(
            "required compiler is not installed; no installation attempted",
        ));
    }
    Ok(())
}
fn portability(source: &Path, cache: &Path, target: &Path, expected_compiler: &str) -> Result<()> {
    // Reuse installed target libraries only when the *full* compiler identity
    // matches the pinned native compiler. Never install or float to a newer
    // stable compiler to make a missing-target gate pass.
    require_installed(source, "stable")?;
    let stable = String::from_utf8(command("rustc", &["+stable", "-vV"], source)?)?;
    if stable != expected_compiler {
        return Err(fail(
            "installed portability compiler identity mismatch; no fallback or installation performed",
        ));
    }
    for platform in ["wasm32-wasip1", "wasm32-wasip2"] {
        let output = Command::new("cargo")
            .args([
                "+stable",
                "check",
                "-p",
                "chronos-engine",
                "--target",
                platform,
                "--frozen",
            ])
            .current_dir(source)
            .env("CARGO_HOME", cache)
            .env("CARGO_TARGET_DIR", target)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC")
            .output()?;
        if !output.status.success() {
            return Err(fail(&format!(
                "{platform} qualification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
    }
    if String::from_utf8(command("rustc", &["+stable", "-vV"], source)?)? != expected_compiler {
        return Err(fail("portability compiler changed during qualification"));
    }
    Ok(())
}
fn relative(path: &Path) -> Result<String> {
    use std::path::Component;
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(fail("non-relative artifact path"));
    }
    let text = path
        .to_str()
        .ok_or_else(|| fail("non-UTF8 artifact path"))?;
    if text.chars().any(char::is_control) || text.contains('\\') {
        return Err(fail("control character in artifact path"));
    }
    Ok(text.to_owned())
}
fn files(root: &Path) -> Result<Vec<PathBuf>> {
    list_files(root, false)
}
fn list_files(root: &Path, exclude_build_state: bool) -> Result<Vec<PathBuf>> {
    fn walk(
        root: &Path,
        at: &Path,
        paths: &mut Vec<PathBuf>,
        bytes: &mut u64,
        visited: &mut usize,
        depth: usize,
        exclude_build_state: bool,
    ) -> Result<()> {
        if depth > 64 {
            return Err(fail("directory depth exceeded"));
        }
        for entry in fs::read_dir(at)? {
            *visited += 1;
            if *visited > MAX_FILES {
                return Err(fail("directory entry bound exceeded"));
            }
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(fail("symlink in source/artifact"));
            }
            let name = entry.file_name();
            if exclude_build_state && (name == "target" || name == ".git" || name == ".DS_Store") {
                continue;
            }
            if kind.is_dir() {
                walk(
                    root,
                    &path,
                    paths,
                    bytes,
                    visited,
                    depth + 1,
                    exclude_build_state,
                )?;
            } else if kind.is_file() {
                *bytes = bytes
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| fail("size overflow"))?;
                if *bytes > MAX_BYTES || paths.len() >= MAX_FILES {
                    return Err(fail("source/artifact bound exceeded"));
                }
                let rel = path.strip_prefix(root)?;
                relative(rel)?;
                paths.push(rel.to_owned());
            } else {
                return Err(fail("special file in source/artifact"));
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(fail("expected non-symlink directory"));
    }
    walk(
        root,
        root,
        &mut paths,
        &mut 0,
        &mut 0,
        0,
        exclude_build_state,
    )?;
    paths.sort();
    Ok(paths)
}
fn digests(root: &Path, paths: &[PathBuf]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut result = std::collections::BTreeMap::new();
    // Bound argv size and amortize process startup instead of one Perl process
    // per file. No shell interpolation or filenames interpreted as options.
    for chunk in paths.chunks(32) {
        let names = chunk
            .iter()
            .map(|p| relative(p))
            .collect::<Result<Vec<_>>>()?;
        let args: Vec<_> = ["-a", "256", "--"]
            .into_iter()
            .chain(names.iter().map(String::as_str))
            .collect();
        let text = String::from_utf8(command("shasum", &args, root)?)?;
        let lines: Vec<_> = text.lines().collect();
        if lines.len() != names.len() {
            return Err(fail("digest count mismatch"));
        }
        for (line, name) in lines.into_iter().zip(names) {
            let (hash, returned) = line
                .split_once("  ")
                .ok_or_else(|| fail("invalid digest record"))?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) || returned != name
            {
                return Err(fail("invalid digest correspondence"));
            }
            result.insert(name, hash.to_ascii_lowercase());
        }
    }
    Ok(result)
}
fn copy_tree(source: &Path, destination: &Path, exclude_build_state: bool) -> Result<()> {
    fs::create_dir(destination)?;
    let canonical_destination = destination.canonicalize()?;
    let destination = canonical_destination.as_path();
    for relative in list_files(source, exclude_build_state)? {
        let output = destination.join(&relative);
        fs::create_dir_all(output.parent().ok_or_else(|| fail("missing parent"))?)?;
        fs::copy(source.join(relative), output)?;
    }
    Ok(())
}
fn inventory(metadata: &Value, root: &Path) -> Result<Value> {
    let mut packages = Vec::new();
    for package in metadata["packages"]
        .as_array()
        .ok_or_else(|| fail("missing packages"))?
    {
        let name = package["name"]
            .as_str()
            .ok_or_else(|| fail("missing package name"))?;
        let version = package["version"]
            .as_str()
            .ok_or_else(|| fail("missing package version"))?;
        let source = match package["source"].as_str() {
            None if Path::new(package["manifest_path"].as_str().unwrap_or(""))
                .starts_with(root.join("third_party")) =>
            {
                "vendored_path"
            }
            None => "workspace",
            Some("registry+https://github.com/rust-lang/crates.io-index") => "crates.io",
            Some(_) => return Err(fail("unqualified dependency source")),
        };
        let location = if source == "workspace" || source == "vendored_path" {
            let manifest = Path::new(
                package["manifest_path"]
                    .as_str()
                    .ok_or_else(|| fail("missing manifest"))?,
            );
            relative(manifest.strip_prefix(root)?)?
        } else {
            format!("vendor/{name}-{version}/Cargo.toml")
        };
        packages.push(json!({"name": name, "version": version, "source": source,
            "manifest": location, "declared_license": package["license"],
            "license_review": "not_performed", "advisory_review": "not_performed"}));
    }
    packages.sort_by_key(|p| {
        (
            p["name"].as_str().unwrap().to_owned(),
            p["version"].as_str().unwrap().to_owned(),
        )
    });
    Ok(
        json!({"schema": 1, "scope": "workspace_with_build_and_dev_dependencies", "manifest_base": "source",
        "not_a_license_or_vulnerability_approval": true, "packages": packages}),
    )
}
fn copy_root_files(root: &Path, destination: &Path, names: &[&str], required: bool) -> Result<()> {
    for name in names {
        let input = root.join(name);
        match fs::symlink_metadata(&input) {
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::copy(input, destination.join(name))?;
            }
            Ok(_) => return Err(fail("expected non-symlink source file")),
            Err(error) if !required && error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn create(root: &Path, destination: &Path) -> Result<()> {
    let planned = planned_destination(destination)?;
    if ROOT_DIRS
        .iter()
        .any(|name| planned.starts_with(root.join(name)))
    {
        return Err(fail("bundle destination overlaps captured source"));
    }
    let compiler = compiler(root)?;
    // A fresh, private destination never overwrites a user's existing artifact.
    fs::create_dir(destination)?;
    let canonical_destination = destination.canonicalize()?;
    let destination = canonical_destination.as_path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o700))?;
    }
    let source = destination.join("source");
    fs::create_dir(&source)?;
    copy_root_files(root, &source, ROOT_FILES, true)?;
    copy_root_files(root, &source, ROOT_LICENSE_FILES, false)?;
    for name in ROOT_DIRS {
        let input = root.join(name);
        if fs::symlink_metadata(&input)?.file_type().is_symlink() {
            return Err(fail("symlink source root"));
        }
        copy_tree(&input, &source.join(name), true)?;
    }
    // Cargo vendor resolves against the captured lockfile, without network.
    let vendor = command(
        "cargo",
        &[
            "+1.95.0",
            "vendor",
            "--locked",
            "--offline",
            "--versioned-dirs",
            "vendor",
        ],
        &source,
    )?;
    fs::create_dir(source.join(".cargo"))?;
    fs::write(source.join(".cargo/config.toml"), vendor)?;
    let metadata: Value = serde_json::from_slice(&command(
        "cargo",
        &["+1.95.0", "metadata", "--frozen", "--format-version", "1"],
        &source,
    )?)?;
    fs::write(
        destination.join("inventory.json"),
        serde_json::to_vec_pretty(&inventory(&metadata, &source)?)?,
    )?;
    fs::write(destination.join("compiler.txt"), compiler)?;
    fs::write(
        destination.join("profile.json"),
        serde_json::to_vec_pretty(&json!({
            "schema": 1, "profile": "private_local_source", "toolchain": "1.95.0",
            "source": "captured_working_tree_not_git_commit", "signed": false,
            "installed_or_published": false, "portability_targets": ["wasm32-wasip1", "wasm32-wasip2"]
        }))?,
    )?;
    let hashes = digests(destination, &files(destination)?)?;
    fs::write(
        destination.join("checksums.json"),
        serde_json::to_vec_pretty(&hashes)?,
    )?;
    verify(destination)?;
    println!("source_bundle=captured checksum_verified=true installation=false publication=false");
    Ok(())
}
fn planned_destination(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| fail("destination needs a filename"))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent.canonicalize()?.join(name))
}
fn verify(root: &Path) -> Result<()> {
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(fail("expected non-symlink artifact directory"));
    }
    let manifest = fs::symlink_metadata(root.join("checksums.json"))?;
    if !manifest.file_type().is_file() || manifest.len() > 4 * 1024 * 1024 {
        return Err(fail("manifest too large"));
    }
    let bytes = fs::read(root.join("checksums.json"))?;
    let hashes: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes)?;
    let actual: BTreeSet<_> = files(root)?
        .into_iter()
        .filter(|p| p != Path::new("checksums.json"))
        .map(|p| relative(&p))
        .collect::<Result<_>>()?;
    for required in [
        "source/Cargo.toml",
        "source/Cargo.lock",
        "source/.cargo/config.toml",
        "compiler.txt",
        "inventory.json",
        "profile.json",
    ] {
        if !hashes.contains_key(required) {
            return Err(fail("incomplete source bundle"));
        }
    }
    if actual != hashes.keys().cloned().collect() {
        return Err(fail("artifact file-set mismatch"));
    }
    let actual_paths = actual.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    if digests(root, &actual_paths)? != hashes {
        return Err(fail("artifact checksum mismatch"));
    }
    Ok(())
}
fn check(root: &Path, workspace: &Path) -> Result<()> {
    verify(root)?;
    if compiler(root)? != fs::read_to_string(root.join("compiler.txt"))? {
        return Err(fail("compiler identity mismatch"));
    }
    let root = root.canonicalize()?;
    let planned = planned_destination(workspace)?;
    if planned.starts_with(&root) {
        return Err(fail("build workspace must be outside artifact"));
    }
    for parent in planned.parent().into_iter().flat_map(Path::ancestors) {
        if parent.join(".cargo/config").exists() || parent.join(".cargo/config.toml").exists() {
            return Err(fail(
                "ambient ancestor Cargo configuration prevents isolated qualification",
            ));
        }
    }
    fs::create_dir(workspace)?;
    let workspace = workspace.canonicalize()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700))?;
    }
    let source = workspace.join("source");
    copy_tree(&root.join("source"), &source, false)?;
    let cache = workspace.join("cargo-home");
    fs::create_dir(&cache)?;
    let output = Command::new("cargo")
        .args([
            "+1.95.0",
            "test",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--frozen",
        ])
        .current_dir(&source)
        .env("CARGO_HOME", &cache)
        .env("CARGO_TARGET_DIR", workspace.join("target"))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC")
        .output()?;
    fs::write(workspace.join("native-tests.stdout"), output.stdout)?;
    fs::write(workspace.join("native-tests.stderr"), output.stderr)?;
    if !output.status.success() {
        return Err(fail(
            "isolated native qualification failed; inspect workspace native-tests.stderr/stdout",
        ));
    }
    portability(
        &source,
        &cache,
        &workspace.join("target"),
        &fs::read_to_string(root.join("compiler.txt"))?,
    )?;
    verify(&root)?;
    println!(
        "source_bundle=isolated_tests_passed cache=initially_empty cargo=frozen portability=identity_matched_wasi_compile authenticity=not_claimed"
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err(fail(
            "usage: source-bundle create|verify DESTINATION; check DESTINATION FRESH_WORKSPACE",
        ));
    }
    let destination = PathBuf::from(&args[1]);
    match args[0].to_str() {
        Some("create") if args.len() == 2 => create(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()?,
            &destination,
        ),
        Some("verify") if args.len() == 2 => {
            verify(&destination)?;
            println!("source_bundle=checksum_verified authenticity=not_claimed");
            Ok(())
        }
        Some("check") if args.len() == 3 => check(&destination, &PathBuf::from(&args[2])),
        _ => Err(fail("unknown command")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_paths_refuse_traversal_absolute_and_controls() {
        for path in ["", "../secret", "/secret", "a/../secret", "a\nsecret"] {
            assert!(relative(Path::new(path)).is_err());
        }
        assert_eq!(
            relative(Path::new("source/Cargo.lock")).unwrap(),
            "source/Cargo.lock"
        );
    }
    #[test]
    fn inventory_is_sorted_content_minimized_and_refuses_foreign_sources() {
        let root = Path::new("/fixture");
        let input = json!({"packages": [{"name": "z", "version": "1", "source": null, "manifest_path": "/fixture/z/Cargo.toml", "license": "MIT"},
            {"name": "a", "version": "2", "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "Apache-2.0"}]});
        let value = inventory(&input, root).unwrap();
        assert_eq!(value["packages"][0]["name"], "a");
        assert!(!value.to_string().contains("/fixture"));
        let mut hostile = input;
        hostile["packages"][1]["source"] = json!("git+https://private.example/repo");
        assert!(inventory(&hostile, root).is_err());
        let snapshot = json!({"packages": [{"name": "keystone-storage", "version": "0.0.1", "source": null,
            "manifest_path": "/fixture/third_party/keystone-storage-0.0.1/Cargo.toml", "license": "MIT OR Apache-2.0"}]});
        let snapshot = inventory(&snapshot, root).unwrap();
        assert_eq!(snapshot["packages"][0]["source"], "vendored_path");
        assert_eq!(
            snapshot["packages"][0]["manifest"],
            "third_party/keystone-storage-0.0.1/Cargo.toml"
        );
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "chronos-source-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn seal(&self) {
            let hashes = digests(&self.0, &files(&self.0).unwrap()).unwrap();
            fs::write(
                self.0.join("checksums.json"),
                serde_json::to_vec(&hashes).unwrap(),
            )
            .unwrap();
        }
        fn bundle() -> Self {
            let fixture = Self::new();
            for path in [
                "source/Cargo.toml",
                "source/Cargo.lock",
                "source/.cargo/config.toml",
                "compiler.txt",
                "inventory.json",
                "profile.json",
            ] {
                let at = fixture.0.join(path);
                fs::create_dir_all(at.parent().unwrap()).unwrap();
                fs::write(at, b"fixture").unwrap();
            }
            fixture.seal();
            fixture
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn checksums_refuse_changed_missing_and_extra_files_including_build_named_paths() {
        for mutation in 0..3 {
            let fixture = Fixture::bundle();
            verify(&fixture.0).unwrap();
            match mutation {
                0 => fs::write(fixture.0.join("source/Cargo.lock"), b"tampered").unwrap(),
                1 => fs::remove_file(fixture.0.join("source/Cargo.lock")).unwrap(),
                2 => {
                    fs::create_dir(fixture.0.join("target")).unwrap();
                    fs::write(fixture.0.join("target/unlisted"), b"extra").unwrap();
                }
                _ => unreachable!(),
            }
            assert!(verify(&fixture.0).is_err());
        }
    }
    #[test]
    fn incomplete_bundle_and_existing_destination_refuse_without_overwriting() {
        let fixture = Fixture::new();
        fixture.seal();
        assert!(verify(&fixture.0).is_err());
        let marker = fixture.0.join("owned");
        fs::write(&marker, b"preserve").unwrap();
        assert!(create(Path::new(env!("CARGO_MANIFEST_DIR")), &fixture.0).is_err());
        assert_eq!(fs::read(marker).unwrap(), b"preserve");
    }
    #[cfg(unix)]
    #[test]
    fn root_license_files_are_captured_and_symlink_notices_are_refused() {
        let root = Fixture::new();
        let destination = Fixture::new();
        fs::write(root.0.join("LICENSE-MIT"), b"fixture").unwrap();
        copy_root_files(&root.0, &destination.0, ROOT_LICENSE_FILES, false).unwrap();
        assert_eq!(
            fs::read(destination.0.join("LICENSE-MIT")).unwrap(),
            b"fixture"
        );
        assert!(!destination.0.join("LICENSE-APACHE").exists());
        std::os::unix::fs::symlink(root.0.join("LICENSE-MIT"), root.0.join("NOTICE")).unwrap();
        assert!(copy_root_files(&root.0, &destination.0, ROOT_LICENSE_FILES, false).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symbolic_links_refuse_even_when_their_target_is_readable() {
        let fixture = Fixture::bundle();
        let external = Fixture::new();
        fs::write(external.0.join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(external.0.join("secret"), fixture.0.join("foreign")).unwrap();
        assert!(verify(&fixture.0).is_err());
        assert!(copy_tree(&fixture.0, &external.0.join("copy"), true).is_err());
        assert_eq!(fs::read(external.0.join("secret")).unwrap(), b"secret");
    }
}
