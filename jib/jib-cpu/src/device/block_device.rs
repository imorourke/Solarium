#[cfg(feature = "std")]
use alloc::string::String;
use alloc::{boxed::Box, vec::Vec};

use crate::{
    device::{DEVICE_BLOCK_MEM_SIZE, DeviceType, ProcessorDevice},
    memory::{MemorySegment, MemorySegmentError},
};

pub trait BlockDeviceBacking {
    fn set_window(&mut self, offset: usize, length: usize);
    fn get_window(&self) -> &[u8];
    fn set_in_window(&mut self, index: usize, value: u8);
    fn sync(&mut self) -> bool;
    fn get_data(&mut self) -> Vec<u8>;
}

pub struct MemoryBacking {
    pub data: Vec<u8>,
    pub offset: usize,
    pub length: usize,
}

impl BlockDeviceBacking for MemoryBacking {
    fn set_window(&mut self, offset: usize, length: usize) {
        self.offset = offset;
        self.length = length;
    }

    fn get_window(&self) -> &[u8] {
        if self.offset < self.data.len() {
            let window_bottom = self.offset;
            let window_top = (window_bottom + self.length).min(self.data.len());
            &self.data[window_bottom..window_top]
        } else {
            &[]
        }
    }

    fn set_in_window(&mut self, index: usize, value: u8) {
        if let Some(v) = self.data.get_mut(index + self.offset) {
            *v = value;
        }
    }

    fn sync(&mut self) -> bool {
        true
    }

    fn get_data(&mut self) -> Vec<u8> {
        self.data.clone()
    }
}

#[cfg(feature = "std")]
mod file {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Seek, Write};
    use std::path::Path;
    use std::vec;

    pub struct FileBacking {
        pub file: File,
        pub max_size: u64,
        pub base_offset: usize,
        pub max_written: usize,
        pub temp_data: Vec<u8>,
    }

    impl FileBacking {
        pub fn new(path: &Path) -> Self {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap();
            let size = f.metadata().unwrap().len();

            Self {
                file: f,
                max_size: size,
                base_offset: 0,
                max_written: 0,
                temp_data: Vec::default(),
            }
        }
    }

    impl BlockDeviceBacking for FileBacking {
        fn set_window(&mut self, offset: usize, length: usize) {
            if offset != self.base_offset {
                self.sync();
            }

            if offset < self.max_size as usize {
                let window_bottom = offset;
                let window_top = (window_bottom + length).min(self.max_size as usize);
                let temp_length = window_top - window_bottom;

                if self.base_offset != offset && temp_length != self.temp_data.len() {
                    self.temp_data = vec![0; temp_length];
                    self.file
                        .seek(std::io::SeekFrom::Start(offset as u64))
                        .unwrap();
                    self.file
                        .read_exact(&mut self.temp_data.as_mut_slice())
                        .unwrap();
                }
            }
        }

        fn get_window(&self) -> &[u8] {
            &self.temp_data
        }

        fn set_in_window(&mut self, index: usize, value: u8) {
            if let Some(v) = self.temp_data.get_mut(index) {
                *v = value;
                self.max_written = self.max_written.max(index);
            }
        }

        fn sync(&mut self) -> bool {
            if self.max_written > 0 {
                self.file
                    .seek(std::io::SeekFrom::Start(self.base_offset as u64))
                    .unwrap();
                self.file
                    .write_all(&self.temp_data[..self.max_written])
                    .unwrap();
                self.max_written = 0;
                true
            } else {
                false
            }
        }

        fn get_data(&mut self) -> Vec<u8> {
            self.file.seek(std::io::SeekFrom::Start(0)).unwrap();
            let mut data = Vec::new();
            self.file.read_to_end(&mut data).unwrap();
            data
        }
    }
}

pub struct BlockDevice {
    data: Box<dyn BlockDeviceBacking>,
    current_offset: u32,
    target_offset: u32,
    parked: bool,
    memory_size: u16,
    write_enabled: bool,
    read_only_set: bool,
}

impl BlockDevice {
    const CONTROL_SIZE: usize = 12;
    const WINDOW_SIZE_MAX: usize = 256;
    const DATA_TOP: usize = BlockDevice::CONTROL_SIZE + BlockDevice::WINDOW_SIZE_MAX;

    pub fn new<T: IntoIterator<Item = u8>>(data: T) -> Self {
        Self {
            data: Box::new(MemoryBacking {
                data: data.into_iter().collect(),
                offset: 0,
                length: 0,
            }),
            current_offset: 0,
            target_offset: 0,
            parked: true,
            memory_size: 0,
            write_enabled: false,
            read_only_set: false,
        }
    }

    #[cfg(feature = "std")]
    pub fn new_file(file: &std::path::Path) -> Result<Self, String> {
        use crate::device::block_device::file::FileBacking;

        Ok(Self {
            data: Box::new(FileBacking::new(file)),
            current_offset: 0,
            target_offset: 0,
            parked: true,
            memory_size: 0,
            write_enabled: false,
            read_only_set: false,
        })
    }

    pub fn set_offset(&mut self, target: u32) {
        self.target_offset = target;
        self.current_offset = target;
        self.data.set_window(target as usize, Self::WINDOW_SIZE_MAX);
    }

    pub fn sync(&mut self) -> bool {
        self.data.sync()
    }

    pub fn get_backing_data(&mut self) -> Vec<u8> {
        self.data.get_data()
    }

    fn get_data_window(&self) -> &[u8] {
        self.data.get_window()
    }

    fn reported_size(&self) -> u16 {
        let window_size = self.get_data_window().len() as u16;
        if self.memory_size != 0 {
            self.memory_size.min(window_size)
        } else {
            window_size
        }
    }

    fn can_write(&self) -> bool {
        self.write_enabled && !self.read_only_set
    }
}

impl Drop for BlockDevice {
    fn drop(&mut self) {
        self.sync();
    }
}

impl ProcessorDevice for BlockDevice {
    fn device_type(&self) -> DeviceType {
        DeviceType::BlockDevice
    }
}

// [2] DeviceID
// [1] Parked
// [1] Ready?
// [4] TargetOffset
// [1] MoveToOffset
// [1] WriteData
// [2] MemorySize (max 256)
// [12.. Data]

impl MemorySegment for BlockDevice {
    fn get(&self, offset: u32) -> Result<u8, MemorySegmentError> {
        let local_offset = offset as usize;

        match local_offset {
            0..2 => Ok(self.device_type().get_device_id().to_be_bytes()[local_offset]),
            2 => Ok(self.parked as u8),
            3 => Ok((self.current_offset == self.target_offset) as u8),
            4..8 => Ok(self.target_offset.to_be_bytes()[local_offset - 4]),
            8 => Ok(0),
            9 => Ok(if self.can_write() { 1 } else { 0 }),
            10..12 => Ok(self.reported_size().to_be_bytes()[local_offset - 10]),
            Self::CONTROL_SIZE..Self::DATA_TOP => {
                if let Some(v) = self
                    .data
                    .get_window()
                    .get(local_offset - Self::CONTROL_SIZE)
                {
                    Ok(*v)
                } else {
                    Ok(0)
                }
            }
            _ => Err(MemorySegmentError::InvalidMemoryAccess(offset)),
        }
    }

    fn set(&mut self, offset: u32, val: u8) -> Result<(), MemorySegmentError> {
        let local_offset = offset as usize;

        match local_offset {
            0..2 => Err(MemorySegmentError::InvalidMemoryAccess(offset)),
            2 => {
                self.parked = val == 0;
                Ok(())
            }
            3 => Err(MemorySegmentError::InvalidMemoryAccess(offset)),
            4..8 => {
                let mut bytes = self.target_offset.to_be_bytes();
                bytes[local_offset - 4] = val;
                self.target_offset = u32::from_be_bytes(bytes);
                Ok(())
            }
            8 => {
                self.set_offset(self.target_offset);
                Ok(())
            }
            9 => {
                self.write_enabled = val != 0;
                Ok(())
            }
            10..12 => {
                let mut temp_array = self.memory_size.to_be_bytes();
                temp_array[local_offset - 10] = val;
                self.memory_size = u16::from_be_bytes(temp_array);
                Ok(())
            }
            Self::CONTROL_SIZE..Self::DATA_TOP => {
                if self.can_write() {
                    self.data
                        .set_in_window(local_offset - Self::CONTROL_SIZE, val);
                }
                Ok(())
            }
            _ => Err(MemorySegmentError::InvalidMemoryAccess(offset)),
        }
    }

    fn len(&self) -> u32 {
        DEVICE_BLOCK_MEM_SIZE
    }

    fn reset(&mut self) {
        self.set_offset(0);
    }
}
