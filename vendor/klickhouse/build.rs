use rustc_version::{Version, version};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if version().unwrap() >= Version::parse("1.51.0").unwrap() {
        println!("cargo:rustc-cfg=const_generics");
    }
}
