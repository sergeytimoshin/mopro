use std::{fs, process::Command, time::SystemTime};

#[test]
fn scaffolds_gnark_web_only_when_selected_and_keeps_native_dependencies_scoped() {
    let suffix = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mopro-gnark-scaffold-{suffix}"));
    fs::create_dir(&root).unwrap();
    for (adapter, name) in [("halo2,gnark", "with-gnark"), ("halo2", "without-gnark")] {
        let output = Command::new(env!("CARGO_BIN_EXE_mopro"))
            .current_dir(&root)
            .args(["init", "--adapter", adapter, "--project-name", name])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let project = root.join(name);
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(project.join("Cargo.toml")).unwrap()).unwrap();
        assert!(manifest["dependencies"].get("rust-gnark").is_none());
        assert!(manifest["dependencies"].get("plonk-fibonacci").is_some());
        if name == "with-gnark" {
            assert!(manifest.get("patch").is_none());
            assert_eq!(
                manifest["target"]["cfg(not(target_arch = \"wasm32\"))"]["dependencies"]
                    ["rust-gnark"]
                    .as_str(),
                Some("0.0.2")
            );
            for file in [
                "go.mod",
                "go.sum",
                "main_js.go",
                "gnark.worker.js",
                "gnark.js",
                "gnark.d.ts",
                "proof_decode.go",
            ] {
                assert!(project.join("gnark-web").join(file).is_file());
            }
            assert!(!project.join("gnark-web/accelerator").exists());
        } else {
            assert!(!project.join("gnark-web").exists());
            assert!(manifest.get("patch").is_none());
        }
    }
    fs::remove_dir_all(root).unwrap();
}
