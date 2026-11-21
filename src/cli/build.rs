use std::{env, fs, path::PathBuf};

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=python/lens_cli");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    for entry in fs::read_dir("python/lens_cli")? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::copy(entry.path(), output.join(entry.file_name()))?;
        }
    }
    Ok(())
}
