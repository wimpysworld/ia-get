fn main() {
    println!("cargo:rerun-if-env-changed=BUILD_VERSION");
}
