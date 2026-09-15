use jib_cpu::{
    device::{DEVICE_MEM_SIZE, ProcessorDevice},
    memory::{MemorySegment, MemorySegmentError},
};

#[derive(Default, Debug, Clone, Copy)]
pub struct ComputerPram {
    pub boot_debug: bool,
}

impl MemorySegment for ComputerPram {
    fn get(&self, offset: u32) -> Result<u8, MemorySegmentError> {
        Ok(match offset {
            0..2 => self.device_type().get_device_id().to_be_bytes()[offset as usize],
            2 if self.boot_debug => 1,
            _ => 0,
        })
    }

    fn set(&mut self, offset: u32, val: u8) -> Result<(), MemorySegmentError> {
        match offset {
            2 => {
                self.boot_debug = val != 0;
                Ok(())
            }
            _ => Err(MemorySegmentError::ReadOnlyMemory(offset)),
        }
    }

    fn len(&self) -> u32 {
        DEVICE_MEM_SIZE
    }

    fn reset(&mut self) {
        // Do Nothing
    }
}

impl ProcessorDevice for ComputerPram {
    fn device_type(&self) -> jib_cpu::device::DeviceType {
        jib_cpu::device::DeviceType::Custom(100)
    }
}
