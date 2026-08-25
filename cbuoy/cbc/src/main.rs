/// cbc is the command-line interface for the C/Buoy compiler. This provides a way to
/// interactively compile arbitary programs and use in various formats
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
};

use cblang::{
    compiler::InterfaceDefinition,
    preprocessor::{OverlayFilesystem, RealFilesystem, VirtualFilesystem},
    CodeGenerationOptions, Compiler, ProgramType,
};
use clap::Parser;

/// Compiler arguments used to control how the compiler functions
#[derive(Default, Debug, Parser)]
#[command(version, about)]
struct CompilerArguments {
    /// Provides the primary input file to compile
    #[arg()]
    input_file: PathBuf,
    /// Provides the name of the generated binary file
    #[arg(
        short = 'o',
        long = "output",
        help = "Creates a binary file with the generated machine code"
    )]
    output_binary: Option<PathBuf>,
    /// If true, will use as a kernel program instead of an application, using the global
    /// kernel offset
    #[arg(
        short = 'k',
        long = "kernel",
        help = "Enables program generation in kernel mode with the provided start location"
    )]
    kernel_program: bool,
    /// The starting location to use for a kernel program
    #[arg(
        short = 'K',
        long = "kernel-start-loc",
        default_value_t = ProgramType::DEFAULT_START_OFFSET,
        help="Initial program location when generating in kernel mode"
    )]
    kernel_start_offset: u32,
    /// The stack location for a kernel program
    #[arg(
        short = 's',
        long = "kernel-stack-loc",
        default_value_t = ProgramType::DEFAULT_STACK_LOC,
        help = "Initial stack location when generating in kernel mode",
    )]
    kernel_stack_loc: u32,
    /// Determines whether unused functions should be trimmed from the binary
    #[arg(
        short = 't',
        long = "trim",
        default_value_t = false,
        help = "Trims unused functions and variables from the generated code"
    )]
    trim_unused: bool,
    /// Provides the overlal AST to the console
    #[arg(
        short = 'a',
        long = "output-ast",
        default_value_t = false,
        help = "Prints the AST to the console"
    )]
    print_ast: bool,
    /// Adds additional debugging information to the generated assembly code
    #[arg(
        short = 'l',
        long = "locs",
        default_value_t = false,
        help = "Includes location/debugging information in output assembly code"
    )]
    include_locations: bool,
    /// Provides a text file with the raw assembly code generated, pre-assembly
    #[arg(
        short = 'j',
        long = "jib",
        help = "Creates a text file with the generated assembly code"
    )]
    output_assembly: Option<PathBuf>,
    /// Provides a text file with the generated/preprocessed source code
    #[arg(
        short = 'c',
        long = "output-cbp",
        help = "Creates a text file containing the preprocessed source code"
    )]
    output_preproc: Option<PathBuf>,
    /// Provdes any additional compiler definitions to use when compiling
    #[arg(
        short = 'D',
        long = "define",
        help = "Adds compiler definitions to define from the start of compiling. Use '=' to assign values"
    )]
    definitions: Vec<String>,
    /// Allows defining system include files
    #[arg(
        short = 'I',
        long = "include",
        help = "Defines the system include path. Defining any parameter here will replace the default include path"
    )]
    include_directories: Vec<String>,
    #[arg(
        short = 'S',
        long = "stdlib-default",
        help = "If true, will include the built-in system stdlib implementation of the <kernel> and <std> namespaces"
    )]
    default_lib: bool,
    /// Provides an output interface/map file to use for constants, structures, and functions
    #[arg(
        short = 'W',
        long = "write-interface",
        help = "Writes an interface file for the provided symbols"
    )]
    interface_file: Option<PathBuf>,
    #[arg(
        long = "interface-guard",
        help = "The include guard to use for the interface file, or empty string for none",
        default_value = "CBOS_DEFS"
    )]
    interface_guard: String,
    #[arg(
        short = 'z',
        long = "zero-locations",
        help = "Zeros locations for functions and variables in generated interface files"
    )]
    zero_locations: bool,
}

impl CompilerArguments {
    /// Provides the current code generation options
    fn compiler_options(&self) -> CodeGenerationOptions {
        CodeGenerationOptions {
            prog_type: if self.kernel_program {
                ProgramType::Kernel {
                    stack_loc_init: Some(self.kernel_stack_loc),
                    base_location: self.kernel_start_offset,
                }
            } else {
                ProgramType::Application
            },
            debug_locations: self.include_locations,
            trim_code: self.trim_unused,
        }
    }
}

/// Main entry function
fn main() -> std::process::ExitCode {
    // Define arguments
    let args = CompilerArguments::parse();

    // Construct the system root
    let mut sysfs = OverlayFilesystem::default();

    for i in args.include_directories.iter() {
        sysfs.systems.push(Rc::new(RealFilesystem::new_relative(i)));
    }

    // Construct the preprocessed argument list
    let mut definitions = HashMap::new();
    for d in args.definitions.iter() {
        if let Some((k, v)) = d.split_once('=') {
            definitions.insert(k.trim().into(), v.trim().into());
        } else {
            definitions.insert(d.trim().into(), String::new());
        }
    }

    if args.default_lib {
        if args.kernel_program {
            sysfs.systems.push(Rc::new(VirtualFilesystem::new_system()));
        } else {
            let kernel_compiler = Compiler {
                definitions: definitions.clone(),
                options: CodeGenerationOptions {
                    prog_type: ProgramType::default(),
                    debug_locations: false,
                    trim_code: false,
                },
                system_root: Rc::new(VirtualFilesystem::new_system()),
            };

            let res = kernel_compiler
                .compile_string(include_str!("../../../cbos/os.cb"), None)
                .unwrap();
            let mut intf = Vec::new();
            res.export_interface.write_interface(&mut intf).unwrap();

            const CBOS_INTF_GUARD: &str = "CBOS_DEFS";
            let interface_str = format!(
                "#ifndef {CBOS_INTF_GUARD}\n#define {CBOS_INTF_GUARD}\n{}\n#endif // {CBOS_INTF_GUARD}\n",
                String::from_utf8(intf).unwrap()
            );

            let mut vfs = VirtualFilesystem::default();
            vfs.add_file(
                &Path::new("cb_app.cb"),
                include_str!("../../../cbos/cb_app.cb"),
            )
            .unwrap();
            vfs.add_file(&Path::new("cbos_defs.cb"), &interface_str)
                .unwrap();
            sysfs.systems.push(Rc::new(vfs));
        }
    }

    let compiled = Compiler {
        definitions,
        options: args.compiler_options(),
        system_root: Rc::new(sysfs),
    };

    let result = match compiled.compile_file(&args.input_file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    // Write the preprocessed data if provided
    if let Some(file) = &args.output_preproc {
        match std::fs::File::create(file) {
            Ok(mut f) => {
                for l in result.preprocessed.get_lines() {
                    writeln!(f, "{}", l.text).unwrap();
                }
            }
            Err(e) => {
                eprintln!(
                    "unable to open output file {} - {e}",
                    file.to_str().unwrap_or("?")
                );
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    // Prints the AST if requested
    if args.print_ast {
        println!("{}", result.ast_statements.join("\n"));
    }

    // Define the interface file if requested
    if let Some(interface_path) = &args.interface_file {
        let interface_file = match std::fs::File::create(interface_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Unable to open interface file - {e}");
                return std::process::ExitCode::FAILURE;
            }
        };

        let mut interface = result.export_interface.clone();
        if args.zero_locations {
            interface = interface.zero_offsets();
        }

        fn write_interface(
            mut interface_file: std::fs::File,
            interface: InterfaceDefinition,
            args: &CompilerArguments,
        ) -> std::io::Result<()> {
            if !args.interface_guard.is_empty() {
                writeln!(interface_file, "#ifndef {}", args.interface_guard)?;
                writeln!(interface_file, "#define {}", args.interface_guard)?;
                writeln!(interface_file)?;
            }

            interface.write_interface(&mut interface_file)?;

            if !args.interface_guard.is_empty() {
                writeln!(interface_file)?;
                writeln!(interface_file, "#endif // {}", args.interface_guard)?;
            }

            Ok(())
        }

        match write_interface(interface_file, interface, &args) {
            Ok(_) => (),
            Err(e) => {
                eprintln!("Error writing interface file - {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    // Output the assembly if requested
    if let Some(out) = args.output_assembly {
        let asm = &result.asm;
        match std::fs::File::create(out) {
            Ok(mut f) => {
                for l in &asm.assembly_lines {
                    writeln!(f, "{l}").unwrap();
                }
                if args.include_locations {
                    for l in &asm.assembly_debug {
                        writeln!(f, "{l}").unwrap();
                    }
                }
            }
            Err(e) => {
                eprintln!("{e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    // Output the binary if requested
    if let Some(out) = args.output_binary {
        match std::fs::File::create(out) {
            Ok(mut f) => f.write_all(&result.binary).unwrap(),
            Err(e) => {
                eprintln!("{e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    std::process::ExitCode::SUCCESS
}
