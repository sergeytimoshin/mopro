use anyhow::{bail, Context};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::{fs, path::PathBuf};

use crate::app_config::cleanup_tmp_local;
use crate::app_config::constants::{
    Mode, PlatformBuilder, WebArch, WebPlatform, WASM_NIGHTLY_TOOLCHAIN, WEB_BINDINGS_DIR,
};

use super::mktemp_local;

// Maintained for backwards compatibility
#[inline]
pub fn build() {
    super::build_from_env::<WebPlatform>()
}

impl PlatformBuilder for WebPlatform {
    type Arch = WebArch;
    type Params = ();

    fn build(
        mode: Mode,
        project_dir: &Path,
        _target_archs: Vec<Self::Arch>,
        _params: Self::Params,
    ) -> anyhow::Result<PathBuf> {
        if !project_dir.join("Cargo.toml").exists() {
            panic!("No Cargo.toml found in {:?}", project_dir);
        }
        let gnark_accelerator =
            gnark_accelerator_enabled(&fs::read_to_string(project_dir.join("Cargo.toml"))?)?;
        let build_dir_path = project_dir.join("build");
        let work_dir = mktemp_local(&build_dir_path);
        let bindings_out = work_dir.join(WEB_BINDINGS_DIR);
        fs::create_dir(&bindings_out).expect("Failed to create bindings out directory");
        let bindings_dest = Path::new(&project_dir).join(WEB_BINDINGS_DIR);

        let mode_cmd = match mode {
            Mode::Release => "--release",
            Mode::Debug => "--dev",
        };

        let mut cmd = Command::new("rustup");
        cmd.args([
            "run",
            WASM_NIGHTLY_TOOLCHAIN,
            "wasm-pack",
            "build",
            "--target",
            "web",
            mode_cmd,
            "--out-dir",
            bindings_out.to_str().unwrap(),
            "--out-name",
            "mopro_wasm_lib",
            "--no-default-features",
            "--features",
            "wasm",
        ]);

        cmd.env(
            "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS",
            "-C target-feature=+atomics,+bulk-memory,+mutable-globals \
             -C link-arg=--shared-memory \
             -C link-arg=--max-memory=1073741824 \
             -C link-arg=--import-memory \
             -C link-arg=--export=__wasm_init_tls \
             -C link-arg=--export=__tls_size \
             -C link-arg=--export=__tls_align \
             -C link-arg=--export=__tls_base",
        );

        cmd.current_dir(project_dir);

        let status = cmd.status().expect("Failed to run wasm-pack");

        if status.success() {
            println!("mopro-ffi wasm package build completed successfully.");
        } else {
            eprintln!("mopro-ffi wasm package build failed.");
            std::process::exit(1);
        }

        build_gnark(project_dir, &bindings_out, gnark_accelerator)?;

        if let Ok(info) = fs::metadata(&bindings_dest) {
            if !info.is_dir() {
                panic!("framework directory exists and is not a directory");
            }
            fs::remove_dir_all(&bindings_dest).expect("Failed to remove framework directory");
        }

        fs::rename(&bindings_out, &bindings_dest).expect("Failed to move framework into place");

        cleanup_tmp_local(&build_dir_path);

        Ok(bindings_dest)
    }
}

/// Go's browser target needs its own runtime; it cannot use the native C bridge.
/// Presence of this scaffold opts the project into building gnark web bindings.
fn build_gnark(project_dir: &Path, bindings_out: &Path, accelerator: bool) -> anyhow::Result<()> {
    let source = project_dir.join("gnark-web");
    if !source.join("go.mod").is_file() {
        return Ok(());
    }
    let output = bindings_out.join("gnark");
    fs::create_dir(&output)?;
    let status = Command::new("go")
        .current_dir(&source)
        .env("GOOS", "js")
        .env("GOARCH", "wasm")
        .env("CGO_ENABLED", "0")
        .args([
            "build",
            "-mod=readonly",
            "-trimpath",
            "-ldflags=-s -w",
            "-o",
        ])
        .arg(output.join("gnark.wasm"))
        .arg(".")
        .status()
        .context("Building gnark for web requires Go 1.24 or newer on PATH")?;
    if !status.success() {
        bail!("gnark Go WASM build failed: {status}");
    }

    if accelerator {
        let status = Command::new("rustup")
            .current_dir(source.join("accelerator"))
            .env("CARGO_TARGET_DIR", project_dir.join("target/gnark-web"))
            .args([
                "run",
                WASM_NIGHTLY_TOOLCHAIN,
                "wasm-pack",
                "build",
                "--target",
                "web",
                "--release",
                "--out-name",
                "gnark_kernel",
                "--out-dir",
            ])
            .arg(output.join("accelerator"))
            .args(["--", "--locked"])
            .status()
            .context("Cannot build gnark's threaded WASM arithmetic module")?;
        if !status.success() {
            bail!("gnark arithmetic WASM build failed: {status}");
        }
        // wasm-pack writes a wildcard .gitignore. npm applies it when this nested
        // package is packed as part of the parent bindings, omitting the kernel and
        // its thread helpers. An empty .npmignore keeps those runtime assets.
        fs::write(output.join("accelerator/.npmignore"), "")?;
        for license in ["LICENSE-APACHE", "LICENSE-MIT"] {
            fs::copy(
                source.join("accelerator/ark-bn254").join(license),
                output.join("accelerator").join(license),
            )?;
        }
    }
    fs::copy(source.join("LICENSE-APACHE"), output.join("LICENSE-APACHE"))?;
    // Copy only the selected backend. Go-only output has no accelerator imports.
    let backend = if accelerator { "rust.js" } else { "go.js" };
    fs::copy(
        source.join("backends").join(backend),
        output.join("gnark.backend.js"),
    )?;

    // Use the same compiler's runtime, including when Go selects a toolchain
    // from go.mod. Do not copy wasm_exec.js from a separately installed Go.
    let goroot = Command::new("go")
        .current_dir(&source)
        .args(["env", "GOROOT"])
        .output()
        .context("Cannot locate the Go WASM runtime")?;
    if !goroot.status.success() {
        bail!("go env GOROOT failed");
    }
    let goroot = PathBuf::from(String::from_utf8(goroot.stdout)?.trim());
    fs::copy(
        goroot.join("lib/wasm/wasm_exec.js"),
        output.join("wasm_exec.js"),
    )
    .context("Cannot copy wasm_exec.js; gnark web requires Go 1.24 or newer")?;
    for file in ["gnark.js", "gnark.worker.js", "gnark.d.ts"] {
        fs::copy(source.join(file), output.join(file))?;
    }
    for file in ["mopro_wasm_lib.js", "mopro_wasm_lib.d.ts"] {
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .open(bindings_out.join(file))?,
            "\nexport {{ initGnark, disposeGnark, prepareGnarkCircuit, generateGnarkProof, verifyGnarkProof }} from './gnark/gnark.js';"
        )?;
    }
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(bindings_out.join("mopro_wasm_lib.d.ts"))?,
        "export type {{ GnarkProofResult, GnarkCircuit, GnarkCircuitKeys, GnarkOptions, GnarkRuntimeInfo, GnarkExecution }} from './gnark/gnark.js';"
    )?;
    let package_path = bindings_out.join("package.json");
    let mut package: serde_json::Value = serde_json::from_slice(&fs::read(&package_path)?)?;
    let files = package["files"]
        .as_array_mut()
        .context("wasm-pack package.json must contain a files array")?;
    files.push(serde_json::json!("gnark"));
    // The top-level module imports Rayon helpers even in a gnark-only app.
    // wasm-pack's files whitelist can omit these from the npm tarball.
    if bindings_out.join("snippets").is_dir() && !files.iter().any(|file| file == "snippets") {
        files.push(serde_json::json!("snippets"));
    }
    // wasm_exec.js installs globalThis.Go as a side effect. Bundlers must
    // retain that import when processing the module worker.
    if package["sideEffects"] == false {
        package["sideEffects"] = serde_json::json!([]);
    }
    if let Some(side_effects) = package["sideEffects"].as_array_mut() {
        side_effects.push(serde_json::json!("./gnark/wasm_exec.js"));
        side_effects.push(serde_json::json!("./gnark/gnark.worker.js"));
    }
    fs::write(package_path, serde_json::to_vec_pretty(&package)?)?;
    println!("gnark Go WASM bindings built successfully.");
    Ok(())
}

/// The same project setting applies to both the CLI and generated web helper.
fn gnark_accelerator_enabled(manifest: &str) -> anyhow::Result<bool> {
    let manifest: toml::Value = toml::from_str(manifest).context("Cannot parse Cargo.toml")?;
    let setting = manifest
        .get("package")
        .and_then(|value| value.get("metadata"))
        .and_then(|value| value.get("mopro"))
        .and_then(|value| value.get("gnark"))
        .and_then(|value| value.get("experimental-accelerator"));
    match setting {
        None => Ok(false),
        Some(value) => value
            .as_bool()
            .context("package.metadata.mopro.gnark.experimental-accelerator must be a boolean"),
    }
}

#[cfg(test)]
mod tests {
    use super::gnark_accelerator_enabled;

    #[test]
    fn gnark_accelerator_requires_explicit_build_opt_in() {
        assert!(!gnark_accelerator_enabled("[package]\nname = 'example'").unwrap());
        for enabled in [false, true] {
            let manifest =
                format!("[package.metadata.mopro.gnark]\nexperimental-accelerator = {enabled}");
            assert_eq!(gnark_accelerator_enabled(&manifest).unwrap(), enabled);
        }
        assert!(gnark_accelerator_enabled(
            "[package.metadata.mopro.gnark]\nexperimental-accelerator = 'true'"
        )
        .is_err());
    }
}
