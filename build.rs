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

    // Try 7z first (best compression), then PowerShell, then skip with warning
    if Command::new("7z").arg("--help").output().is_ok() {
        // Use 7z if available
        let status = Command::new("7z")
            .args(["a", &zip_path.to_string_lossy(), "."])
            .current_dir(artifacts_dir)
            .status();

        match status {
            Ok(s) if s.success() => {
                println!(
                    "cargo:warning=Created zip archive with 7z: {}",
                    zip_path.display()
                );
                return;
            }
            _ => {
                println!("cargo:warning=7z failed, trying PowerShell...");
            }
        }
    } else {
        println!("cargo:warning=7z not available, trying PowerShell...");
    }

    // Fallback to PowerShell compression if 7z failed or not available
    let ps_script = format!(
        "try {{ Compress-Archive -Path (Join-Path '{}' '*') -DestinationPath '{}' -Force; Write-Host 'SUCCESS' }} catch {{ Write-Host 'ERROR: $($_.Exception.Message)' }}",
        artifacts_dir.to_string_lossy(),
        zip_path.to_string_lossy()
    );

    println!("cargo:warning=Running PowerShell command: {}", ps_script);

    let output = Command::new("powershell")
        .args(["-Command", &ps_script])
        .output();

    match output {
        Ok(result) => {
            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);

            if result.status.success() && stdout.contains("SUCCESS") {
                println!(
                    "cargo:warning=Created zip archive with PowerShell: {}",
                    zip_path.display()
                );
            } else {
                println!("cargo:warning=PowerShell compression failed");
                println!("cargo:warning=STDOUT: {}", stdout);
                println!("cargo:warning=STDERR: {}", stderr);
                println!(
                    "cargo:warning=Files are available in: {}",
                    artifacts_dir.display()
                );
            }
        }
        Err(e) => {
            println!("cargo:warning=Failed to run PowerShell: {}", e);
            println!(
                "cargo:warning=Files are available in: {}",
                artifacts_dir.display()
            );
        }
    }
}

fn create_tar_archive(artifacts_dir: &Path, package_name: &str) {
    let tar_name = format!("{}.tar.gz", package_name);
    let tar_path = artifacts_dir.with_file_name(&tar_name);

    // Try tar command first
    let status = Command::new("tar")
        .args(["czf", &tar_path.to_string_lossy(), "."])
        .current_dir(artifacts_dir)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:warning=Created tar archive: {}", tar_path.display());
        }
        _ => {
            println!("cargo:warning=tar command failed, trying gzip fallback...");

            // Fallback: create tar file first, then compress with gzip
            let tar_only_name = format!("{}.tar", package_name);
            let tar_only_path = artifacts_dir.with_file_name(&tar_only_name);

            let tar_status = Command::new("tar")
                .args(["cf", &tar_only_path.to_string_lossy(), "."])
                .current_dir(artifacts_dir)
                .status();

            if tar_status.is_ok() && tar_status.unwrap().success() {
                let gzip_status = Command::new("gzip")
                    .arg(tar_only_path.to_string_lossy().as_ref())
                    .status();

                match gzip_status {
                    Ok(s) if s.success() => {
                        println!(
                            "cargo:warning=Created tar.gz archive with gzip: {}",
                            tar_path.display()
                        );
                    }
                    _ => {
                        println!(
                            "cargo:warning=gzip compression failed, tar file available: {}",
                            tar_only_path.display()
                        );
                    }
                }
            } else {
                println!(
                    "cargo:warning=Archive creation failed, files are available in: {}",
                    artifacts_dir.display()
                );
            }
        }
    }
}
