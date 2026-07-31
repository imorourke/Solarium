mod compiler;
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
    path::{Path, PathBuf},
    rc::Rc,
};

pub use compiler::{CodeGenerationOptions, CompilerError, CompilingState, ProgramType};
pub use parser::{compile_str, compile_tokens};
use preprocessor::read_and_preprocess;
pub use tokenizer::{TokenError, tokenize, tokenize_file, tokenize_str};
pub use typing::Type;

use crate::preprocessor::{
    PreprocessorFilesystem, PreprocessorOutput, PreprocessorState, RealFilesystem,
    VirtualFilesystem,
};

#[derive(Debug)]
pub struct CompileResults {
    pub preprocessed: PreprocessorOutput,
    pub state: CompilingState,
}

#[derive(Debug)]
pub struct Compiler {
    pub system_root: Rc<dyn PreprocessorFilesystem>,
    pub start_file: PathBuf,
    pub definitions: Vec<String>,
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

    fn compile_fs(
        &self,
        file: &Path,
        fs: Rc<dyn PreprocessorFilesystem>,
    ) -> Result<CompileResults, CompilerError> {
        let mut state = PreprocessorState::new_system(fs, self.system_root.clone());
        for d in self.definitions.iter().cloned() {
            state.definitions.insert(d);
        }
        let preprocessed = state.read_file(file)?;

        // Tokenize the input preprocessed data
        let input_tokens = preprocessed.tokenize()?;

        // Compile the program
        let cbstate = compile_tokens(input_tokens.clone(), self.options.clone())?;

        Ok(CompileResults {
            preprocessed,
            state: cbstate,
        })
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use jib_asm::assemble_lines;

    use crate::{CodeGenerationOptions, compile_tokens, tokenize_file};

    static EXAMPLE_FILES: &[&str] = &[
        "examples/array_test.cb",
        "examples/default.cb",
        "../../cbos/os.cb",
        "examples/printing.cb",
        "examples/threading.cb",
        "tests/test_kmalloc.cb",
        "tests/test_struct_ptr.cb",
        "tests/test_comment.cb",
    ];

    #[test]
    fn valid_compiling_and_assembler_output() {
        for s in EXAMPLE_FILES {
            let input_file = Path::join(&Path::new(env!("CARGO_MANIFEST_DIR")), &Path::new(s));
            let tokens = tokenize_file(&input_file).unwrap();
            let cb_out = compile_tokens(tokens, CodeGenerationOptions::default()).unwrap();
            let asm_out_main = cb_out.get_assembler().unwrap();
            let asm_out_duplicate =
                assemble_lines(asm_out_main.assembly_lines.iter().map(|x| x.as_ref())).unwrap();

            assert_eq!(asm_out_main.bytes, asm_out_duplicate.bytes);
        }
    }
}
