fn main() {
    let mut cx_bridge = cxx_build::bridge("src/main.rs");
    let mut cx = cx_bridge
        .file("src/cbfs_fuse.cpp")
        .include("include")
        .std("c++20")
        .define("_FILE_OFFSET_BITS", "64")
        .define("FUSE_USE_VERSION", "35");

    let fsp_base = std::path::Path::new("C:\\Program Files (x86)\\WinFsp");

    if cfg!(target_os = "windows") {
        cx = cx.include(fsp_base.join("inc")).define("WIN32", "1");
    } else if cfg!(target_os = "macos") || cfg!(target_os = "freebsd") {
        cx = cx.include("/usr/local/include");
    }

    cx.compile("cbfs-cxx");

    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            println!("cargo:rustc-link-lib=winfsp-a64");
        } else if cfg!(target_arch = "x86_64") {
            println!("cargo:rustc-link-lib=winfsp-x64");
        } else {
            panic!("unsupported windows architecture");
        }

        println!("cargo:rustc-link-lib=ws2_32");
        println!("cargo:rustc-link-lib=ntdll");
        println!("cargo:rustc-link-lib=userenv");

        println!(
            "cargo:rustc-link-search=native={}",
            fsp_base.join("lib").display()
        );
    } else {
        if cfg!(target_os = "macos") {
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
            println!("cargo:rustc-link-search=native=/usr/local/lib");
        } else if cfg!(target_os = "freebsd") {
            println!("cargo:rustc-link-lib=pthread");
            println!("cargo:rustc-link-search=native=/usr/local/lib");
        }

        println!("cargo:rustc-link-lib=fuse3");
    }

    println!("cargo:rerun-if-changed=src/cbfs_fuse.cpp");
    println!("cargo:rerun-if-changed=include/cbfs_fuse.hpp");
}
