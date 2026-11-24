use std::path::{Path, PathBuf};

/// Reads a test WASM fixture from an explicit path or the workspace build output.
pub fn read_wasm_artifact(path_var: &str, file_name: &str) -> Vec<u8> {
    let path = std::env::var_os(path_var)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("POOL_WASM_PATH").and_then(|pool_path| {
                PathBuf::from(pool_path)
                    .parent()
                    .map(|dir| dir.join(file_name))
            })
        })
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/wasm32v1-none/release")
                .join(file_name)
        });
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "{file_name} WASM not found at {}: {error}. Run 'make deploy-artifacts' first.",
            path.display()
        )
    })
}
