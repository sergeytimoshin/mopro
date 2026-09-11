#[path = "../template_stage.rs"]
mod template_stage;

use std::{fs, time::SystemTime};

#[test]
fn excludes_generated_artifacts_before_embedding_but_preserves_source_and_fixtures() {
    let suffix = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mopro-template-stage-{suffix}"));
    let source = root.join("source");
    let output = root.join("output");
    let retained = [
        "gnark-web/go.mod",
        "gnark-web/prover.go",
        "gnark-web/go.sum",
        "build.rs",
        "test-vectors/circuit.wasm",
    ];
    let ignored = [
        "gnark-web/target/gnark.wasm",
        "gnark-web/pkg/gnark.js",
        "node_modules/unused.js",
        ".git/config",
        "build/tmp/binary",
        ".DS_Store",
    ];
    for name in retained.iter().chain(&ignored) {
        let file = source.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, name).unwrap();
    }
    template_stage::stage(&source, &output).unwrap();
    for name in retained {
        assert_eq!(fs::read_to_string(output.join(name)).unwrap(), name);
    }
    for name in ignored {
        assert!(!output.join(name).exists(), "embedded {name}");
    }
    fs::remove_dir_all(root).unwrap();
}
