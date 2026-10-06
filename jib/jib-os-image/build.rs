use std::path::Path;

use cbfs_lib::{CbContainerOptions, ContainerHeader, save_container};
use jib_os::JibOsImage;

fn main() {
    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("cbos.cbfs");
    let os_img = JibOsImage::compile_os_image().unwrap();

    save_container(
        &ContainerHeader::new(CbContainerOptions {
            sparse: true,
            compressed: false,
        }),
        &os_img.create_hard_drive().unwrap(),
        &dest_path,
    )
    .unwrap();

    std::fs::write(
        &Path::new(&out_dir).join("cbos.cb"),
        os_img.kernel_header.as_bytes(),
    )
    .unwrap();

    println!("cargo::rerun-if-changed=build.rs");
}
