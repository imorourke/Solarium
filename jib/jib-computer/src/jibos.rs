use cbfs_lib::{FileSystem, SectorHandle, VolumeHeader};
use cblang::{CodeGenerationOptions, CompilingState, ProgramType, VirtualFilesystem};
use std::{
    format,
    path::{Component, Path},
    vec::Vec,
};

use crate::ComputerError;

#[derive(Debug, Clone)]
pub struct JibOsImage {
    pub kernel: Vec<u8>,
    pub kernel_header: String,
    pub applications: Vec<(String, Vec<u8>)>,
}

struct JibApplication {
    exec: &'static str,
    filename: &'static str,
    code: &'static str,
}

impl JibOsImage {
    /// OS Code
    const CODE_OS: &str = include_str!("../../../cbos/os.cb");

    /// Application Code
    const CODE_APPS: &[JibApplication] = &[
        JibApplication {
            exec: "hello",
            filename: "hello.cb",
            code: include_str!("../../../cbos/bin/hello.cb"),
        },
        JibApplication {
            exec: "hello_mem",
            filename: "hello_mem.cb",
            code: include_str!("../../../cbos/bin/hello_mem.cb"),
        },
        JibApplication {
            exec: "cat",
            filename: "cat.cb",
            code: include_str!("../../../cbos/bin/cat.cb"),
        },
        JibApplication {
            exec: "echo",
            filename: "echo.cb",
            code: include_str!("../../../cbos/bin/echo.cb"),
        },
        JibApplication {
            exec: "nm",
            filename: "nm.cb",
            code: include_str!("../../../cbos/bin/nm.cb"),
        },
        JibApplication {
            exec: "stat",
            filename: "stat.cb",
            code: include_str!("../../../cbos/bin/stat.cb"),
        },
    ];

    /// Get the build date
    const BUILD_DATE: &'static str = env!("BUILD_DATE");

    pub fn compile_kernel_code(
        code: &str,
        start_offset: Option<u32>,
        trim_code: bool,
    ) -> Result<CompilingState, ComputerError> {
        let preprocessed =
            cblang::preprocess_code_as_file(code, Path::new("input.cb"), [].into_iter())?;

        let tokens = preprocessed.tokenize()?;

        let options = CodeGenerationOptions {
            prog_type: ProgramType::Kernel {
                stack_loc_init: Some(ProgramType::DEFAULT_STACK_LOC),
                base_location: start_offset.unwrap_or(ProgramType::DEFAULT_START_OFFSET),
            },
            trim_code,
            ..Default::default()
        };

        Ok(cblang::compile(tokens, options)?)
    }

    pub fn compile_app_code(&self, code: &str) -> Result<CompilingState, ComputerError> {
        let mut fs = VirtualFilesystem::default();
        fs.add_file(Path::new("main.cb"), code)?;
        fs.add_file(Path::new("cbos_defs.cb"), &self.kernel_header)?;

        let preprocessed =
            cblang::preprocess_code_with_fs(Path::new("main.cb"), fs, [].into_iter())?;

        let tokens = preprocessed.tokenize()?;

        let options = CodeGenerationOptions {
            prog_type: ProgramType::Application,
            trim_code: true,
            ..Default::default()
        };

        Ok(cblang::compile(tokens, options)?)
    }

    pub fn compile_os_image() -> Result<JibOsImage, ComputerError> {
        // Compile OS into a file
        let kernel_compiled = Self::compile_kernel_code(Self::CODE_OS, None, false)?;
        let kernel_data = kernel_compiled.get_binary()?;

        // Obtain the default interface value
        let mut interface_data = Vec::new();
        {
            let mut writer = std::io::BufWriter::new(&mut interface_data);
            kernel_compiled
                .get_export_interface()?
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

        for app in Self::CODE_APPS {
            os_image.applications.push((
                app.exec.into(),
                os_image.compile_app_code(app.code)?.get_binary()?,
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

        fs.create_file(home_dir, "hello.txt", b"Welcome to CB/OS!\n")?;

        for (name, binary) in self.applications.iter() {
            let entry = fs.create_file(bin_dir, name, binary)?;
            Self::set_executable_attribute(&mut fs, entry)?;
        }

        fs.create_file(
            home_dir,
            "script.run",
            b"date\nmem\n\npwd\ncat hello.txt\ncat hello.txt",
        )?;

        let src = fs.create_directory(fs.root_sector(), "src")?;

        fs.create_file(src, "os.cb", Self::CODE_OS.as_bytes())?;
        fs.create_file(src, "cbos_defs.cb", self.kernel_header.as_bytes())?;

        for app in Self::CODE_APPS {
            fs.create_file(src, app.filename, app.code.as_bytes())?;
        }

        for (path, code) in cblang::DEFAULT_FILES.iter() {
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
