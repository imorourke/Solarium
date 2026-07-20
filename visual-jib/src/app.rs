use crate::cpu_thread::CpuState;
use crate::messages::{ThreadToUi, UiToThread};
use cblang::{
    CodeGenerationOptions, CompilerError, CompilingState, PreprocessorOutput, TokenError,
};
use eframe::egui::{
    self, CentralPanel, Context, Grid, Id, MenuBar, ScrollArea, Slider, TextBuffer, TextEdit,
};
use jib_asm::{AssemblerErrorLoc, InstructionList};
use jib_computer::JibCode;
use jib_cpu::cpu::RegisterManager;
use std::{path::PathBuf, sync::LazyLock, time::Duration};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ProgramCounterView {
    pc: u32,
    val: u32,
}

impl ProgramCounterView {
    fn get_instruction_string(&self) -> String {
        static INSTRUCTIONS: LazyLock<InstructionList> = LazyLock::new(InstructionList::default);

        if let Some(disp_val) = INSTRUCTIONS.get_display_inst(self.val.into()) {
            disp_val
        } else {
            "??".into()
        }
    }
}

#[derive(Debug, Clone)]
struct MemoryViewWindow {
    base: u32,
    base_str: String,
    values: [Option<u8>; Self::SIZE],
    memory_id: Id,
    shown: bool,
}

impl MemoryViewWindow {
    const COLUMNS: usize = 8;
    const ROWS: usize = 6;
    const SIZE: usize = MemoryViewWindow::COLUMNS * MemoryViewWindow::ROWS;

    fn new(id: usize) -> Self {
        Self {
            base: 0,
            base_str: "0".into(),
            values: [None; Self::SIZE],
            memory_id: Id::from(format!("mem{id}")),
            shown: true,
        }
    }

    fn draw(&mut self, ctx: &Context) {
        let mut opened = self.shown;
        egui::Window::new("Memory")
            .open(&mut opened)
            .resizable(false)
            .id(self.memory_id)
            .show(ctx, |ui| {
                ui.label("Base Address");
                if ui
                    .text_edit_singleline(&mut self.base_str)
                    .on_hover_text_at_pointer("The base address for the memory view")
                    .lost_focus()
                {
                    if let Some(rest) = self.base_str.strip_prefix("0x")
                        && let Ok(num) = u32::from_str_radix(rest, 16)
                    {
                        self.base = num;
                    } else if let Ok(num) = self.base_str.parse::<u32>() {
                        self.base = num;
                    }

                    self.base_str = format!("{:#x}", self.base);
                }
                Grid::new("memory_view")
                    .striped(true)
                    .num_columns(Self::COLUMNS)
                    .show(ui, |ui| {
                        for (i, v) in self.values.iter().enumerate() {
                            if i % MemoryViewWindow::COLUMNS == 0 {
                                if i > 0 {
                                    ui.end_row();
                                }

                                ui.label(format!("{:#010x}", self.base + i as u32));
                            }

                            if let Some(b) = v {
                                ui.label(format!("{b:02x}"));
                            } else {
                                ui.label("??");
                            }
                        }
                    });
            });
        self.shown = opened;
    }
}

pub struct VisualJib {
    log_serial: String,
    log_text: String,
    text_serial_input: String,
    cpu_run_requested: bool,
    #[cfg(not(target_arch = "wasm32"))]
    cpu_thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(target_arch = "wasm32")]
    cpu_state: CpuState,
    tx_ui: std::sync::mpsc::Sender<UiToThread>,
    tx_thread: std::sync::mpsc::Sender<ThreadToUi>,
    rx_ui: std::sync::mpsc::Receiver<ThreadToUi>,
    tx_window: std::sync::mpsc::Sender<CodeWindowAction>,
    rx_window: std::sync::mpsc::Receiver<CodeWindowAction>,
    registers: RegisterManager,
    program_counter: ProgramCounterView,
    current_cpu_speed: i32,
    code_windows: Vec<CodeWindow>,
    code_window_id: usize,
    memory_windows: Vec<MemoryViewWindow>,
    memory_window_id: usize,
    use_bootloader: bool,
}

impl Default for VisualJib {
    fn default() -> Self {
        let (tx_ui, rx_thread) = std::sync::mpsc::channel::<UiToThread>();
        let (tx_thread, rx_ui) = std::sync::mpsc::channel::<ThreadToUi>();
        let tx_thread_local = tx_thread.clone();
        let (tx_window, rx_window) = std::sync::mpsc::channel::<CodeWindowAction>();

        CodeWindow::new(
            0,
            tx_ui.clone(),
            tx_thread.clone(),
            tx_window.clone(),
            include_str!("../../cbos/os.cb").to_string(),
            CodeWindowType::Cbuoy,
            None,
        )
        .compile_cbuoy();

        let window = Self {
            cpu_run_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            cpu_thread: Some(std::thread::spawn(move || {
                crate::cpu_thread::cpu_thread(rx_thread, tx_thread)
            })),
            log_serial: String::default(),
            log_text: String::default(),
            text_serial_input: String::default(),
            #[cfg(target_arch = "wasm32")]
            cpu_state: CpuState::new(rx_thread, tx_thread).unwrap(),
            tx_ui,
            rx_ui,
            tx_thread: tx_thread_local,
            tx_window,
            rx_window,
            registers: RegisterManager::default(),
            program_counter: ProgramCounterView::default(),
            current_cpu_speed: 10.clamp(Self::SPEED_MIN, Self::SPEED_MAX),
            code_windows: Vec::new(),
            code_window_id: 0,
            memory_windows: Vec::new(),
            memory_window_id: 0,
            use_bootloader: false,
        };

        window
            .tx_ui
            .send(UiToThread::SetMultiplier(window.current_cpu_speed))
            .unwrap();
        window
            .tx_ui
            .send(UiToThread::UseBootloader(window.use_bootloader))
            .unwrap();
        window.tx_ui.send(UiToThread::CpuRun(true)).unwrap();

        window
    }
}

impl VisualJib {
    const SPEED_MIN: i32 = 0;
    const SPEED_MAX: i32 = 15;

    #[cfg(not(target_arch = "wasm32"))]
    pub fn name() -> &'static str {
        "VisualJib"
    }

    fn read_cpu_responses(&mut self) {
        while let Ok(msg) = self.rx_ui.try_recv() {
            match msg {
                ThreadToUi::ProcessorReset => {
                    self.log_serial.clear();
                }
                ThreadToUi::RegisterState(regs) => {
                    self.registers = *regs;
                }
                ThreadToUi::ProgramCounterValue(pc, val) => {
                    self.program_counter.pc = pc;
                    self.program_counter.val = val;
                }
                ThreadToUi::LogMessage(msg) => {
                    if !self.log_text.is_empty() {
                        self.log_text = format!("{}\n{}", self.log_text, msg);
                    } else {
                        self.log_text = msg;
                    }
                }
                ThreadToUi::SerialOutput(msg) => {
                    self.log_serial = format!("{}{}", self.log_serial, msg);
                }
                ThreadToUi::ResponseMemory(base, vals) => {
                    for mem in self.memory_windows.iter_mut() {
                        if mem.base == base {
                            for (i, v) in mem.values.iter_mut().enumerate() {
                                if i < vals.len() {
                                    *v = Some(vals[i]);
                                } else {
                                    *v = None;
                                }
                            }
                        }
                    }
                }
                ThreadToUi::CpuRunning(running) => self.cpu_run_requested = running,
                ThreadToUi::BootloaderState(bootloader) => self.use_bootloader = bootloader,
                #[cfg(not(target_arch = "wasm32"))]
                ThreadToUi::ThreadExit => std::process::exit(1),
            };
        }

        for mem in self.memory_windows.iter() {
            self.tx_ui
                .send(UiToThread::RequestMemory(mem.base, mem.values.len() as u32))
                .unwrap();
        }

        #[cfg(target_arch = "wasm32")]
        self.cpu_state.process_messages().unwrap();
    }

    fn update_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(if self.cpu_run_requested {
            CpuState::THREAD_LOOP_MS
        } else {
            1000
        }))
    }

    fn open_code_window(
        &mut self,
        code_type: CodeWindowType,
        code: String,
        filepath: Option<&'static str>,
    ) {
        self.code_windows.push(CodeWindow::new(
            self.code_window_id,
            self.tx_ui.clone(),
            self.tx_thread.clone(),
            self.tx_window.clone(),
            code,
            code_type,
            filepath,
        ));
        self.code_window_id += 1;
    }

    fn open_memory_window(&mut self) {
        self.memory_windows
            .push(MemoryViewWindow::new(self.memory_window_id));
        self.memory_window_id += 1;
    }
}

impl eframe::App for VisualJib {
    fn on_exit(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.tx_ui.send(UiToThread::Exit).is_ok() {
            self.cpu_thread.take().map(|x| x.join());
        } else {
            std::process::exit(1);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.read_cpu_responses();

        // Remove old windows
        {
            let mut i = 0;
            while i < self.code_windows.len() {
                if !self.code_windows[i].shown {
                    self.code_windows.swap_remove(i);
                } else {
                    i += 1;
                }
            }
        }

        {
            let mut i = 0;
            while i < self.memory_windows.len() {
                if !self.memory_windows[i].shown {
                    self.memory_windows.swap_remove(i);
                } else {
                    i += 1;
                }
            }
        }

        while let Ok(msg) = self.rx_window.try_recv() {
            match msg {
                CodeWindowAction::NewAssemblyWindow(code, filename) => {
                    self.open_code_window(CodeWindowType::Assembly, code, filename);
                }
            }
        }

        CentralPanel::default().show(ui, |ui| {
            MenuBar::new().ui(ui, |ui| {
                #[cfg(not(target_arch = "wasm32"))]
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("C/Buoy", |ui| {
                    if ui.button("New").clicked() {
                        self.open_code_window(CodeWindowType::Cbuoy, String::new(), None);
                    }

                    if ui.button("CB/OS").clicked() {
                        self.open_code_window(
                            CodeWindowType::Cbuoy,
                            include_str!("../../cbos/os.cb").to_string(),
                            Some("os.cb"),
                        );
                    }

                    if ui.button("Bootloader").clicked() {
                        self.open_code_window(
                            CodeWindowType::Cbuoy,
                            include_str!("../../cbos/bootloader.cb").into(),
                            Some("bootloader.cb"),
                        );
                    }

                    ui.menu_button("Applications", |ui| {
                        if ui.button("Hello World").clicked() {
                            self.open_code_window(
                                CodeWindowType::Cbuoy,
                                include_str!("../../cbos/bin/hello.cb").to_string(),
                                Some("hello.cb"),
                            );
                        }

                        if ui.button("Hello World (malloc)").clicked() {
                            self.open_code_window(
                                CodeWindowType::Cbuoy,
                                include_str!("../../cbos/bin/hello_mem.cb").to_string(),
                                Some("hello_mem.cb"),
                            );
                        }
                    });

                    ui.menu_button("Examples", |ui| {
                        static CB_CODES: &[(&str, &str, &str)] = &[
                            (
                                "Default",
                                include_str!("../../cbuoy/cblang/examples/default.cb"),
                                "default.cb",
                            ),
                            (
                                "Threading",
                                include_str!("../../cbuoy/cblang/examples/threading.cb"),
                                "threading.cb",
                            ),
                            (
                                "kmalloc",
                                include_str!("../../cbuoy/cblang/tests/test_kmalloc.cb"),
                                "test/kmalloc.cb",
                            ),
                            (
                                "Structures",
                                include_str!("../../cbuoy/cblang/tests/test_struct_ptr.cb"),
                                "test/structures.cb",
                            ),
                        ];

                        for (name, code, path) in CB_CODES.iter().cloned() {
                            if ui.button(name).clicked() {
                                self.open_code_window(
                                    CodeWindowType::Cbuoy,
                                    code.into(),
                                    Some(path),
                                );
                            }
                        }
                    });
                    ui.menu_button("Components", |ui| {
                        for (path, code) in cblang::DEFAULT_FILES.iter().cloned() {
                            let name = path.split('/').next_back().unwrap_or(path);
                            if ui.button(name).clicked() {
                                self.open_code_window(
                                    CodeWindowType::Cbuoy,
                                    code.into(),
                                    Some(path),
                                );
                            }
                        }
                    });
                });

                ui.menu_button("J/ASM", |ui| {
                    if ui.button("New").clicked() {
                        self.open_code_window(CodeWindowType::Assembly, String::new(), None);
                    }

                    ui.menu_button("Examples", |ui| {
                        static ASM_CODES: &[(&str, &str, &str)] = &[
                            (
                                "Hello World",
                                include_str!("../../jib/jib-asm/examples/hello_world.jsm"),
                                "hello_world.jsm",
                            ),
                            (
                                "Thread Test",
                                include_str!("../../jib/jib-asm/examples/thread_test.jsm"),
                                "thread_test.jsm",
                            ),
                            (
                                "Serial Echo",
                                include_str!("../../jib/jib-asm/examples/serial_echo.jsm"),
                                "serial_echo.jsm",
                            ),
                            (
                                "Infinite Counter",
                                include_str!("../../jib/jib-asm/examples/infinite_counter.jsm"),
                                "infinite_counter.jsm",
                            ),
                        ];

                        for (name, code, filename) in ASM_CODES.iter().cloned() {
                            if ui.button(name).clicked() {
                                self.open_code_window(
                                    CodeWindowType::Assembly,
                                    code.into(),
                                    Some(filename),
                                );
                            }
                        }
                    });
                });

                ui.menu_button("Devices", |ui| {
                    if ui.button("Memory View").clicked() {
                        self.open_memory_window();
                    }

                    ui.menu_button("IRQ", |ui| {
                        for i in 0..16 {
                            if ui.button(format!("IRQ{}", i)).clicked() {
                                self.tx_ui.send(UiToThread::CpuIrq(i)).unwrap();
                            }
                        }
                    });

                    if ui.button("Reset Disk").clicked() {
                        self.tx_ui.send(UiToThread::DiskReset).unwrap();
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button("Save Disk").clicked() {
                        self.tx_ui.send(UiToThread::DiskSave).unwrap();
                    }
                });
            });

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Commands");
                    if ui
                        .checkbox(&mut self.use_bootloader, "Use Bootloader")
                        .changed()
                    {
                        self.tx_ui
                            .send(UiToThread::UseBootloader(self.use_bootloader))
                            .unwrap();
                    }
                    Grid::new("cpu_commands").show(ui, |ui| {
                        if ui.button("Step").clicked() {
                            self.tx_ui.send(UiToThread::CpuStep).unwrap();
                        }

                        if ui.button("Start").clicked() {
                            self.tx_ui.send(UiToThread::CpuRun(true)).unwrap();
                        }

                        if ui.button("Stop").clicked() {
                            self.tx_ui.send(UiToThread::CpuRun(false)).unwrap();
                        }

                        if ui.button("Reset").clicked() {
                            self.tx_ui.send(UiToThread::CpuReset).unwrap();
                        }
                    });

                    ui.label("Speed Multiplier");
                    if ui
                        .add(
                            Slider::new(
                                &mut self.current_cpu_speed,
                                VisualJib::SPEED_MIN..=VisualJib::SPEED_MAX,
                            )
                            .show_value(true),
                        )
                        .changed()
                    {
                        self.tx_ui
                            .send(UiToThread::SetMultiplier(self.current_cpu_speed))
                            .unwrap();
                    }

                    ui.heading("CPU Registers");
                    const NUM_COLS: usize = 2;
                    Grid::new("cpu_registers")
                        .striped(true)
                        .num_columns(NUM_COLS)
                        .show(ui, |ui| {
                            const NUM_ROWS: usize = RegisterManager::REGISTER_COUNT / NUM_COLS;

                            for i in 0..NUM_ROWS {
                                ui.label(format!("R{:02}: {:08x}", i, self.registers.registers[i]));
                                ui.label(format!(
                                    "R{:02}: {:08x}",
                                    i + NUM_ROWS,
                                    self.registers.registers[i + NUM_ROWS]
                                ));
                                ui.end_row();
                            }
                        });

                    ui.heading("Program Log");

                    ScrollArea::both().stick_to_bottom(true).show(ui, |ui| {
                        TextEdit::multiline(&mut self.log_text)
                            .code_editor()
                            .cursor_at_end(true)
                            .interactive(false)
                            .clip_text(false)
                            .show(ui);
                    });
                });

                ui.vertical(|ui| {
                    ui.heading("Program Counter");
                    ui.label(format!(
                        "PC[0x{:08x}] = 0x{:08x}",
                        self.program_counter.pc, self.program_counter.val
                    ));
                    ui.label(format!(
                        "Inst: {}",
                        self.program_counter.get_instruction_string()
                    ));

                    ui.heading("Serial Input");
                    const RETURN_KEY: egui::Key = egui::Key::Enter;
                    const RETURN_SHORTCUT: egui::KeyboardShortcut =
                        egui::KeyboardShortcut::new(egui::Modifiers::NONE, RETURN_KEY);

                    let serial_txt = TextEdit::singleline(&mut self.text_serial_input)
                        .desired_width(ui.available_width())
                        .return_key(Some(RETURN_SHORTCUT))
                        .code_editor()
                        .show(ui)
                        .response;

                    if serial_txt.lost_focus() && ui.input(|x| x.key_pressed(RETURN_KEY)) {
                        self.tx_ui
                            .send(UiToThread::SerialInput(self.text_serial_input.take()))
                            .unwrap();
                        serial_txt.request_focus();
                    }

                    ui.heading("Serial Log");
                    if ui.button("Clear").clicked() {
                        self.log_serial = String::new();
                    }
                    ScrollArea::vertical()
                        .id_salt("serial_log")
                        .stick_to_bottom(true)
                        .stick_to_right(true)
                        .show(ui, |ui| {
                            TextEdit::multiline(&mut self.log_serial)
                                .interactive(false)
                                .code_editor()
                                .desired_width(ui.available_width())
                                .show(ui);
                        });
                });

                for w in self.code_windows.iter_mut() {
                    w.draw(ui)
                }

                for m in self.memory_windows.iter_mut() {
                    m.draw(ui);
                }
            });
        });

        if let Some(int) = self.update_interval() {
            ui.request_repaint_after(int);
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CodeWindowType {
    Assembly,
    Cbuoy,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum CodeWindowAction {
    NewAssemblyWindow(String, Option<&'static str>),
}

struct CodeWindow {
    tx_ui: std::sync::mpsc::Sender<UiToThread>,
    tx_thread: std::sync::mpsc::Sender<ThreadToUi>,
    tx_window: std::sync::mpsc::Sender<CodeWindowAction>,
    code: String,
    shown: bool,
    id: String,
    compiled: CodeWindowType,
    filename: Option<&'static str>,
}

impl CodeWindow {
    const CB_NAME: &str = "CB";
    const ASM_NAME: &str = "ASM";

    fn new(
        id: usize,
        tx_ui: std::sync::mpsc::Sender<UiToThread>,
        tx_thread: std::sync::mpsc::Sender<ThreadToUi>,
        tx_window: std::sync::mpsc::Sender<CodeWindowAction>,
        code: String,
        code_type: CodeWindowType,
        filename: Option<&'static str>,
    ) -> Self {
        Self {
            tx_ui,
            tx_thread,
            tx_window,
            code,
            shown: true,
            id: format!(
                "{} {id}{}",
                match code_type {
                    CodeWindowType::Assembly => "ASM",
                    CodeWindowType::Cbuoy => "C/Buoy",
                },
                if let Some(t) = &filename {
                    format!(" ({t})")
                } else {
                    String::new()
                }
            ),
            compiled: code_type,
            filename,
        }
    }

    fn compile_asm(&self) {
        let asm = &self.code;
        match jib_asm::assemble_text(asm.as_str()) {
            Ok(v) => {
                self.tx_ui.send(UiToThread::SetCode(v.into())).unwrap();
                self.tx_thread
                    .send(ThreadToUi::LogMessage(format!(
                        "{} Successful",
                        Self::ASM_NAME
                    )))
                    .unwrap();
            }
            Err(e) => self.log_error(e),
        }
    }

    fn log_error<T: ErrorString>(&self, e: T) {
        self.tx_thread
            .send(ThreadToUi::LogMessage(e.get_error_string()))
            .unwrap();
    }

    fn compile_cbuoy_to_asm(&self) -> Option<CompilingState> {
        let preprocessed = match cblang::preprocess_code_as_file(
            &self.code,
            &if let Some(f) = &self.filename {
                PathBuf::from(f)
            } else {
                PathBuf::from("code.cb")
            },
            [].into_iter(),
        ) {
            Ok(val) => val,
            Err(err) => {
                self.tx_thread
                    .send(ThreadToUi::LogMessage(format!(
                        "{} Preprocessor: {err}",
                        Self::CB_NAME
                    )))
                    .unwrap();
                return None;
            }
        };

        let tokens = match preprocessed.tokenize() {
            Ok(val) => val,
            Err(e) => {
                self.log_error(e);
                return None;
            }
        };

        let options = CodeGenerationOptions::default();

        match cblang::compile(tokens, options).map_err(CompilerError::from) {
            Ok(cmp) => Some(cmp),
            Err(CompilerError::TokenError(e)) => {
                self.log_error(TokenErrorContext {
                    e,
                    preprocessed: Some(preprocessed),
                });
                None
            }
            Err(e) => {
                self.log_error(e);
                None
            }
        }
    }

    fn compile_cbuoy(&self) {
        if let Some(cmp) = self.compile_cbuoy_to_asm() {
            match cmp.get_assembler() {
                Ok(asm) => {
                    let mut code = asm.bytes;

                    // Determine the export region location and length values, searching
                    // for the key variables to assign into the kernel
                    let export_bytes = cmp
                        .get_export_interface()
                        .unwrap()
                        .get_interface_region()
                        .unwrap();

                    const VAR_EXPORT_DATA: &str = "K_LINK_EXPORT_DATA";
                    const VAR_EXPORT_SIZE: &str = "K_LINK_EXPORT_SIZE";

                    let export_loc = code.len() as u32 + asm.start_address;
                    let export_len = export_bytes.len();

                    let mut assign_data = None;
                    let mut assign_size = None;

                    for v in cmp.get_import_interface().unwrap_or_default().variables {
                        if v.name == VAR_EXPORT_DATA {
                            assign_data = Some((v.loc - asm.start_address, export_loc as u32));
                        } else if v.name == VAR_EXPORT_SIZE {
                            assign_size = Some((v.loc - asm.start_address, export_len as u32));
                        }
                    }

                    // If both variables were found, assign the respective variables and add the
                    // export region to the compiled code
                    if let Some(a1) = assign_data
                        && let Some(a2) = assign_size
                    {
                        for (copy_loc, val) in [a1, a2] {
                            let bytes = val.to_be_bytes();
                            for (i, b) in bytes.iter().enumerate() {
                                code[copy_loc as usize + i] = *b;
                            }
                        }

                        code.extend(export_bytes);
                    }

                    // Send the code
                    self.tx_ui
                        .send(UiToThread::SetCode(JibCode {
                            start_location: asm.start_address,
                            code,
                        }))
                        .unwrap();
                    self.tx_thread
                        .send(ThreadToUi::LogMessage(format!(
                            "{}: Compile Successful",
                            Self::CB_NAME
                        )))
                        .unwrap();
                }
                Err(e) => self.log_error(e),
            }
        }
    }

    fn draw(&mut self, ctx: &Context) {
        let mut opened = self.shown;

        egui::Window::new(self.id.as_str())
            .resizable(true)
            .open(&mut opened)
            .show(ctx, |ui| {
                if ui
                    .button(match self.compiled {
                        CodeWindowType::Cbuoy => "Compile",
                        CodeWindowType::Assembly => "Assemble",
                    })
                    .clicked()
                {
                    match self.compiled {
                        CodeWindowType::Assembly => self.compile_asm(),
                        CodeWindowType::Cbuoy => self.compile_cbuoy(),
                    };
                }

                if self.compiled == CodeWindowType::Cbuoy
                    && ui.button("Show Assembly").clicked()
                    && let Some(cmp_out) = self.compile_cbuoy_to_asm()
                    && let Some(asm_out) = cmp_out.get_assembler().ok()
                {
                    let asm = format!(
                        "{}\n{}",
                        asm_out.assembly_lines.join("\n"),
                        asm_out.assembly_debug.join("\n")
                    );

                    self.tx_window
                        .send(CodeWindowAction::NewAssemblyWindow(asm, self.filename))
                        .unwrap();
                }

                ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    let text = &mut self.code;
                    ui.add_sized(
                        ui.available_size(),
                        TextEdit::multiline(text)
                            .code_editor()
                            .cursor_at_end(false)
                            .desired_width(ui.available_width())
                            .clip_text(false),
                    );
                });
            });

        self.shown = opened;
    }
}

trait ErrorString {
    fn get_error_string(&self) -> String;
}

impl ErrorString for AssemblerErrorLoc {
    fn get_error_string(&self) -> String {
        format!("{}: {self}", CodeWindow::ASM_NAME)
    }
}

impl ErrorString for CompilerError {
    fn get_error_string(&self) -> String {
        format!("{}", self)
    }
}

impl ErrorString for TokenError {
    fn get_error_string(&self) -> String {
        format!("{}: Tokenize error {self}", CodeWindow::CB_NAME)
    }
}

struct TokenErrorContext {
    e: TokenError,
    preprocessed: Option<PreprocessorOutput>,
}

impl ErrorString for TokenErrorContext {
    fn get_error_string(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("{}: {}", CodeWindow::CB_NAME, self.e));

        if let Some(t) = &self.e.token
            && let Some(preprocessed) = self.preprocessed.as_ref()
        {
            for l in preprocessed.get_lines().iter() {
                if l.loc.line == t.get_loc().line && Some(&l.loc.file) == t.get_loc().file.as_ref()
                {
                    let line = &l.text;

                    let mut err_msg = format!("{} >> {line}\n", l.loc);
                    err_msg += &format!("{}    ", l.loc);
                    for _ in 0..t.get_loc().column {
                        err_msg += " ";
                    }
                    for _ in 0..t.get_value().len() {
                        err_msg += "^";
                    }
                    lines.push(err_msg);
                    break;
                }
            }
        }

        lines.join("\n")
    }
}
