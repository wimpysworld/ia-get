use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Only run packaging logic when building in release mode
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    if profile != "release" {
        return;
    }

    // Get build target information
    let target = env::var("TARGET").unwrap_or_else(|_| env::consts::ARCH.to_string());
    let os = env::consts::OS;

    println!("cargo:warning=Building for {} on {}", target, os);

    // Get package information
    let package_name = env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "ia-get".to_string());
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "dev".to_string());

    // Create artifacts directory
    let artifacts_dir = PathBuf::from("artifacts");
    if !artifacts_dir.exists() {
        fs::create_dir_all(&artifacts_dir).expect("Failed to create artifacts directory");
    }

    // Copy binary to artifacts
    let binary_name = if os == "windows" {
        format!("{}.exe", package_name)
    } else {
        package_name.clone()
    };

    let source_binary = PathBuf::from("target").join("release").join(&binary_name);
    let dest_binary = artifacts_dir.join(&binary_name);

    if source_binary.exists() {
        fs::copy(&source_binary, &dest_binary).expect("Failed to copy binary");
        println!("cargo:warning=Copied binary to {}", dest_binary.display());
    }

    // Copy documentation files
    let doc_files = ["README.md", "LICENSE"];
    for doc_file in &doc_files {
        let doc_path = Path::new(doc_file);
        if doc_path.exists() {
            let dest_doc = artifacts_dir.join(doc_file);
            fs::copy(doc_path, &dest_doc).expect("Failed to copy documentation");
            println!(
                "cargo:warning=Copied {} to {}",
                doc_file,
                dest_doc.display()
            );
        }
    }

    // Generate package name
    let package_name_full = format!("{}-{}-{}", package_name, version, target);

    // Create archive based on platform
    if os == "windows" {
        create_zip_archive(&artifacts_dir, &package_name_full);
    } else {
        create_tar_archive(&artifacts_dir, &package_name_full);
    }

    println!(
        "cargo:warning=Packaging completed for {}",
        package_name_full
    );
}

fn create_zip_archive(artifacts_dir: &Path, package_name: &str) {
    let zip_name = format!("{}.zip", package_name);
    let zip_path = artifacts_dir.with_file_name(&zip_name);

    // Use 7z if available (better compression), otherwise use built-in zip
    let use_7z = Command::new("7z").arg("--help").output().is_ok();

    if use_7z {
        let status = Command::new("7z")
            .args(["a", &zip_path.to_string_lossy(), "."])
            .current_dir(artifacts_dir)
            .status()
            .expect("Failed to create zip archive with 7z");

        if status.success() {
            println!("cargo:warning=Created zip archive: {}", zip_path.display());
        }
    } else {
        println!("cargo:warning=7z not available, skipping archive creation");
    }
}

fn create_tar_archive(artifacts_dir: &Path, package_name: &str) {
    let tar_name = format!("{}.tar.gz", package_name);
    let tar_path = artifacts_dir.with_file_name(&tar_name);

    let status = Command::new("tar")
        .args(["czf", &tar_path.to_string_lossy(), "."])
        .current_dir(artifacts_dir)
        .status()
        .expect("Failed to create tar archive");

    if status.success() {
        println!("cargo:warning=Created tar archive: {}", tar_path.display());
    }
}
