//! C/Buoy Preprocessor
//!
//! Provides simple macros and preprocessing logic to handle comments and similar items
//! to help with C/Buoy code compilation.

use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::Entry},
    fmt::{Debug, Display},
    ops::RangeInclusive,
    path::{Path, PathBuf},
    rc::Rc,
    sync::LazyLock,
};

use regex::Regex;

use crate::{TokenError, tokenize, tokenizer::Token};

/// Provides default files that should be included within the compiler standard library
#[cfg(feature = "cbos")]
pub static DEFAULT_FILES: &[(&str, &str)] = &[
    (
        "kernel/kconfig.cb",
        include_str!("../../../cbos/kernel/kconfig.cb"),
    ),
    (
        "kernel/kclock.cb",
        include_str!("../../../cbos/kernel/kclock.cb"),
    ),
    (
        "kernel/kcpu.cb",
        include_str!("../../../cbos/kernel/kcpu.cb"),
    ),
    (
        "kernel/kdbg.cb",
        include_str!("../../../cbos/kernel/kdbg.cb"),
    ),
    (
        "kernel/kdef.cb",
        include_str!("../../../cbos/kernel/kdef.cb"),
    ),
    (
        "kernel/kdevice.cb",
        include_str!("../../../cbos/kernel/kdevice.cb"),
    ),
    (
        "kernel/kdisk.cb",
        include_str!("../../../cbos/kernel/kdisk.cb"),
    ),
    (
        "kernel/kexec.cb",
        include_str!("../../../cbos/kernel/kexec.cb"),
    ),
    (
        "kernel/kirq.cb",
        include_str!("../../../cbos/kernel/kirq.cb"),
    ),
    ("kernel/kfs.cb", include_str!("../../../cbos/kernel/kfs.cb")),
    (
        "kernel/kmalloc.cb",
        include_str!("../../../cbos/kernel/kmalloc.cb"),
    ),
    (
        "kernel/klink.cb",
        include_str!("../../../cbos/kernel/klink.cb"),
    ),
    (
        "kernel/kmalloc_dbg.cb",
        include_str!("../../../cbos/kernel/kmalloc_dbg.cb"),
    ),
    (
        "kernel/krtc.cb",
        include_str!("../../../cbos/kernel/krtc.cb"),
    ),
    (
        ("kernel/kserialio.cb"),
        include_str!("../../../cbos/kernel/kserialio.cb"),
    ),
    (
        ("kernel/kshell.cb"),
        include_str!("../../../cbos/kernel/kshell.cb"),
    ),
    (
        "kernel/ktsk.cb",
        include_str!("../../../cbos/kernel/ktsk.cb"),
    ),
    ("std/list.cb", include_str!("../../../cbos/std/list.cb")),
    (
        ("std/string.cb"),
        include_str!("../../../cbos/std/string.cb"),
    ),
];

/// Provides the ouptut of the preprocessor
#[derive(Debug, Clone)]
pub struct PreprocessorOutput {
    lines: Vec<PreprocessorLine>,
}

impl PreprocessorOutput {
    pub fn full_string(&self) -> String {
        self.lines
            .iter()
            .map(|x| &x.text)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn tokenize(&self) -> Result<Vec<Token>, TokenError> {
        tokenize(
            self.lines
                .iter()
                .map(|x| (x.text.clone(), Some(x.loc.clone()))),
        )
    }

    pub fn get_lines(&self) -> &[PreprocessorLine] {
        &self.lines
    }
}

#[derive(Debug, Clone)]
pub struct PreprocessorState {
    pub filesystem: Rc<dyn PreprocessorFilesystem>,
    pub system_fs: Rc<dyn PreprocessorFilesystem>,
    pub init_definitions: HashMap<String, String>,
}

impl PreprocessorState {
    pub fn new(fs: Rc<dyn PreprocessorFilesystem>) -> PreprocessorState {
        Self {
            filesystem: fs,
            system_fs: Rc::new(VirtualFilesystem::new_system()),
            init_definitions: HashMap::default(),
        }
    }

    pub fn new_system(
        fs: Rc<dyn PreprocessorFilesystem>,
        sys: Rc<dyn PreprocessorFilesystem>,
    ) -> PreprocessorState {
        Self {
            filesystem: fs,
            system_fs: sys,
            init_definitions: HashMap::default(),
        }
    }

    pub(crate) fn read_file(
        &mut self,
        file: &Path,
    ) -> Result<PreprocessorOutput, PreprocessorError> {
        let mut if_statements = Vec::default();
        let mut definitions = self.init_definitions.clone();
        self.read_file_inner(file, None, 0, &mut definitions, &mut if_statements)
    }

    fn find_comment_spans(
        s: &str,
        file: &str,
    ) -> Result<Vec<RangeInclusive<usize>>, PreprocessorError> {
        enum CommentType {
            Line(usize),
            Block(usize, i32),
        }

        let mut within_comment: Option<CommentType> = None;
        let mut within_string = if let Some(c) = s.chars().next()
            && (c == '\'' || c == '"')
        {
            Some(c)
        } else {
            None
        };

        let mut comment_spans = Vec::new();
        let mut line_num = 1;

        for ((i0, c0), (i1, c1)) in s.char_indices().zip(s.char_indices().skip(1)) {
            match (c0, c1) {
                ('/', '/') if within_string.is_none() && within_comment.is_none() => {
                    within_comment = Some(CommentType::Line(i0));
                }
                ('/', '*') if within_string.is_none() => {
                    if within_comment.is_none() {
                        within_comment = Some(CommentType::Block(i0, 1));
                    } else if let Some(CommentType::Block(i, blks)) = within_comment {
                        within_comment = Some(CommentType::Block(i, blks + 1));
                    }
                }
                ('*', '/') => {
                    if let Some(CommentType::Block(i, blks)) = within_comment {
                        if (blks - 1).max(0) == 0 {
                            comment_spans.push(i..=i1);
                            within_comment = None;
                        } else {
                            within_comment = Some(CommentType::Block(i, blks - 1));
                        }
                    }
                }
                (_, '\n') => {
                    line_num += 1;
                    if let Some(CommentType::Line(i)) = within_comment {
                        comment_spans.push(i..=i0);
                        within_comment = None;
                    }
                }
                (_, '"') | (_, '\'') if within_comment.is_none() => {
                    if within_string.is_none() {
                        within_string = Some(c1);
                    } else if let Some(c) = within_string
                        && c1 == c
                        && c0 != '\\'
                    {
                        within_string = None;
                    }
                }
                _ => (),
            }
        }

        match within_comment {
            Some(CommentType::Line(i)) => comment_spans.push(i..=s.len()),
            Some(CommentType::Block(_, _)) => {
                return Err(PreprocessorError {
                    loc: Some(PreprocessorLocation {
                        file: file.into(),
                        line: line_num,
                    }),
                    text: "".into(),
                    error: "unclosed block comment found".into(),
                });
            }
            _ => (),
        };

        Ok(comment_spans)
    }

    fn remove_comments(s: &str, file: &str) -> Result<String, PreprocessorError> {
        let blocks = Self::find_comment_spans(s, file)?;

        if blocks.is_empty() {
            return Ok(s.to_string());
        }

        let mut blk_i = 0;
        let mut current = &blocks[0];

        let mut char_vec = s.chars().collect::<Vec<_>>();

        'outer: for (i, c) in char_vec.iter_mut().enumerate() {
            while i > *current.end() {
                blk_i += 1;
                if blk_i < blocks.len() {
                    current = &blocks[blk_i];
                } else {
                    break 'outer;
                }
            }

            if i < *current.start() {
                continue;
            } else if current.contains(&i) && *c != '\n' {
                *c = ' ';
            }
        }

        Ok(char_vec.into_iter().collect())
    }

    fn get_file_path(
        &self,
        current: &Path,
        arg: &str,
    ) -> Result<(PathBuf, Option<&dyn PreprocessorFilesystem>), PreprocessorError> {
        static SYS_REGEX: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^<(?<file>.*)>$").unwrap());

        Ok(
            if let Some(m) = SYS_REGEX.captures(arg)
                && let Some(file) = m.name("file")
            {
                (PathBuf::from(file.as_str()), Some(self.system_fs.as_ref()))
            } else {
                let mut current = Path::parent(current).map(|x| x.to_path_buf());
                for c in Path::new(&arg).components() {
                    if c.as_os_str() == "." {
                        // Do Nothing
                    } else if c.as_os_str() == ".." {
                        current = current.and_then(|x| x.parent().map(|x| x.to_path_buf()));
                    } else {
                        current = current.map(|x| x.join(c));
                    }
                }
                (current.unwrap_or(Path::new(&arg).to_path_buf()), None)
            },
        )
    }

    fn read_file_inner(
        &self,
        file: &Path,
        fs: Option<&dyn PreprocessorFilesystem>,
        level: i32,
        definitions: &mut HashMap<String, String>,
        if_statements: &mut Vec<IfState>,
    ) -> Result<PreprocessorOutput, PreprocessorError> {
        let fname: Rc<str> = file.to_str().unwrap().into();

        let file_text_orig = fs.unwrap_or(self.filesystem.as_ref()).read_file(file)?;

        let file_text = Self::remove_comments(
            file_text_orig.as_ref(),
            file.as_os_str().to_str().unwrap_or(""),
        )?;

        fn execute_line_replacements(l: &str, definitions: &HashMap<String, String>) -> String {
            static REPLACE_REGEX: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"\$\{(?<key>[a-zA-Z_][\w\d]*)\}").unwrap());
            let mut new_l = l.to_string();
            for m in REPLACE_REGEX.captures_iter(l) {
                let name = m.name("key").unwrap().as_str();
                if let Some(val) = definitions.get(name) {
                    new_l = new_l.replace(&format!("${{{name}}}"), val);
                }
            }
            new_l
        }

        let mut lines = Vec::new();

        for (i, line_unprocessed) in file_text.lines().enumerate() {
            // First, run find/replace throughout the texts
            let l = execute_line_replacements(line_unprocessed, definitions);

            // Then, look for prefixes
            if let Some(after) = l.trim_start().strip_prefix("#") {
                let verb;
                let arg;

                if let Some((first, second)) = after.split_once(' ') {
                    verb = first;
                    arg = second.trim();
                } else {
                    verb = after;
                    arg = "";
                }

                let gen_error = |s: &str| PreprocessorError {
                    loc: Some(PreprocessorLocation {
                        file: fname.clone(),
                        line: i + 1,
                    }),
                    text: l.to_string(),
                    error: s.to_string(),
                };

                if verb == "include" {
                    let (file_to_load, file_fs) = self.get_file_path(file, arg)?;
                    lines.extend(
                        self.read_file_inner(
                            &file_to_load,
                            file_fs.or(fs),
                            level + 1,
                            definitions,
                            if_statements,
                        )?
                        .lines,
                    );
                } else if verb == "ifdef" {
                    if_statements.push(IfState::new(definitions.contains_key(arg)));
                } else if verb == "ifexist" {
                    let (file_path, file_fs) = self.get_file_path(file, arg)?;
                    if_statements.push(IfState::new(
                        file_fs.or(fs).map_or(false, |x| x.file_exists(&file_path)),
                    ));
                } else if verb == "ifndef" {
                    if_statements.push(IfState::new(!definitions.contains_key(arg)));
                } else if verb == "define" {
                    if let Some((key, val)) = arg.split_once('=') {
                        definitions.insert(key.trim().into(), val.trim().into());
                    } else {
                        definitions.insert(arg.into(), String::default());
                    }
                } else if verb == "else" {
                    if !arg.is_empty() {
                        return Err(gen_error("no argument expected for line"));
                    }

                    if let Some(state) = if_statements.last_mut() {
                        if state.is_else {
                            return Err(gen_error(
                                "else statement already used for this statement",
                            ));
                        } else {
                            state.is_else = true;
                        }
                    }
                } else if verb == "endif" {
                    if !arg.is_empty() {
                        return Err(gen_error("no argument expected for line"));
                    } else if if_statements.pop().is_none() {
                        return Err(gen_error(
                            "cannot end an if statement after all statements have been applied already",
                        ));
                    }
                } else {
                    return Err(gen_error(&format!(
                        "unknown preprocessor action '{verb}' with arg '{arg}'"
                    )));
                }
            } else if if_statements.iter().all(|x| x.get_current()) {
                lines.push(PreprocessorLine {
                    text: l,
                    loc: PreprocessorLocation {
                        file: fname.clone(),
                        line: i + 1,
                    },
                });
            }
        }

        if level == 0 && !if_statements.is_empty() {
            Err(PreprocessorError {
                loc: None,
                text: String::default(),
                error: "unclosed if statements remaining after processing completion".into(),
            })
        } else {
            Ok(PreprocessorOutput { lines })
        }
    }
}

#[derive(Debug, Clone)]
pub enum FilesystemError {
    FileNotFound(PathBuf),
    UnableToLoadFile(PathBuf, String),
}

impl FilesystemError {
    fn get_path(&self) -> PathBuf {
        match self {
            Self::FileNotFound(f) => f.clone(),
            Self::UnableToLoadFile(f, _) => f.clone(),
        }
    }
}

impl Display for FilesystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileNotFound(file) => write!(f, "file \"{}\" not found", file.display()),
            Self::UnableToLoadFile(file, err) => {
                write!(f, "unable to load file \"{}\": {err}", file.display())
            }
        }
    }
}

pub trait PreprocessorFilesystem: Debug {
    fn read_file(&self, file: &Path) -> Result<Rc<str>, FilesystemError>;
    fn file_exists(&self, file: &Path) -> bool;
}

#[derive(Debug, Default)]
pub struct RealFilesystem {
    files: Rc<RefCell<HashMap<PathBuf, Rc<str>>>>,
    base: Option<PathBuf>,
}

impl RealFilesystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_relative<T: Into<PathBuf>>(base: T) -> Self {
        Self {
            base: Some(base.into().canonicalize().unwrap()),
            ..Default::default()
        }
    }
}

impl RealFilesystem {
    fn full_path(&self, file: &Path) -> Result<PathBuf, FilesystemError> {
        match self
            .base
            .as_ref()
            .map_or_else(|| file.to_path_buf(), |x| x.join(file))
            .canonicalize()
        {
            Ok(path) => Ok(path),
            Err(e) => Err(FilesystemError::UnableToLoadFile(
                file.to_path_buf(),
                e.to_string(),
            )),
        }
    }
}

impl PreprocessorFilesystem for RealFilesystem {
    fn file_exists(&self, file: &Path) -> bool {
        let stored = self.files.borrow();
        match self.full_path(file) {
            Ok(f) => stored.contains_key(&f) || f.exists(),
            Err(_) => false,
        }
    }

    fn read_file(&self, file: &Path) -> Result<Rc<str>, FilesystemError> {
        let mut stored = self.files.borrow_mut();

        let file_path = self.full_path(file)?;
        if !file_path.exists() {
            return Err(FilesystemError::FileNotFound(file_path));
        }

        match stored.entry(file_path.clone()) {
            Entry::Occupied(e) => Ok(e.get().clone()),
            Entry::Vacant(e) => {
                let txt: Rc<str> = match std::fs::read_to_string(file_path) {
                    Ok(s) => s.into(),
                    Err(e) => {
                        return Err(FilesystemError::UnableToLoadFile(
                            file.to_path_buf(),
                            e.to_string(),
                        ));
                    }
                };

                e.insert(txt.clone());
                Ok(txt)
            }
        }
    }
}

#[cfg(feature = "cbfs")]
#[derive(Debug)]
pub struct ImageFilesystem {
    fs: cbfs_lib::FileSystem,
    root: Option<cbfs_lib::SectorHandle>,
}

#[cfg(feature = "cbfs")]
impl PreprocessorFilesystem for ImageFilesystem {
    fn file_exists(&self, file: &Path) -> bool {
        self.read_file(file).is_ok()
    }

    fn read_file(&self, file: &Path) -> Result<Rc<str>, FilesystemError> {
        let mut current = self.root.unwrap_or(self.fs.root_sector());

        'next_path: for p in file.iter() {
            let current_name = p.to_str().unwrap();

            let lists = match self.fs.directory_listing(current) {
                Ok(v) => v,
                Err(_) => {
                    return Err(FilesystemError::FileNotFound(file.into()));
                }
            };

            for l in lists {
                if l.get_name() == current_name {
                    current = l.get_base_sector();
                    continue 'next_path;
                }
            }

            return Err(FilesystemError::FileNotFound(file.into()));
        }

        let (entry_header, file_data) = match self.fs.entry_data(current) {
            Ok(v) => v,
            Err(e) => {
                return Err(FilesystemError::UnableToLoadFile(
                    file.into(),
                    format!("unable to load entry data - {e}"),
                ));
            }
        };

        if entry_header.get_entry_type() != cbfs_lib::EntryType::File {
            return Err(FilesystemError::UnableToLoadFile(
                file.into(),
                format!(
                    "selected entry is not a file - {}",
                    entry_header.get_entry_type(),
                ),
            ));
        }

        match std::str::from_utf8(&file_data) {
            Ok(val) => Ok(val.into()),
            Err(e) => Err(FilesystemError::UnableToLoadFile(
                file.into(),
                format!(
                    "unable to load as utf8 '{}' - {e}",
                    file.as_os_str().to_str().unwrap_or_default()
                ),
            )),
        }
    }
}

#[derive(Debug, Default)]
pub struct OverlayFilesystem {
    pub systems: Vec<Rc<dyn PreprocessorFilesystem>>,
}

impl PreprocessorFilesystem for OverlayFilesystem {
    fn file_exists(&self, file: &Path) -> bool {
        self.systems.iter().rev().any(|x| x.file_exists(file))
    }

    fn read_file(&self, file: &Path) -> Result<Rc<str>, FilesystemError> {
        for s in self.systems.iter().rev() {
            match s.read_file(file) {
                Ok(f) => return Ok(f),
                Err(FilesystemError::FileNotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }

        Err(FilesystemError::FileNotFound(file.into()))
    }
}

#[derive(Debug, Default)]
pub struct VirtualFilesystem {
    pub files: HashMap<PathBuf, Rc<str>>,
}

impl VirtualFilesystem {
    #[cfg(feature = "cbos")]
    pub fn new_system() -> Self {
        let mut fs = Self::default();

        for (p, c) in DEFAULT_FILES {
            fs.add_file(Path::new(p), c).unwrap();
        }

        fs
    }

    pub fn new(code: &str, file: &Path) -> Self {
        let mut fs = VirtualFilesystem::default();
        fs.add_file(file, code).unwrap();
        fs
    }

    pub fn add_file(&mut self, file: &Path, code: &str) -> Result<(), FilesystemError> {
        match self.files.entry(file.to_path_buf()) {
            std::collections::hash_map::Entry::Occupied(_) => {
                Err(FilesystemError::UnableToLoadFile(
                    file.to_path_buf(),
                    "file already exists in virtual filesystem".into(),
                ))
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(code.into());
                Ok(())
            }
        }
    }
}

impl PreprocessorFilesystem for VirtualFilesystem {
    fn file_exists(&self, file: &Path) -> bool {
        self.files.contains_key(file)
    }

    fn read_file(&self, file: &Path) -> Result<Rc<str>, FilesystemError> {
        if let Some(val) = self.files.get(file) {
            Ok(val.clone())
        } else {
            Err(FilesystemError::FileNotFound(file.to_path_buf()))
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreprocessorError {
    pub loc: Option<PreprocessorLocation>,
    pub text: String,
    pub error: String,
}

impl From<FilesystemError> for PreprocessorError {
    fn from(value: FilesystemError) -> Self {
        Self {
            loc: None,
            text: value.get_path().display().to_string(),
            error: value.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreprocessorLine {
    pub text: String,
    pub loc: PreprocessorLocation,
}

#[derive(Debug, Clone)]
pub struct PreprocessorLocation {
    pub file: Rc<str>,
    pub line: usize,
}

impl Display for PreprocessorLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.file, self.line)
    }
}

impl Display for PreprocessorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CBP: ")?;
        if let Some(loc) = &self.loc {
            write!(f, "[{}] @ ", loc)?;
        }
        write!(f, "\"{}\" => {}", self.text, self.error)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct IfState {
    value: bool,
    is_else: bool,
}

impl IfState {
    fn new(value: bool) -> Self {
        Self {
            value,
            is_else: false,
        }
    }

    fn get_current(&self) -> bool {
        self.value ^ self.is_else
    }
}

pub fn preprocess_code_std<I: Iterator<Item = String>>(
    text: &str,
    defs: I,
) -> Result<PreprocessorOutput, PreprocessorError> {
    preprocess_code_as_file(text, Path::new("main.cb"), defs)
}

pub fn preprocess_code_as_file<I: Iterator<Item = String>>(
    text: &str,
    file_path: &Path,
    defs: I,
) -> Result<PreprocessorOutput, PreprocessorError> {
    preprocess_code_with_fs(file_path, VirtualFilesystem::new(text, file_path), defs)
}

pub fn preprocess_code_with_fs<I: Iterator<Item = String>>(
    file: &Path,
    fs: VirtualFilesystem,
    defs: I,
) -> Result<PreprocessorOutput, PreprocessorError> {
    let mut state = PreprocessorState::new(Rc::new(fs));
    for d in defs.into_iter() {
        state.init_definitions.insert(d, "".into());
    }
    state.read_file(file)
}

pub fn read_and_preprocess<I: Iterator<Item = String>>(
    file: &Path,
    defs: I,
) -> Result<PreprocessorOutput, PreprocessorError> {
    let mut state = PreprocessorState::new(Rc::new(RealFilesystem::default()));
    for d in defs.into_iter() {
        state.init_definitions.insert(d, "".into());
    }
    state.read_file(file)
}
