use std::path::Path;

use cbfs_lib::{CbContainerOptions, ContainerHeader, save_container};
use jib_os::JibOsImage;

fn main() {
    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("cbos.cbfs");
    let os_img = match JibOsImage::compile_os_image() {
        Ok(x) => x,
        Err(e) => {
            panic!("{e}");
        }
    };

    let hd = match os_img.create_hard_drive() {
        Ok(x) => x,
        Err(e) => panic!("{e}"),
    };

    save_container(
        &ContainerHeader::new(CbContainerOptions {
            sparse: true,
            compressed: false,
        }),
        &hd,
        &dest_path,
    )
    .unwrap();

    std::fs::write(
        Path::new(&out_dir).join("cbos.cb"),
        os_img.kernel_header.as_bytes(),
    )
    .unwrap();

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/lib.rs");
}
