mod jibos;

use cbfs_lib::{FileSystem, FileSystemError};
use cblang::{CompilerError, ProgramType, TokenError, preprocessor::PreprocessorError};
use circ_buff::CircularBuffer;
use core::{cell::RefCell, fmt::Display};
use jib_asm::{AssemblerError, AssemblerErrorLoc, AssemblerOutput};
#[cfg(not(target_arch = "wasm32"))]
use jib_cpu::device::RtcTimerDevice;
use jib_cpu::{
    cpu::{Instruction, Opcode, Processor, ProcessorError, RegisterManager, ResetType},
    device::{
        BlankDevice, BlockDevice, DEVICE_MEM_SIZE, InterruptClockDevice, ProcessorDevice,
        RtcClockDevice, SerialInputOutputDevice,
    },
    memory::{MemorySegment, ReadOnlySegment, ReadWriteSegment},
    text::{CharacterError, byte_to_character, character_to_byte},
};
use std::{rc::Rc, vec::Vec};

pub use jibos::JibOsImage;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JibCode {
    pub start_location: u32,
    pub code: Vec<u8>,
}

impl From<AssemblerOutput> for JibCode {
    fn from(value: AssemblerOutput) -> Self {
        Self {
            start_location: value.start_address,
            code: value.bytes,
        }
    }
}

pub struct JibComputer {
    running: bool,
    running_requested: bool,
    bootloader: bool,
    cpu: Processor,
    os_image: JibOsImage,
    dev_serial_io: Rc<RefCell<SerialInputOutputDevice>>,
    #[cfg(not(target_arch = "wasm32"))]
    dev_rtc_timer: Rc<RefCell<RtcTimerDevice>>,
    inst_history: CircularBuffer<(u32, Instruction), 10>,
    hard_drive: Rc<RefCell<BlockDevice>>,
    #[cfg(test)]
    step_count: u128,
    pub step_callback: Option<Box<dyn Fn(RegisterManager, Instruction)>>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopMode {
    pub debug: bool,
    pub cancel_run: bool,
}

impl JibComputer {
    const INIT_MEMORY_SIZE: u32 = 0x40000000;
    pub const BOOTLOADER_START: u32 = 0xFFFF0000;
    const DEVICE_START_ADDR: u32 = 0xFFFFA000;
    const DEVICE_HD_START_ADDR: u32 = 0xFFFFB000;
    const DEVICE_COUNT: usize =
        ((Self::DEVICE_HD_START_ADDR - Self::DEVICE_START_ADDR) / DEVICE_MEM_SIZE) as usize;
    pub const THREAD_LOOP_MS: u64 = 50;

    pub fn new() -> Result<Self, ComputerError> {
        let os_image = JibOsImage::compile_os_image()?;
        let hd = os_image.create_hard_drive()?;
        let mut s = Self {
            running: false,
            running_requested: false,
            bootloader: false,
            os_image,
            cpu: Processor::default(),
            dev_serial_io: Rc::new(RefCell::new(SerialInputOutputDevice::new(2048))),
            #[cfg(not(target_arch = "wasm32"))]
            dev_rtc_timer: Rc::new(RefCell::new(RtcTimerDevice::default())),
            inst_history: Default::default(),
            hard_drive: Self::create_block_device(&hd)?,
            #[cfg(test)]
            step_count: 0,
            step_callback: None,
        };

        s.reset(None)?;
        Ok(s)
    }

    fn create_block_device(fs: &FileSystem) -> Result<Rc<RefCell<BlockDevice>>, ComputerError> {
        Ok(Rc::new(RefCell::new(BlockDevice::new(fs.get_fs_bytes()?))))
    }

    pub fn get_inst_history(&self) -> Vec<(u32, Instruction)> {
        self.inst_history.elts()
    }

    pub fn get_register_state(&self) -> RegisterManager {
        self.cpu.get_register_state()
    }

    pub fn memory_inspect_u8(&self, addr: u32) -> Result<u8, ComputerError> {
        Ok(self.cpu.memory_inspect(addr)?)
    }

    pub fn memory_inspect_u32(&self, addr: u32) -> Result<u32, ComputerError> {
        Ok(self.cpu.memory_inspect_u32(addr)?)
    }

    pub fn current_operation(&self) -> Result<Opcode, ComputerError> {
        Ok(self.cpu.get_current_op()?)
    }

    pub fn step_cpu(
        &mut self,
        breakpoint: Option<u32>,
        stop_mode: Option<StopMode>,
    ) -> Result<bool, ComputerError> {
        let pc = self.cpu.get_current_pc().unwrap_or(0);
        if let Ok(inst) = self.cpu.get_current_inst() {
            self.inst_history.push((pc, inst));
        }

        if let Some(auto_stop) = stop_mode {
            let (debug_stop, mut cancel_run_request) = if let Ok(stop_normal) =
                self.cpu.should_stop()
                && let Ok(stop_debug) = self.cpu.should_stop_debug()
            {
                let dbg = stop_debug && auto_stop.debug;
                (stop_normal || dbg, dbg)
            } else if let Some(brk) = breakpoint {
                (brk == pc && auto_stop.debug, true)
            } else {
                (false, true)
            };

            cancel_run_request &= auto_stop.cancel_run;

            if debug_stop {
                self.running = false;
                self.running_requested = self.running_requested && !cancel_run_request;
                return Ok(false);
            }
        }

        self.cpu.step()?;

        if let Some(func) = &self.step_callback {
            func(self.cpu.get_register_state(), self.cpu.get_current_inst()?);
        }

        #[cfg(test)]
        {
            self.step_count += 1;
        }

        Ok(true)
    }

    pub fn soft_reset(&mut self) -> Result<(), ComputerError> {
        self.cpu.reset(ResetType::Soft)?;
        Ok(())
    }

    pub fn reset(&mut self, input_code: Option<&JibCode>) -> Result<(), ComputerError> {
        const INIT_RO_LEN: u32 = Processor::BASE_HW_INT_ADDR;

        self.cpu = Processor::default();
        self.dev_serial_io.borrow_mut().reset();
        self.hard_drive = Self::create_block_device(&self.os_image.create_hard_drive()?)?;

        self.inst_history.clear();

        #[cfg(test)]
        {
            self.step_count = 0;
        }

        let mut reset_vec_data: Vec<u8> = vec![0; INIT_RO_LEN as usize];
        let start_loc = if self.bootloader {
            Self::BOOTLOADER_START
        } else {
            ProgramType::DEFAULT_START_OFFSET
        };

        for (i, x) in start_loc.to_be_bytes().iter().enumerate() {
            reset_vec_data[i] = *x;
            reset_vec_data[i + std::mem::size_of::<u32>()] = *x;
        }

        assert!(reset_vec_data.len() == INIT_RO_LEN as usize);

        self.cpu.memory_add_segment(
            0,
            Rc::new(RefCell::new(ReadOnlySegment::new(reset_vec_data))),
        )?;

        self.cpu.memory_add_segment(
            INIT_RO_LEN,
            Rc::new(RefCell::new(ReadWriteSegment::new(
                (Self::INIT_MEMORY_SIZE - INIT_RO_LEN) as usize,
            ))),
        )?;

        self.cpu.memory_add_segment(
            Self::BOOTLOADER_START,
            Rc::new(RefCell::new(ReadWriteSegment::new(
                (Self::DEVICE_START_ADDR - Self::BOOTLOADER_START) as usize,
            ))),
        )?;

        let blank_dev = Rc::new(RefCell::new(BlankDevice));
        let devices: [Rc<RefCell<dyn ProcessorDevice>>; _] = [
            self.dev_serial_io.clone(),
            Rc::new(RefCell::new(InterruptClockDevice::default())),
            Rc::new(RefCell::new(RtcClockDevice)),
            #[cfg(not(target_arch = "wasm32"))]
            self.dev_rtc_timer.clone(),
        ];

        for i in 0..Self::DEVICE_COUNT {
            let dev_loc = Self::DEVICE_START_ADDR + (i as u32) * DEVICE_MEM_SIZE;

            let dev = if let Some(d) = devices.get(i) {
                self.cpu.device_add(d.clone())?;
                d.clone()
            } else {
                blank_dev.clone()
            };

            self.cpu.memory_add_segment(dev_loc, dev)?;
        }

        self.cpu.device_add(self.hard_drive.clone())?;
        self.cpu
            .memory_add_segment(Self::DEVICE_HD_START_ADDR, self.hard_drive.clone())?;

        self.cpu.reset(ResetType::Hard)?;

        // Compile and setup bootloader
        for (i, x) in JibOsImage::compile_kernel_code(
            include_str!("../../../cbos/bootloader.cb"),
            "bootloader.cb",
            Some(Self::BOOTLOADER_START),
            true,
        )?
        .asm
        .bytes
        .iter()
        .enumerate()
        {
            self.cpu.memory_set(Self::BOOTLOADER_START + i as u32, *x)?;
        }

        if !self.bootloader
            && let Some(code) = input_code
        {
            self.cpu.memory_set_range(code.start_location, &code.code)?;
        }

        if self.running_requested {
            self.running = true;
        }

        if let Some(func) = &self.step_callback {
            func(self.cpu.get_register_state(), self.cpu.get_current_inst()?);
        }

        Ok(())
    }

    pub fn step_devices(&mut self) -> Result<bool, ComputerError> {
        Ok(if self.cpu.step_devices()? {
            self.running = true;
            true
        } else {
            false
        })
    }

    pub fn use_bootloader(&mut self, value: bool) -> Result<(), ComputerError> {
        self.bootloader = value;
        Ok(())
    }

    pub fn using_bootloader(&self) -> bool {
        self.bootloader
    }

    pub fn trigger_irq(&mut self, irq: u32) -> Result<bool, ComputerError> {
        if self.cpu.trigger_hardware_interrupt(irq)? {
            if self.running_requested && !self.running && self.cpu.step_devices()? {
                self.running = true;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn set_running_request(&mut self, running_requested: bool) -> bool {
        if running_requested != self.running_requested {
            self.running = running_requested;
            self.running_requested = running_requested;
            true
        } else {
            false
        }
    }

    pub fn get_running(&self) -> bool {
        self.running
    }

    pub fn get_running_requested(&self) -> bool {
        self.running_requested
    }

    pub fn set_code(&mut self, code: &JibCode) -> Result<(), ComputerError> {
        self.reset(Some(code))
    }

    pub fn pub_serial_byte(&mut self, b: u8) -> Result<bool, ComputerError> {
        if !self.dev_serial_io.borrow_mut().push_input(b) {
            Ok(false)
        } else if self.running_requested && !self.running && self.cpu.step_devices()? {
            self.running = true;
            Ok(true)
        } else {
            Ok(true)
        }
    }

    fn set_serial_input_inner<const ALLOW_UNKNOWN: bool>(
        &mut self,
        s: &str,
    ) -> Result<bool, ComputerError> {
        for c in s.chars().chain(['\n']) {
            let cv = if ALLOW_UNKNOWN {
                match character_to_byte(c) {
                    Ok(v) => v,
                    Err(CharacterError::CharacterToByte(_)) => b'?',
                    Err(e) => return Err(e.into()),
                }
            } else {
                character_to_byte(c)?
            };

            if !self.pub_serial_byte(cv)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn set_serial_input(&mut self, s: &str) -> Result<bool, ComputerError> {
        self.set_serial_input_inner::<false>(s)
    }

    pub fn set_serial_input_unknown(&mut self, s: &str) -> Result<bool, ComputerError> {
        self.set_serial_input_inner::<true>(s)
    }

    pub fn get_serial_output(&mut self) -> Result<Vec<char>, ComputerError> {
        let mut char_vec = Vec::new();
        while let Some(w) = self.dev_serial_io.borrow_mut().pop_output() {
            char_vec.push(byte_to_character(w)?);
        }
        Ok(char_vec)
    }

    pub fn get_serial_output_unknown(&mut self) -> Vec<char> {
        let mut char_vec = Vec::new();
        while let Some(w) = self.dev_serial_io.borrow_mut().pop_output() {
            char_vec.push(byte_to_character(w).unwrap_or('?'));
        }
        char_vec
    }

    pub fn get_disk_data(&self) -> Result<Vec<u8>, ComputerError> {
        Ok(self.hard_drive.borrow().data.clone())
    }
}

#[derive(Debug)]
pub enum ComputerError {
    ProcessorError(ProcessorError),
    AssemblerError(AssemblerError),
    AssemblerErrorLoc(AssemblerErrorLoc),
    DiskError(FileSystemError),
    TokenError(TokenError),
    PreprocessorError(PreprocessorError),
    CharacterError(CharacterError),
    FilesystemError(cblang::preprocessor::FilesystemError),
    Utf8Error,
    IoError(std::io::Error),
}

impl Display for ComputerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreprocessorError(e) => write!(f, "preprocessor => {e}"),
            Self::AssemblerError(e) => write!(f, "assembler => {e}"),
            Self::AssemblerErrorLoc(e) => write!(f, "assembler => {e}"),
            Self::DiskError(e) => write!(f, "disk => {e}"),
            Self::TokenError(e) => write!(f, "token => {e}"),
            Self::ProcessorError(e) => write!(f, "processor => {e}"),
            Self::CharacterError(e) => write!(f, "character => {e}"),
            Self::FilesystemError(e) => write!(f, "filesystem => {e}"),
            Self::Utf8Error => write!(f, "utf8 error"),
            Self::IoError(e) => write!(f, "io error => {e}"),
        }
    }
}

impl From<CompilerError> for ComputerError {
    fn from(value: CompilerError) -> Self {
        match value {
            CompilerError::AssemblerError(v) => Self::AssemblerErrorLoc(v),
            CompilerError::TokenError(v) => Self::TokenError(v),
            CompilerError::TokenErrorFancy(v, _) => Self::TokenError(v),
            CompilerError::IoError(v) => Self::IoError(v),
            CompilerError::PreprocessorError(v) => Self::PreprocessorError(v),
        }
    }
}

impl From<ProcessorError> for ComputerError {
    fn from(value: ProcessorError) -> Self {
        Self::ProcessorError(value)
    }
}

impl From<AssemblerError> for ComputerError {
    fn from(value: AssemblerError) -> Self {
        Self::AssemblerError(value)
    }
}

impl From<AssemblerErrorLoc> for ComputerError {
    fn from(value: AssemblerErrorLoc) -> Self {
        Self::AssemblerErrorLoc(value)
    }
}

impl From<TokenError> for ComputerError {
    fn from(value: TokenError) -> Self {
        Self::TokenError(value)
    }
}

impl From<PreprocessorError> for ComputerError {
    fn from(value: PreprocessorError) -> Self {
        Self::PreprocessorError(value)
    }
}

impl From<FileSystemError> for ComputerError {
    fn from(value: FileSystemError) -> Self {
        Self::DiskError(value)
    }
}

impl From<CharacterError> for ComputerError {
    fn from(value: CharacterError) -> Self {
        Self::CharacterError(value)
    }
}

impl From<cblang::preprocessor::FilesystemError> for ComputerError {
    fn from(value: cblang::preprocessor::FilesystemError) -> Self {
        Self::FilesystemError(value)
    }
}

impl From<std::io::Error> for ComputerError {
    fn from(value: std::io::Error) -> Self {
        Self::IoError(value)
    }
}

#[cfg(test)]
mod test {
    use super::JibComputer;
    use crate::{JibCode, JibOsImage};

    fn run_cpu_serial_out_test(in_code: &str, expected_out: &str) {
        let asm = JibOsImage::compile_kernel_code(in_code, "input.cb", None, true)
            .unwrap()
            .asm;

        let mut cpu = JibComputer::new().unwrap();
        cpu.set_code(&JibCode {
            start_location: asm.start_address,
            code: asm.bytes,
        })
        .unwrap();

        let mut serial_output = Vec::new();
        let mut iter_count = 0;

        while !cpu.cpu.should_stop().unwrap() {
            cpu.step_cpu(None, None).unwrap();
            while let Some(c) = cpu.dev_serial_io.borrow_mut().pop_output() {
                serial_output.push(c);
            }
            iter_count += 1;
            assert!(iter_count < 40000);
        }

        println!("Step Count: {}", cpu.step_count);
        println!("{}", str::from_utf8(&serial_output).unwrap());

        assert_eq!(
            str::from_utf8(&serial_output).unwrap(),
            expected_out.replace("\r\n", "\n")
        );
    }

    #[test]
    fn test_malloc() {
        run_cpu_serial_out_test(
            include_str!("../../../cbuoy/cblang/tests/test_kmalloc.cb"),
            include_str!("../../../cbuoy/cblang/tests/test_kmalloc.out"),
        );
    }

    #[test]
    fn test_struct_ptr() {
        run_cpu_serial_out_test(
            include_str!("../../../cbuoy/cblang/tests/test_struct_ptr.cb"),
            include_str!("../../../cbuoy/cblang/tests/test_struct_ptr.out"),
        );
    }

    #[test]
    fn test_struct_func() {
        run_cpu_serial_out_test(
            include_str!("../../../cbuoy/cblang/tests/test_struct_func.cb"),
            include_str!("../../../cbuoy/cblang/tests/test_struct_func.out"),
        );
    }

    #[test]
    fn test_math() {
        run_cpu_serial_out_test(
            include_str!("../../../cbuoy/cblang/tests/test_math.cb"),
            include_str!("../../../cbuoy/cblang/tests/test_math.out"),
        );
    }
}
