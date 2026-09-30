fn main() {
    let mut cx_bridge = cxx_build::bridge("src/main.rs");
    let mut cx = cx_bridge
        .file("src/cbfs_fuse.cpp")
        .include("include")
        .std("c++20")
        .define("_FILE_OFFSET_BITS", "64")
        .define("FUSE_USE_VERSION", "35");

    cfg_select! {
        target_os = "windows" => {
            cx = cx
                .include("C:\\Program Files (x86)\\WinFsp\\inc")
                .define("WIN32", "1");

            if cfg!(target_arch = "aarch64") {
                println!("cargo:rustc-link-lib=dylib=winfsp-a64");
            } else if cfg!(target_arch = "x86_64") {
                println!("cargo:rustc-link-lib=dylib=winfsp-x64");
            } else {
                panic!("unsupported windows architecture");
            }

            println!("cargo:rustc-link-lib=dylib=ws2_32");
            println!("cargo:rustc-link-lib=dylib=ntdll");
            println!("cargo:rustc-link-lib=dylib=userenv");

            println!("cargo:rustc-link-search=native=C:\\Program Files (x86)\\WinFsp\\lib") ;
        }
        _ => {
            if cfg!(target_os = "macos") {
                println!("cargo:rustc-link-lib=framework=CoreFoundation");
            } else if cfg!(target_os = "freebsd") {
                println!("cargo:rustc-link-lib=dylib=pthread");
                println!("cargo:rustc-link-lib=native=/usr/local/lib");
                cx = cx.include("/usr/local/include");
            }

            println!("cargo:rustc-link-lib=dylib=fuse3");
        }
    }

    cx.compile("cbfs-cxx");

    println!("cargo:rerun-if-changed=src/cbfs_fuse.cpp");
    println!("cargo:rerun-if-changed=include/cbfs_fuse.hpp");
}
