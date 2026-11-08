fn main() {
    // Declare the app's own commands so that capabilities must grant each one explicitly.
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(&["has_capability_profile"]));
    if let Err(err) = tauri_build::try_build(attributes) {
        eprintln!("tauri-build failed: {err:#}");
        std::process::exit(1);
    }
}
