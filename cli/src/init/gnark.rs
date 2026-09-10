use crate::init::adapter::Adapter;
use crate::init::proving_system::ProvingSystem;
use include_dir::include_dir;
use include_dir::Dir;
use std::io::Write;

pub struct Gnark;

impl ProvingSystem for Gnark {
    const TEMPLATE_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/src/template/gnark");

    const ADAPTER: Adapter = Adapter::Gnark;

    // The C/Go bridge cannot be linked into Rust WASM. Browser builds use
    // the Go module in gnark-web instead. Append the table so subsequent
    // adapter dependencies keep their original TOML scope.
    fn dep_template(file_path: &str) -> anyhow::Result<()> {
        let mut file = std::fs::OpenOptions::new().append(true).open(file_path)?;
        file.write_all(
            br#"
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
rust-gnark = "0.0.2"

# The published mopro-ffi does not yet build the gnark web modules.
# Pin the matching build helper until a release includes this adapter.
[patch.crates-io]
mopro-ffi = { git = "https://github.com/sergeytimoshin/mopro.git", rev = "decffd93936a55948687568e2f51ccb786d93910" }
"#,
        )?;
        Ok(())
    }
}
