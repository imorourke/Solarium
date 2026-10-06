use cbfs_lib::{FileSystem, FileSystemError, read_container};

pub static JIB_OS_IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cbos.cbfs"));
pub static JIB_OS_DEFS: &str = include_str!(concat!(env!("OUT_DIR"), "/cbos.cb"));

pub fn get_os_image() -> Result<FileSystem, FileSystemError> {
    Ok(read_container(&mut std::io::Cursor::new(JIB_OS_IMAGE))?.1)
}
