use std::fmt::Debug;

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    big_endian::{U16, U32},
};

use crate::{
    FileSystemError, SectorHandle, datetime::DateTime, names::array_to_string, string_to_array,
};

#[repr(u8)]
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum EntryType {
    #[default]
    Unknown = 0,
    Directory = 1,
    File = 2,
}

impl From<u8> for EntryType {
    fn from(value: u8) -> Self {
        const DIR_ID: u8 = EntryType::Directory as u8;
        const FILE_ID: u8 = EntryType::File as u8;

        match value {
            DIR_ID => EntryType::Directory,
            FILE_ID => EntryType::File,
            _ => EntryType::Unknown,
        }
    }
}

impl From<EntryType> for u8 {
    fn from(value: EntryType) -> Self {
        value as u8
    }
}

#[repr(C)]
#[derive(Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct PackedEntryType(u8);

impl Debug for PackedEntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", EntryType::from(self.0))
    }
}

impl From<EntryType> for PackedEntryType {
    fn from(value: EntryType) -> Self {
        Self(value.into())
    }
}

impl From<PackedEntryType> for EntryType {
    fn from(value: PackedEntryType) -> Self {
        Self::from(value.0)
    }
}

#[repr(C)]
#[repr(packed)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct EntryHeader {
    pub entry_type: PackedEntryType,
    pub reserved: u8,
    pub parent: U16,
    pub modification_time: DateTime,
    pub payload_size: U32,
}

impl EntryHeader {
    pub fn get_payload_size(&self) -> usize {
        self.payload_size.get() as usize
    }

    pub fn set_payload_size(&mut self, s: usize) {
        self.payload_size = U32::new(s as u32);
    }

    pub const fn get_header_size(&self) -> usize {
        std::mem::size_of::<EntryHeader>()
    }

    pub fn get_parent(&self) -> SectorHandle {
        SectorHandle(self.parent.get())
    }

    pub fn get_total_size(&self) -> usize {
        self.get_payload_size() + self.get_header_size()
    }

    pub fn get_entry_type(&self) -> EntryType {
        EntryType::from(self.entry_type)
    }

    pub fn get_modification_time(&self) -> DateTime {
        self.modification_time
    }

    pub fn set_modification_time(&mut self, time: DateTime) {
        self.modification_time = time;
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct DirectoryAttributes(u8);

impl DirectoryAttributes {
    pub const MASK_EXECUTABLE: u8 = (1 << 0) as u8;

    pub const fn is_executable(&self) -> bool {
        self.get_mask(Self::MASK_EXECUTABLE)
    }

    pub const fn set_executable(&mut self, val: bool) {
        self.set_mask(Self::MASK_EXECUTABLE, val);
    }

    const fn get_mask(&self, mask: u8) -> bool {
        (self.0 & mask) != 0
    }

    const fn set_mask(&mut self, mask: u8, val: bool) {
        self.0 = if val { self.0 | mask } else { self.0 & !mask }
    }
}

impl From<u8> for DirectoryAttributes {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<DirectoryAttributes> for u8 {
    fn from(value: DirectoryAttributes) -> Self {
        value.0
    }
}

#[repr(C)]
#[repr(packed)]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
pub struct DirectoryEntry {
    pub base_block: U16,
    pub attributes: DirectoryAttributes,
    pub entry_type: PackedEntryType,
    pub name: [u8; Self::DIRECTORY_NAME_SIZE],
}

impl DirectoryEntry {
    pub const DIRECTORY_NAME_SIZE: usize = 60;

    pub fn get_base_sector(&self) -> SectorHandle {
        SectorHandle(self.base_block.get())
    }

    pub fn get_entry_type(&self) -> EntryType {
        EntryType::from(self.entry_type)
    }

    pub fn get_name(&self) -> String {
        array_to_string(&self.name)
    }

    pub fn get_name_raw(&self) -> [u8; Self::DIRECTORY_NAME_SIZE] {
        self.name
    }

    pub fn set_name(&mut self, s: &str) -> Result<(), FileSystemError> {
        self.name = string_to_array(s)?;
        Ok(())
    }
}
