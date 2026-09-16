use chrono::{Datelike, Timelike};

use crate::{
    device::{DEVICE_ID_SIZE, DEVICE_MEM_SIZE, DeviceType, ProcessorDevice},
    memory::{MemorySegment, MemorySegmentError},
};

extern crate std;

pub struct RtcClockDevice {
    reset_time: chrono::DateTime<chrono::Utc>,
}

impl Default for RtcClockDevice {
    fn default() -> Self {
        Self {
            reset_time: chrono::Utc::now(),
        }
    }
}

impl MemorySegment for RtcClockDevice {
    fn get(&self, offset: u32) -> Result<u8, MemorySegmentError> {
        self.inspect(offset)
    }

    fn inspect(&self, offset: u32) -> Result<u8, MemorySegmentError> {
        if offset < DEVICE_ID_SIZE {
            Ok(self.device_type().get_device_id().to_be_bytes()[offset as usize])
        } else {
            let index = offset - DEVICE_ID_SIZE;
            let time = chrono::Utc::now();
            let elapsed = (time - self.reset_time).num_milliseconds().max(0) as u32;
            match index {
                0 => Ok((time.year() as i16).to_be_bytes()[0]),
                1 => Ok((time.year() as i16).to_be_bytes()[1]),
                2 => Ok(time.month() as u8),
                3 => Ok(time.day0() as u8),
                4 => Ok(time.hour() as u8),
                5 => Ok(time.minute() as u8),
                6 => Ok(time.second() as u8),
                7 => Ok((time.timestamp_millis() / 10) as u8),
                8..12 => Ok(elapsed.to_be_bytes()[index as usize - 8]),
                _ => Ok(0),
            }
        }
    }

    fn set(&mut self, offset: u32, _val: u8) -> Result<(), MemorySegmentError> {
        Err(MemorySegmentError::ReadOnlyMemory(offset))
    }

    fn reset(&mut self) {
        self.reset_time = chrono::Utc::now();
    }

    fn len(&self) -> u32 {
        DEVICE_MEM_SIZE
    }
}

impl ProcessorDevice for RtcClockDevice {
    fn device_type(&self) -> DeviceType {
        DeviceType::RtcClock
    }
}
