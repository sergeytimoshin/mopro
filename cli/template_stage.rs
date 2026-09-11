use std::{fs, io, path::Path};

/// Stage source templates before include_dir! expands. Git ignores do not apply
/// to that macro, so build outputs must be excluded before compilation.
pub fn stage(source: &Path, output: &Path) -> io::Result<()> {
    fs::create_dir_all(output)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let destination = output.join(&name);
        if kind.is_dir() {
            if ["target", "build", "node_modules", ".git", "pkg"]
                .iter()
                .any(|ignored| name == *ignored)
            {
                continue;
            }
            stage(&entry.path(), &destination)?;
        } else if kind.is_file() && name != ".DS_Store" {
            fs::copy(entry.path(), destination)?;
        } else if kind.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "symlink in source template",
            ));
        }
    }
    Ok(())
}
