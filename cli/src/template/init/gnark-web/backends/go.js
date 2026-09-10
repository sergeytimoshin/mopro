// Selected at build time; this module has no Rust imports or dependencies.
export async function initAccelerator(options) {
    if (options.experimental) {
        throw new Error("Gnark accelerator is not included in this build; set package.metadata.mopro.gnark.experimental-accelerator = true in Cargo.toml and rebuild");
    }
    return 0;
}
