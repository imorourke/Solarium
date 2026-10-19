use cbfs_lib::{FileSystem, FileSystemError, read_container};

static CBOS_IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cbos.cbfs"));
pub static CBOS_DEFS: &str = include_str!(concat!(env!("OUT_DIR"), "/cbos.cb"));

pub fn get_os_image() -> Result<FileSystem, FileSystemError> {
    Ok(read_container(&mut std::io::Cursor::new(CBOS_IMAGE))?.1)
}
