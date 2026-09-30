use cbfs_lib::{
    ContainerHeader, Date, DateTime, DirectoryEntry, EntryType, FileSystem, FileSystemError,
    SectorHandle, Time, open_container, save_container,
};
use std::{
    fmt::{Debug, Display},
    path::{Path, PathBuf},
    process::ExitCode,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().into_iter().collect();
    let retcode = ffi::cxx_fuse_main(&args);
    if retcode == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[derive(Debug)]
struct CbFs {
    fs: FileSystem,
    flags: ContainerHeader,
}

#[cxx::bridge]
mod ffi {
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    enum CbFsEntryType {
        #[default]
        Unknown,
        File,
        Directory,
    }

    #[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
    pub struct CbFsTime {
        pub year: i16,
        pub month: u8,
        pub day: u8,
        pub hour: u8,
        pub minute: u8,
        pub second: u8,
        pub hundredths: u8,
    }

    #[derive(Default, Debug, Clone, Eq, PartialEq)]
    pub struct CbFsStats {
        pub num_blocks: u16,
        pub block_size: u16,
        pub free_blocks: u16,
        pub entry_blocks: u16,
        pub root_node: u16,
        pub name: String,
    }

    #[derive(Clone, Eq, PartialEq)]
    pub struct CbFsEntry {
        pub entry_id: u16,
        pub entry_type: CbFsEntryType,
        pub size_bytes: u32,
        pub size_blocks: u32,
        pub last_time: CbFsTime,
        pub name: String,
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    pub enum CbFsError {
        InvalidEntry,
        EntryNotFound,
        EntryNotDirectory,
        EntryNotFile,
        DuplicateName,
        InvalidDateTime,
        InvalidName,
        InvalidConfig,
        NoSpace,
        NullProvided,
        ContainerError,
        UnknownError,
    }

    extern "Rust" {
        fn cbfs_get_version() -> &'static str;
        fn cbfs_error_from_str(s: &str) -> CbFsError;

        fn to_millis(self: &CbFsTime) -> i64;
        #[Self = CbFsTime]
        fn from_millis(millis: i64) -> CbFsTime;
    }

    extern "Rust" {
        type CbFs;

        #[Self = CbFs]
        fn open(backing_file: &str, randomize: bool) -> Result<Box<CbFs>>;
        fn save(&mut self, backing_file: &str) -> Result<()>;

        fn get_stats(&self) -> Result<CbFsStats>;

        fn is_executable(&self, entry: u16) -> Result<bool>;
        fn set_executable(&mut self, entry: u16, setting: bool) -> Result<()>;

        fn entry_by_path(&self, path: &str) -> Result<CbFsEntry>;
        fn entry_by_id(&self, id: u16) -> Result<CbFsEntry>;

        fn read_dir(&self, dir: u16) -> Result<Vec<CbFsEntry>>;
        fn read_entry_data(&self, id: u16, offset: u32, data: &mut [u8]) -> Result<u32>;

        fn write_entry_data(&mut self, id: u16, offset: u32, data: &[u8]) -> Result<u32>;
        fn create_entry(
            &mut self,
            path: &str,
            entry_type: CbFsEntryType,
            truncate: bool,
        ) -> Result<u16>;

        fn truncate(&mut self, id: u16, size: u32) -> Result<()>;
        fn entry_set_time(&mut self, id: u16, time: &CbFsTime) -> Result<()>;
        fn entry_set_time_current(&mut self, id: u16) -> Result<()>;

        fn rename_entry(&mut self, path: &str, new_path: &str, replace: bool) -> Result<()>;
        fn remove_entry(&mut self, path: &str, entry_type: CbFsEntryType) -> Result<()>;

        fn get_parent_node(&self, node: u16) -> Result<u16>;
    }

    unsafe extern "C++" {
        include!("cbfs_fuse.hpp");

        fn cxx_fuse_main(args: &[String]) -> i32;
    }
}

impl From<EntryType> for ffi::CbFsEntryType {
    fn from(value: EntryType) -> Self {
        match value {
            EntryType::Directory => Self::Directory,
            EntryType::File => Self::File,
            _ => Self::Unknown,
        }
    }
}

impl From<ffi::CbFsEntryType> for EntryType {
    fn from(value: ffi::CbFsEntryType) -> Self {
        match value {
            ffi::CbFsEntryType::Directory => Self::Directory,
            ffi::CbFsEntryType::File => Self::File,
            _ => Self::Unknown,
        }
    }
}

/// Provides the current package version
fn cbfs_get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

impl ffi::CbFsTime {
    /// Converts the given time structure to a millisecond count
    pub fn to_millis(&self) -> i64 {
        let dt: DateTime = (*self).into();
        dt.to_posix_millis().unwrap_or_default()
    }

    /// Converts from a millisecond count to a time structure
    pub fn from_millis(millis: i64) -> Self {
        DateTime::from_posix_millis(millis)
            .map(|x| x.into())
            .unwrap_or_default()
    }
}

impl From<DateTime> for ffi::CbFsTime {
    fn from(value: DateTime) -> Self {
        Self {
            year: value.date.year.get(),
            month: value.date.month,
            day: value.date.day,
            hour: value.time.hour,
            minute: value.time.minute,
            second: value.time.second,
            hundredths: value.time.hundredths,
        }
    }
}

impl From<ffi::CbFsTime> for DateTime {
    fn from(value: ffi::CbFsTime) -> Self {
        Self {
            date: Date {
                year: value.year.into(),
                month: value.month,
                day: value.day,
            },
            time: Time {
                hour: value.hour,
                minute: value.minute,
                second: value.second,
                hundredths: value.hundredths,
            },
        }
    }
}

impl Default for ffi::CbFsEntry {
    fn default() -> Self {
        Self {
            entry_id: 0,
            name: String::default(),
            entry_type: ffi::CbFsEntryType::Unknown,
            size_blocks: 0,
            size_bytes: 0,
            last_time: ffi::CbFsTime::default(),
        }
    }
}

pub fn cbfs_error_from_str(s: &str) -> ffi::CbFsError {
    match s {
        "InvalidEntry" => ffi::CbFsError::InvalidEntry,
        "EntryNotFound" => ffi::CbFsError::EntryNotFound,
        "EntryNotDirectory" => ffi::CbFsError::EntryNotDirectory,
        "EntryNotFile" => ffi::CbFsError::EntryNotFile,
        "DuplicateName" => ffi::CbFsError::DuplicateName,
        "InvalidDateTime" => ffi::CbFsError::InvalidDateTime,
        "InvalidName" => ffi::CbFsError::InvalidName,
        "InvalidConfig" => ffi::CbFsError::InvalidConfig,
        "NoSpace" => ffi::CbFsError::NoSpace,
        "NullProvided" => ffi::CbFsError::NullProvided,
        "ContainerError" => ffi::CbFsError::ContainerError,
        _ => ffi::CbFsError::UnknownError,
    }
}

impl Display for ffi::CbFsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match *self {
                Self::InvalidEntry => "InvalidEntry",
                Self::EntryNotFound => "EntryNotFound",
                Self::EntryNotDirectory => "EntryNotDirectory",
                Self::EntryNotFile => "EntryNotFile",
                Self::DuplicateName => "DuplicateName",
                Self::InvalidDateTime => "InvalidDateTime",
                Self::InvalidName => "InvalidName",
                Self::InvalidConfig => "InvalidConfig",
                Self::NoSpace => "NoSpace",
                Self::NullProvided => "NullProvided",
                Self::ContainerError => "ContainerError",
                _ => "UnknownError",
            }
        )
    }
}

impl From<FileSystemError> for ffi::CbFsError {
    fn from(value: FileSystemError) -> Self {
        match value {
            FileSystemError::NameExists(_) => Self::DuplicateName,
            FileSystemError::EntryInvalid(_) => Self::InvalidEntry,
            FileSystemError::EntryNotDirectory(_) => Self::EntryNotDirectory,
            FileSystemError::EntryNotFile(_) => Self::EntryNotFile,
            FileSystemError::InvalidDateTime => Self::InvalidDateTime,
            FileSystemError::InvalidName => Self::InvalidName,
            FileSystemError::InvalidSectorCount(_) => Self::InvalidConfig,
            FileSystemError::NonZeroDirectoryData => Self::InvalidConfig,
            FileSystemError::SectorSizeTooSmall(_) => Self::InvalidConfig,
            FileSystemError::PathNotFound(_) => Self::EntryNotFound,
            FileSystemError::TableFull => Self::NoSpace,
            FileSystemError::UnknownEntryType(_) => Self::InvalidEntry,
            FileSystemError::ContainerError(_) => Self::ContainerError,
            FileSystemError::UnknownError(_) => Self::UnknownError,
        }
    }
}

fn get_path(p: &str) -> Option<PathBuf> {
    if p.is_empty() {
        None
    } else {
        Some(PathBuf::from(p))
    }
}

impl CbFs {
    /// Compatibility function to create a filesystem with the provided parameters
    pub fn open(backing_file: &str, randomize: bool) -> Result<Box<Self>, ffi::CbFsError> {
        let (header, mut cbfs) = if let Some(fp) = &get_path(backing_file)
            && fp.exists()
        {
            open_container(fp)?
        } else {
            return Err(ffi::CbFsError::InvalidName);
        };

        cbfs.randomize_sectors(randomize);

        Ok(Box::new(Self {
            flags: header,
            fs: cbfs,
        }))
    }

    /// Saves the provided CbFs struct to the path provided
    pub fn save(&mut self, backing_file: &str) -> Result<(), ffi::CbFsError> {
        if let Some(backing_file) = get_path(backing_file) {
            save_container(&self.flags, &self.fs, &backing_file)?;
            Ok(())
        } else {
            Err(ffi::CbFsError::NullProvided)
        }
    }

    /// Provides the resulting file system statistics
    pub fn get_stats(&self) -> Result<ffi::CbFsStats, ffi::CbFsError> {
        Ok(ffi::CbFsStats {
            num_blocks: self.fs.sector_count(),
            block_size: self.fs.sector_size(),
            entry_blocks: self.fs.base_entries.len() as u16,
            free_blocks: self.fs.num_free_sectors() as u16,
            name: self.fs.vol_name(),
            root_node: self.fs.root_sector().0,
        })
    }

    /// Determines if the provided file entry is currently executable
    pub fn is_executable(&self, entry: u16) -> Result<bool, ffi::CbFsError> {
        if let Ok(dir_entry) = self.fs.directory_entry(SectorHandle(entry)) {
            Ok(dir_entry.attributes.is_executable())
        } else {
            Err(ffi::CbFsError::EntryNotFound)
        }
    }

    /// Determines if the provided file entry is currently executable
    pub fn set_executable(&mut self, entry: u16, setting: bool) -> Result<(), ffi::CbFsError> {
        let entry = SectorHandle(entry);

        let mut dir_entry = self.fs.directory_entry(entry)?;
        dir_entry.attributes.set_executable(setting);

        self.fs.set_entry_attributes(entry, dir_entry.attributes)?;
        Ok(())
    }

    fn entry_for_path<T: AsRef<Path>>(
        &self,
        path: Option<T>,
    ) -> Result<DirectoryEntry, FileSystemError> {
        if let Some(path) = path {
            let mut current_entry = self.fs.root_directory_entry();

            if path.as_ref().has_root() {
                'parts: for p in path.as_ref().iter().skip(1) {
                    let dirs = self.fs.directory_listing(current_entry.get_base_sector())?;
                    for d in dirs {
                        if d.get_name() == p.to_str().unwrap() {
                            current_entry = d;
                            continue 'parts;
                        }
                    }

                    return Err(FileSystemError::PathNotFound(
                        p.to_str().unwrap().to_string(),
                    ));
                }

                Ok(current_entry)
            } else {
                Err(FileSystemError::InvalidName)
            }
        } else {
            Err(FileSystemError::InvalidName)
        }
    }

    /// Provides the dentry for a directory header value
    fn entry_from_dir_val(
        &self,
        dir_hdr: &DirectoryEntry,
    ) -> Result<ffi::CbFsEntry, ffi::CbFsError> {
        let hdr = self.fs.entry_header(dir_hdr.get_base_sector())?;

        Ok(ffi::CbFsEntry {
            entry_id: dir_hdr.base_block.get(),
            entry_type: dir_hdr.get_entry_type().into(),
            name: dir_hdr.get_name(),
            size_bytes: hdr.get_payload_size() as u32,
            last_time: hdr.get_modification_time().into(),
            size_blocks: self.fs.num_sectors_for_entry(dir_hdr.get_base_sector()) as u32,
        })
    }

    /// Provides the given entry header by path
    pub fn entry_by_path(&self, path: &str) -> Result<ffi::CbFsEntry, ffi::CbFsError> {
        if let Some(path) = get_path(path) {
            let dir_entry = self.entry_for_path(Some(path))?;
            self.entry_from_dir_val(&dir_entry)
        } else {
            Err(ffi::CbFsError::NullProvided)
        }
    }

    /// Provides the given entry header by entry id
    pub fn entry_by_id(&self, id: u16) -> Result<ffi::CbFsEntry, ffi::CbFsError> {
        let id = SectorHandle(id);
        let entry_hdr = self.fs.entry_header(id)?;

        let dir_entry = if entry_hdr.parent.get() == 0 {
            self.fs.root_directory_entry()
        } else {
            self.fs.directory_entry(id)?
        };

        Ok(self.entry_from_dir_val(&dir_entry)?)
    }

    /// Reads the entries provided in a directory directory entry
    pub fn read_dir(&self, dir: u16) -> Result<Vec<ffi::CbFsEntry>, ffi::CbFsError> {
        fn compat_gen(fs: &CbFs, x: DirectoryEntry) -> Result<ffi::CbFsEntry, FileSystemError> {
            let tv: ffi::CbFsTime = fs
                .fs
                .entry_header(x.get_base_sector())?
                .get_modification_time()
                .into();

            Ok(ffi::CbFsEntry {
                entry_id: x.base_block.get(),
                size_blocks: 0,
                entry_type: x.get_entry_type().into(),
                size_bytes: 0,
                name: x.get_name(),
                last_time: tv,
            })
        }

        let dir = SectorHandle(dir);
        let dirs = self.fs.directory_listing(dir)?;
        let dirs_compat = dirs
            .into_iter()
            .map(|x| compat_gen(self, x))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(dirs_compat)
    }

    /// Reads data from the provided entry in the filesystem. Set requested size to 0 for the full file size
    pub fn read_entry_data(
        &self,
        id: u16,
        offset: u32,
        data: &mut [u8],
    ) -> Result<u32, ffi::CbFsError> {
        let id = SectorHandle(id);
        let (_, entry_data) = self.fs.entry_data(id)?;

        let slice = entry_data.iter().skip(offset as usize).take(data.len());
        let size_u32 = slice.len() as u32;

        for (dst, src) in data.iter_mut().zip(slice) {
            *dst = *src;
        }

        Ok(size_u32)
    }

    /// Writes data to the provided entry in the filesystem, updating the modification time
    pub fn write_entry_data(
        &mut self,
        id: u16,
        offset: u32,
        data: &[u8],
    ) -> Result<u32, ffi::CbFsError> {
        let id = SectorHandle(id);
        let (hdr, mut entry_data) = self.fs.entry_data(id)?;

        let res_len;
        {
            let slice = entry_data.iter_mut().skip(offset as usize).take(data.len());
            res_len = slice.len() as u32;

            for (dst, src) in slice.zip(data) {
                *dst = *src;
            }
        }

        self.fs.set_entry_data(id, hdr, &entry_data)?;
        Ok(res_len)
    }

    /// Creates a new entry of the provided type at the given path
    pub fn create_entry(
        &mut self,
        path: &str,
        entry_type: ffi::CbFsEntryType,
        truncate: bool,
    ) -> Result<u16, ffi::CbFsError> {
        if let Some(path) = get_path(path) {
            if let Ok(entry) = self.entry_for_path(Some(&path)) {
                if truncate {
                    if entry_type == entry.get_entry_type().into() {
                        let id_val = entry.get_base_sector();
                        if entry.get_entry_type() == EntryType::File {
                            self.fs.set_entry_payload_byte_size(id_val, 0)?;
                        }
                        Ok(id_val.0)
                    } else {
                        Err(ffi::CbFsError::EntryNotFile)
                    }
                } else {
                    Err(ffi::CbFsError::DuplicateName)
                }
            } else if let Ok(parent) = self.entry_for_path(path.parent()) {
                let new_name = path.file_name().unwrap().to_str().unwrap();
                let id_val = self.fs.create_entry(
                    parent.get_base_sector(),
                    new_name,
                    entry_type.into(),
                    &[],
                )?;

                Ok(id_val.0)
            } else {
                Err(ffi::CbFsError::EntryNotFound)
            }
        } else {
            Err(ffi::CbFsError::NullProvided)
        }
    }

    /// Renames the provided entry
    pub fn rename_entry(
        &mut self,
        path: &str,
        new_path: &str,
        replace: bool,
    ) -> Result<(), ffi::CbFsError> {
        if let Some(path) = get_path(path)
            && let Some(new_path) = get_path(new_path)
        {
            if path == new_path {
                Ok(())
            } else if let Ok(entry) = self.entry_for_path(Some(&path))
                && let Ok(parent) = self.entry_for_path(new_path.parent())
            {
                self.fs.move_entry(
                    entry.get_base_sector(),
                    parent.get_base_sector(),
                    Some(new_path.file_name().unwrap().to_str().unwrap()),
                    replace,
                )?;
                Ok(())
            } else {
                Err(ffi::CbFsError::NullProvided)
            }
        } else {
            Err(ffi::CbFsError::NullProvided)
        }
    }

    /// Removes an entry of the specified type at the provided path from the filesystem
    pub fn remove_entry(
        &mut self,
        path: &str,
        entry_type: ffi::CbFsEntryType,
    ) -> Result<(), ffi::CbFsError> {
        if let Some(path) = get_path(path) {
            let entry = self.entry_for_path(Some(&path))?;

            if entry_type == ffi::CbFsEntryType::Unknown
                || (entry_type == entry.get_entry_type().into())
            {
                self.fs.delete_entry(entry.get_base_sector())?;
                Ok(())
            } else {
                Err(ffi::CbFsError::InvalidEntry)
            }
        } else {
            Err(ffi::CbFsError::NullProvided)
        }
    }

    /// Provides the parent of a node, if one exists
    pub fn get_parent_node(&self, node: u16) -> Result<u16, ffi::CbFsError> {
        let node = SectorHandle(node);
        if node == self.fs.root_sector() {
            Ok(0)
        } else {
            Ok(self.fs.entry_header(node)?.get_parent().0)
        }
    }

    /// Sets the entry payload size of a given entry
    pub fn truncate(&mut self, id: u16, size: u32) -> Result<(), ffi::CbFsError> {
        let id = SectorHandle(id);
        self.fs.set_entry_payload_byte_size(id, size)?;
        Ok(())
    }

    /// Sets the modification time of a given entry by id
    pub fn entry_set_time(&mut self, id: u16, time: &ffi::CbFsTime) -> Result<(), ffi::CbFsError> {
        let id = SectorHandle(id);
        self.fs.set_entry_time(id, (*time).into())?;
        Ok(())
    }

    /// Sets the modification time of a given entry by id to current
    pub fn entry_set_time_current(&mut self, id: u16) -> Result<(), ffi::CbFsError> {
        let id = SectorHandle(id);
        self.fs.set_entry_time(id, chrono::Utc::now().into())?;
        Ok(())
    }
}
