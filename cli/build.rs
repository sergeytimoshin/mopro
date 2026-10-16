mod template_stage;

fn main() {
    let source = std::path::Path::new("src/template/init");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=template_stage.rs");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("init-template");
    if output.exists() {
        std::fs::remove_dir_all(&output).expect("remove old template staging directory");
    }
    template_stage::stage(source, &output).expect("stage source templates for embedding");
}
