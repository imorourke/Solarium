use std::path::Path;

use cblang::{CodeGenerationOptions, Compiler, ProgramType, compiler::KernelOptions};

fn main() {
    pub const BOOTLOADER_CODE: &str = include_str!("../../cbos/bootloader.cb");
    let compiler = Compiler {
        options: CodeGenerationOptions {
            prog_type: ProgramType::Kernel(KernelOptions {
                base_location: jib_computer_defs::BOOTLOADER_START_ADDR,
                ..Default::default()
            }),
            trim_code: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let bootloader = compiler
        .compile_string(BOOTLOADER_CODE, Some(Path::new("bootloader.cb")))
        .unwrap();

    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("bootloader.bin");

    std::fs::write(dest_path, bootloader.asm.bytes).unwrap();
}
