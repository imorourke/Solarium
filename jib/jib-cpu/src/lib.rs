#![no_std]

pub mod cpu;
pub mod device;
pub mod memory;
pub mod text;

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod locations {
    pub const DEFAULT_START_ADDR: u32 = 0x2000;
    pub const DEFAULT_STACK_LOC: u32 = 0x1000;

    pub const INIT_MEMORY_SIZE: u32 = 0x40000000;
    pub const DEVICE_START_ADDR: u32 = 0xFFFFA000;
    pub const DEVICE_HD_START_ADDR: u32 = 0xFFFFB000;
    pub const DEVICE_COUNT: usize =
        ((DEVICE_HD_START_ADDR - DEVICE_START_ADDR) / super::device::DEVICE_MEM_SIZE) as usize;

    pub const BOOTLOADER_START_ADDR: u32 = 0xFFFF0000;
}
