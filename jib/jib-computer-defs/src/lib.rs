//! Provides definitions for common location used in jib computers

/// The starting location for typical programs
pub const START_ADDR: u32 = 0x2000;

/// The stack location for kernel programs
pub const STACK_ADDR: u32 = 0x1000;

/// Overall memory size for the coomputer
pub const MEMORY_SIZE: u32 = 0x40000000;

/// Starting address for standard-sized devices
pub const DEVICE_START_ADDR: u32 = 0xFFFFA000;

/// The number of standard devices to support
pub const DEVICE_COUNT: usize =
    ((DEVICE_HD_START_ADDR - DEVICE_START_ADDR) / jib_cpu::device::DEVICE_MEM_SIZE) as usize;

/// Starting address for block/disk devices
pub const DEVICE_HD_START_ADDR: u32 = 0xFFFFB000;

/// Bootloader starding location
pub const BOOTLOADER_START_ADDR: u32 = 0xFFFF0000;
