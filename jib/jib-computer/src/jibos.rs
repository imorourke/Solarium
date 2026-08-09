use cbfs_lib::{FileSystem, SectorHandle, VolumeHeader};
use cblang::{
    CodeGenerationOptions, CompileResults, Compiler, ProgramType, preprocessor::VirtualFilesystem,
};
use std::{
    collections::HashMap,
    format,
    path::{Component, Path},
    rc::Rc,
    vec::Vec,
};

use crate::ComputerError;

#[derive(Debug, Clone)]
pub struct JibOsImage {
    pub kernel: Vec<u8>,
    pub kernel_header: String,
    pub applications: Vec<(String, Vec<u8>, ApplicationCategory)>,
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum ApplicationCategory {
    #[default]
    PathBinary,
    SampleApplication,
    Test,
}

pub struct JibApplication {
    pub exec: &'static str,
    pub filename: &'static str,
    pub code: &'static str,
    pub category: ApplicationCategory,
}

macro_rules! os_dir {
    ($p:expr) => {
        concat!("../../../cbos/", $p)
    };
}

impl JibOsImage {
    /// OS Code
    pub const CODE_OS: &str = include_str!(os_dir!("os.cb"));

    /// Definitions name
    const DEFS_FILENAME: &str = "cbos_defs.cb";

    /// Application Code
    pub const CODE_APP_BIN: &[JibApplication] = &[
        JibApplication {
            exec: "hello",
            filename: "hello.cb",
            code: include_str!(os_dir!("bin/sample/hello.cb")),
            category: ApplicationCategory::SampleApplication,
        },
        JibApplication {
            exec: "hello_mem",
            filename: "hello_mem.cb",
            code: include_str!(os_dir!("bin/sample/hello_mem.cb")),
            category: ApplicationCategory::SampleApplication,
        },
        JibApplication {
            exec: "cat",
            filename: "cat.cb",
            code: include_str!(os_dir!("bin/cat.cb")),
            category: ApplicationCategory::PathBinary,
        },
        JibApplication {
            exec: "echo",
            filename: "echo.cb",
            code: include_str!(os_dir!("bin/echo.cb")),
            category: ApplicationCategory::PathBinary,
        },
        JibApplication {
            exec: "nm",
            filename: "nm.cb",
            code: include_str!(os_dir!("bin/nm.cb")),
            category: ApplicationCategory::PathBinary,
        },
        JibApplication {
            exec: "stat",
            filename: "stat.cb",
            code: include_str!(os_dir!("bin/stat.cb")),
            category: ApplicationCategory::PathBinary,
        },
        JibApplication {
            exec: "math_test",
            filename: "math_test.cb",
            code: include_str!(os_dir!("bin/test/math_test.cb")),
            category: ApplicationCategory::Test,
        },
    ];

    /// Get the build date
    const BUILD_DATE: &'static str = env!("BUILD_DATE");

    pub fn compile_kernel_code(
        code: &str,
        name: &str,
        start_offset: Option<u32>,
        trim_code: bool,
    ) -> Result<CompileResults, ComputerError> {
        let mut defs = HashMap::new();
        defs.insert("K_OS_VER".into(), env!("CARGO_PKG_VERSION").into());

        let compiler = Compiler {
            definitions: defs,
            options: CodeGenerationOptions {
                prog_type: ProgramType::Kernel {
                    stack_loc_init: Some(ProgramType::DEFAULT_STACK_LOC),
                    base_location: start_offset.unwrap_or(ProgramType::DEFAULT_START_OFFSET),
                },
                trim_code,
                ..Default::default()
            },
            ..Default::default()
        };

        Ok(compiler.compile_string(code, Some(Path::new(name)))?)
    }

    pub fn compile_app_code(
        &self,
        code: &str,
        name: Option<&str>,
    ) -> Result<CompileResults, ComputerError> {
        const DEFAULT_NAME: &str = "main.cb";

        let compiler = Compiler {
            options: CodeGenerationOptions {
                prog_type: ProgramType::Application,
                trim_code: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let name_val = name.unwrap_or(DEFAULT_NAME);
        let path_val = Path::new(name_val);

        let mut fs = VirtualFilesystem::default();
        fs.add_file(path_val, code)?;
        fs.add_file(Path::new(Self::DEFS_FILENAME), &self.kernel_header)?;

        Ok(compiler.compile_fs(path_val, Rc::new(fs))?)
    }

    pub fn compile_os_image() -> Result<JibOsImage, ComputerError> {
        // Compile OS into a file
        let kernel_compiled = Self::compile_kernel_code(Self::CODE_OS, "os.cb", None, false)?;
        let kernel_data = kernel_compiled.binary;

        // Obtain the default interface value
        let mut interface_data = Vec::new();
        {
            let mut writer = std::io::BufWriter::new(&mut interface_data);
            kernel_compiled
                .export_interface
                .zero_offsets()
                .write_interface(&mut writer)?;
        }

        const CBOS_INTF_GUARD: &str = "CBOS_DEFS";
        let interface_str = match String::from_utf8(interface_data) {
            Ok(x) => format!(
                "#ifndef {CBOS_INTF_GUARD}\n#define {CBOS_INTF_GUARD}\n{x}\n#endif // {CBOS_INTF_GUARD}\n"
            ),
            Err(_) => return Err(ComputerError::Utf8Error),
        };

        let mut os_image = JibOsImage {
            kernel: kernel_data,
            kernel_header: interface_str,
            applications: Vec::new(),
        };

        for app in Self::CODE_APP_BIN {
            os_image.applications.push((
                app.exec.into(),
                os_image
                    .compile_app_code(app.code, Some(app.filename))?
                    .binary,
                app.category,
            ));
        }

        Ok(os_image)
    }

    fn build_datetime() -> cbfs_lib::DateTime {
        let (year, rest) = Self::BUILD_DATE.split_once('-').unwrap();
        let (month, rest) = rest.split_once('-').unwrap();
        let (day, rest) = rest.split_once(' ').unwrap();
        let (hour, rest) = rest.split_once(':').unwrap();
        let (minute, second) = rest.split_once(':').unwrap();

        cbfs_lib::DateTime {
            date: cbfs_lib::Date {
                year: year.parse().unwrap_or(0i16).into(),
                month: month.parse().unwrap_or(0),
                day: day.parse().unwrap_or(0),
            },
            time: cbfs_lib::Time {
                hour: hour.parse().unwrap_or(0),
                minute: minute.parse().unwrap_or(0),
                second: second.parse().unwrap_or(0),
                hundredths: 0,
            },
        }
    }

    fn set_executable_attribute(
        fs: &mut FileSystem,
        entry: SectorHandle,
    ) -> Result<(), ComputerError> {
        let mut dir_vals = fs.directory_entry(entry)?;
        dir_vals.attributes.set_executable(true);
        fs.set_entry_attributes(entry, dir_vals.attributes)?;
        Ok(())
    }

    pub fn create_hard_drive(&self) -> Result<FileSystem, ComputerError> {
        let mut fs = FileSystem::new("cbos", 256, 4096)?;

        fs.create_file(fs.root_sector(), "boot.bin", &self.kernel)?;

        let home_dir = fs.create_directory(fs.root_sector(), "home")?;
        let bin_dir = fs.create_directory(fs.root_sector(), "bin")?;
        let test_dir = fs.create_directory(fs.root_sector(), "test")?;
        let sample_dir = fs.create_directory(fs.root_sector(), "samples")?;

        fs.create_file(home_dir, "hello.txt", b"Welcome to CB/OS!\n")?;

        for (name, binary, category) in self.applications.iter() {
            let entry = fs.create_file(
                match category {
                    ApplicationCategory::PathBinary => bin_dir,
                    ApplicationCategory::Test => test_dir,
                    ApplicationCategory::SampleApplication => sample_dir,
                },
                name,
                binary,
            )?;
            Self::set_executable_attribute(&mut fs, entry)?;
        }

        let test_script = fs.create_file(
            home_dir,
            "script.run",
            b"#!sh\n\ndate\nmem\n\npwd\ncat hello.txt\ncat hello.txt",
        )?;
        Self::set_executable_attribute(&mut fs, test_script)?;

        let src = fs.create_directory(fs.root_sector(), "src")?;

        fs.create_file(src, "os.cb", Self::CODE_OS.as_bytes())?;
        fs.create_file(src, Self::DEFS_FILENAME, self.kernel_header.as_bytes())?;

        let src_bin = fs.create_directory(src, "bin")?;
        for app in Self::CODE_APP_BIN {
            fs.create_file(src_bin, app.filename, app.code.as_bytes())?;
        }

        for (path, code) in cblang::preprocessor::DEFAULT_FILES.iter() {
            let mut current_dir = src;
            let path_val = Path::new(path);

            for p in path_val.parent().unwrap().components() {
                if let Component::Normal(os_name) = &p
                    && let Some(name) = os_name.to_str()
                {
                    current_dir = if let Some(existing) = fs
                        .directory_listing(current_dir)?
                        .iter()
                        .find(|x| x.get_name() == name)
                    {
                        existing.get_base_sector()
                    } else {
                        fs.create_directory(current_dir, name)?
                    };
                } else {
                    panic!("unsupported name value");
                }
            }

            if let Some(name) = path_val.file_name().and_then(|x| x.to_str())
                && name.len() < VolumeHeader::VOLUME_NAME_SIZE
            {
                fs.create_file(current_dir, name, code.as_bytes())?;
            }
        }

        let nodes = fs.base_entries.clone();
        for n in nodes {
            fs.set_entry_time(n, JibOsImage::build_datetime())?;
        }

        Ok(fs)
    }
}
