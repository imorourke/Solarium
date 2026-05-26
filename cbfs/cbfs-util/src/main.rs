use cbfs_lib::{
    CbContainerOptions, ContainerHeader, EntryType, FileSystem, FileSystemError, open_container,
    save_container,
};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Arguments {
    #[command(subcommand)]
    options: CommandOptions,
}

#[derive(Debug, Subcommand)]
enum CommandOptions {
    Create(CreateOptions),
    Modify(ModifyOptions),
    List(ListOptions),
    Info(InfoOptions),
    Extract(ExtractOptions),
    Archive(ArchiveOptions),
}

#[derive(Debug, Parser)]
struct CreateOptions {
    file: PathBuf,
    #[arg(
        short = 's',
        long = "secsize",
        help = "sector size in bytes",
        default_value_t = 256
    )]
    secsize: u16,
    #[arg(
        short = 'S',
        long = "seccount",
        help = "sector count",
        default_value_t = 32768
    )]
    seccount: u16,
    #[arg(short = 'g', long = "gz", help = "enable gzip compression")]
    gzip: bool,
    #[arg(short = 'c', long = "sparse", help = "enable sparse file support")]
    sparse: bool,
    #[arg(short = 'n', long = "name", help = "filesystem name")]
    name: Option<String>,
}

#[derive(Debug, Parser)]
struct ModifyOptions {
    #[arg(help = "image file to modify")]
    file: PathBuf,
    #[arg(short = 'g', long = "gz", help = "enable gzip compression")]
    gzip: bool,
    #[arg(short = 'c', long = "sparse", help = "enable sparse file support")]
    sparse: bool,
    #[arg(short = 'z', long = "zero", help = "zero unused sectors")]
    zero: bool,
    #[arg(short = 'n', long = "name", help = "filesystem name")]
    name: Option<String>,
}

#[derive(Debug, Parser)]
struct ListOptions {
    #[arg(help = "image file to read")]
    file: PathBuf,
    #[arg(short = 'f', long = "files", help = "list contained files")]
    files: bool,
    #[arg(short = 'F', long = "folders", help = "list contained folder")]
    folders: bool,
}

#[derive(Debug, Parser)]
struct InfoOptions {
    #[arg(help = "image file to read")]
    file: PathBuf,
}

#[derive(Debug, Parser)]
struct ExtractOptions {
    #[arg(help = "cbfs file to extract")]
    file: PathBuf,
    #[arg(help = "output directory to extract to")]
    output: PathBuf,
}

#[derive(Debug, Parser)]
struct ArchiveOptions {
    #[arg(help = "input directory to use as the archive template")]
    input: PathBuf,
    #[arg(help = "cbfs file to create")]
    file: PathBuf,
    #[arg(short = 'g', long = "gz", help = "enable gzip compression")]
    gzip: bool,
    #[arg(short = 'c', long = "sparse", help = "enable sparse file support")]
    sparse: bool,
    #[arg(
        short = 's',
        long = "secsize",
        help = "sector size in bytes",
        default_value_t = 256
    )]
    secsize: u16,
    #[arg(
        short = 'S',
        long = "seccount",
        help = "sector count",
        default_value_t = 32768
    )]
    seccount: u16,
}

fn main() {
    let args = Arguments::parse();
    match args.options {
        CommandOptions::Create(opt) => {
            let name = opt.name.as_deref().unwrap_or("cbfs");
            let fs = FileSystem::new(name, opt.secsize, opt.seccount)
                .expect("unable to create filesystem");
            let header = ContainerHeader::new(CbContainerOptions {
                sparse: opt.sparse,
                compressed: opt.gzip,
            });
            save_container(&header, &fs, &opt.file).expect("unable to write fs to a file");
        }
        CommandOptions::Modify(opt) => {
            let (mut header, mut filesystem) =
                open_container(&opt.file).expect("unable to open file");
            header.set_options(CbContainerOptions {
                sparse: opt.sparse,
                compressed: opt.gzip,
            });
            if opt.zero {
                filesystem
                    .zero_unused_sectors()
                    .expect("unable to zero sectors");
            }
            if let Some(n) = opt.name.as_ref() {
                filesystem
                    .set_vol_name(n)
                    .expect("unable to set filesystem name");
            }
            save_container(&header, &filesystem, &opt.file).expect("unable to write fs to file");
        }
        CommandOptions::List(opt) => {
            let (_, fs) = open_container(&opt.file).expect("unable to open file");

            fn folder_entry_vals(
                path_so_far: &str,
                fs: &FileSystem,
                node: u16,
                opt: &ListOptions,
            ) -> Result<(), FileSystemError> {
                if opt.folders {
                    if node == fs.root_sector() {
                        println!("/");
                    } else {
                        println!("{path_so_far}");
                    }
                }
                for n in fs.directory_listing(node)? {
                    match n.get_entry_type() {
                        EntryType::File if opt.files => {
                            println!("{path_so_far}/{}", n.get_name())
                        }

                        EntryType::Directory => {
                            folder_entry_vals(
                                &format!("{path_so_far}/{}", n.get_name()),
                                fs,
                                n.base_block.get(),
                                opt,
                            )?;
                        }
                        _ => (),
                    }
                }

                Ok(())
            }

            folder_entry_vals("", &fs, fs.root_sector(), &opt).expect("unable to list entries")
        }
        CommandOptions::Info(opt) => {
            let (header, filesystem) = open_container(&opt.file).expect("unable to open file");

            print!("Container: ");

            let mut container_flags = Vec::new();
            let opts = header.get_options();
            if opts.compressed {
                container_flags.push("Compressed");
            }
            if opts.sparse {
                container_flags.push("Sparse");
            }
            if container_flags.is_empty() {
                container_flags.push("Raw");
            }
            println!("{}", container_flags.join(", "));

            println!("Sector Info:");

            println!("  {} byte sectors", filesystem.sector_size(),);
            println!("  {} sectors", filesystem.sector_count(),);

            println!(
                "  {} total bytes",
                filesystem.sector_size() as u32 * filesystem.sector_count() as u32
            );

            println!(
                "  {} / {} free sectors",
                filesystem.num_free_sectors(),
                filesystem.sector_count()
            );
            println!("  {} root entries", filesystem.num_primary_entries(),)
        }
        CommandOptions::Extract(opt) => {
            if !opt.output.exists() {
                std::fs::create_dir(&opt.output).expect("unable to create output directory");
            } else if opt.output.is_file() {
                panic!("base directory must be a directory entry");
            } else if opt.output.is_dir() {
                std::fs::remove_dir_all(&opt.output).expect("unable to delete output directory");
                std::fs::create_dir(&opt.output).expect("unable to create output directory");
            }

            let (hdr, fs) = cbfs_lib::open_container(&opt.file).expect("unable to open filesystem");
            hdr.check_data()
                .expect("inconsistent CBFS container header");

            fn write_entries(
                fs: &FileSystem,
                current_path: &Path,
                current_directory: u16,
            ) -> Result<(), cbfs_lib::FileSystemError> {
                for n in fs.directory_listing(current_directory)? {
                    let path = current_path.join(n.get_name());
                    match n.get_entry_type() {
                        EntryType::Directory => {
                            std::fs::create_dir(&path).expect("unable to create directory");
                            write_entries(fs, &path, n.base_block.get())?;
                        }
                        EntryType::File => {
                            let (_, data) = fs.entry_data(n.base_block.get())?;
                            std::fs::write(&path, data).expect("unable to write data for file");
                        }
                        _ => panic!("unexpected entry data type"),
                    }
                }

                Ok(())
            }

            write_entries(&fs, &opt.output, fs.root_sector())
                .expect("unable to extract root folder");
        }
        CommandOptions::Archive(opt) => {
            if !opt.input.exists() || !opt.input.is_dir() {
                panic!("input directory must exist");
            }

            let mut fs = cbfs_lib::FileSystem::new("cbfs", opt.secsize, opt.seccount)
                .expect("unable to create filesystem");

            fn read_fs_entries(
                fs: &mut FileSystem,
                current_path: &Path,
                current_directory: u16,
            ) -> Result<(), cbfs_lib::FileSystemError> {
                for n in current_path
                    .read_dir()
                    .expect("unable to get directory listing")
                {
                    let entry = n.expect("unable to get directory entry");
                    let name = entry
                        .file_name()
                        .as_os_str()
                        .to_str()
                        .expect("unable to get entry name")
                        .to_string();
                    let path = current_path.join(&name);

                    if entry.path().is_dir() {
                        let new_block = fs
                            .create_entry(current_directory, &name, EntryType::Directory, &[])
                            .unwrap();
                        read_fs_entries(fs, &path, new_block)?;
                    } else if entry.path().is_file() {
                        let data = std::fs::read(path).expect("unable to read file data");
                        fs.create_entry(current_directory, &name, EntryType::File, &data)
                            .unwrap();
                    } else {
                        panic!("unexpected entry data type");
                    }
                }

                Ok(())
            }

            let root = fs.root_sector();
            read_fs_entries(&mut fs, &opt.input, root).expect("unable to create fs");

            cbfs_lib::save_container(
                &ContainerHeader::new(CbContainerOptions {
                    compressed: opt.gzip,
                    sparse: opt.sparse,
                }),
                &fs,
                &opt.file,
            )
            .unwrap();
        }
    }
}
