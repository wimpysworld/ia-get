#[cfg(target_os = "windows")]
extern crate winres;

fn main() {
    // Handle Windows-specific manifest for long path support
    #[cfg(target_os = "windows")]
    embed_windows_manifest();

    // Note: Artifact packaging is now handled by CI/CD workflow after build completion
    // The build script runs before the binary is created, so we can't package it here

    println!("cargo:warning=Build script completed - artifacts will be created by CI/CD workflow");
}

#[cfg(target_os = "windows")]
fn embed_windows_manifest() {
    let mut res = winres::WindowsResource::new();
    res.set_manifest_file("ia-get.exe.manifest");

    if let Err(e) = res.compile() {
        println!("cargo:warning=Failed to embed Windows manifest: {}", e);
    } else {
        println!("cargo:warning=Windows manifest embedded successfully");
    }
}
