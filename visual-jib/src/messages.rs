use jib_computer::{ComputerPram, JibCode};
use jib_cpu::cpu::RegisterManager;

#[derive(Debug, Clone)]
pub enum UiToThread {
    CpuStep,
    CpuRun(bool),
    UseBootloader(bool),
    CpuReset,
    DiskReset,
    #[cfg(not(target_arch = "wasm32"))]
    DiskSave,
    CpuIrq(u8),
    SetCode(JibCode),
    SerialInput(String),
    RequestMemory(u32, u32),
    SetMultiplier(i32),
    SetPramSettings(ComputerPram),
    #[cfg(not(target_arch = "wasm32"))]
    Exit,
}

#[derive(Debug, Clone)]
pub enum ThreadToUi {
    ResponseMemory(u32, Vec<u8>),
    SerialOutput(String),
    LogMessage(String),
    RegisterState(Box<RegisterManager>),
    ProgramCounterValue(u32, u32),
    ProcessorReset,
    #[cfg(not(target_arch = "wasm32"))]
    ThreadExit,
    CpuRunning(bool),
    BootloaderState(bool),
    PramSettings(ComputerPram),
}
