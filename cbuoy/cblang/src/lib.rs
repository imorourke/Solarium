pub mod compiler;
mod expressions;
mod functions;
mod literals;
mod parser;
pub mod preprocessor;
mod tokenizer;
mod typing;
mod utilities;
mod variables;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

pub use compiler::{CodeGenerationOptions, CompilerError, ProgramType};
use jib_asm::AssemblerOutput;
use parser::compile_tokens;
use preprocessor::read_and_preprocess;
pub use tokenizer::{TokenError, tokenize, tokenize_file, tokenize_str};
pub use typing::Type;

use crate::{
    compiler::InterfaceDefinition,
    preprocessor::{
        PreprocessorFilesystem, PreprocessorOutput, PreprocessorState, RealFilesystem,
        VirtualFilesystem,
    },
};

#[derive(Debug)]
pub struct CompileResults {
    pub preprocessed: PreprocessorOutput,
    pub export_interface: InterfaceDefinition,
    pub import_interface: InterfaceDefinition,
    pub asm: AssemblerOutput,
    pub binary: Vec<u8>,
    pub ast_statements: Vec<String>,
}

#[derive(Debug)]
pub struct Compiler {
    pub system_root: Rc<dyn PreprocessorFilesystem>,
    pub definitions: HashMap<String, String>,
    pub options: CodeGenerationOptions,
}

impl Compiler {
    pub fn compile_file(&self, file: &Path) -> Result<CompileResults, CompilerError> {
        self.compile_fs(file, Rc::new(RealFilesystem::default()))
    }

    pub fn compile_string(
        &self,
        s: &str,
        name: Option<&Path>,
    ) -> Result<CompileResults, CompilerError> {
        let file = name.map_or(PathBuf::from("input.cb"), |x| x.to_path_buf());
        self.compile_fs(&file, Rc::new(VirtualFilesystem::new(s, &file)))
    }

    pub fn compile_fs(
        &self,
        file: &Path,
        fs: Rc<dyn PreprocessorFilesystem>,
    ) -> Result<CompileResults, CompilerError> {
        let mut state = PreprocessorState::new_system(fs, self.system_root.clone());
        for (k, v) in self.definitions.iter() {
            state.init_definitions.insert(k.clone(), v.clone());
        }
        let preprocessed = state.read_file(file)?;

        // Tokenize the input preprocessed data
        let input_tokens = preprocessed.tokenize()?;

        // Compile the program
        let state = match compile_tokens(input_tokens.clone(), self.options.clone()) {
            Ok(v) => v,
            Err(e) => {
                return Err(CompilerError::TokenErrorFancy(e, preprocessed));
            }
        };

        let asm = match state.get_assembler() {
            Ok(v) => v,
            Err(CompilerError::TokenError(e)) => {
                return Err(CompilerError::TokenErrorFancy(e, preprocessed));
            }
            Err(e) => {
                return Err(e);
            }
        };

        let export_interface = match state.get_export_interface() {
            Ok(v) => v,
            Err(CompilerError::TokenError(e)) => {
                return Err(CompilerError::TokenErrorFancy(e, preprocessed));
            }
            Err(e) => {
                return Err(e);
            }
        };

        let import_interface = match state.get_import_interface() {
            Ok(v) => v,
            Err(CompilerError::TokenError(e)) => {
                return Err(CompilerError::TokenErrorFancy(e, preprocessed));
            }
            Err(e) => {
                return Err(e);
            }
        };

        let binary = match state.get_binary() {
            Ok(v) => v,
            Err(CompilerError::TokenError(e)) => {
                return Err(CompilerError::TokenErrorFancy(e, preprocessed));
            }
            Err(e) => {
                return Err(e);
            }
        };

        Ok(CompileResults {
            preprocessed,
            export_interface,
            import_interface,
            asm,
            binary,
            ast_statements: state.get_statements(),
        })
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self {
            definitions: HashMap::default(),
            options: CodeGenerationOptions::default(),
            system_root: Rc::new(VirtualFilesystem::new_system()),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::Compiler;
    use jib_asm::assemble_lines;
    use std::path::Path;

    static EXAMPLE_FILES: &[&str] = &[
        "examples/array_test.cb",
        "examples/default.cb",
        "../../cbos/os.cb",
        "examples/printing.cb",
        "examples/threading.cb",
        "tests/test_kmalloc.cb",
        "tests/test_struct_ptr.cb",
        "tests/test_comment.cb",
        "tests/test_struct_ptr.cb",
    ];

    #[test]
    fn valid_compiling_and_assembler_output() {
        for s in EXAMPLE_FILES {
            let input_file = Path::join(&Path::new(env!("CARGO_MANIFEST_DIR")), &Path::new(s));
            let compiler = Compiler::default();

            let res = compiler.compile_file(&input_file).unwrap();

            let asm_out_duplicate =
                assemble_lines(res.asm.assembly_lines.iter().map(|x| x.as_ref())).unwrap();

            assert_eq!(res.asm.bytes, asm_out_duplicate.bytes);
        }
    }
}
