//! Definition for the core CPU and basic devices for the Jib Processor

#![no_std]

pub mod cpu;
pub mod device;
pub mod memory;
pub mod text;

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;
