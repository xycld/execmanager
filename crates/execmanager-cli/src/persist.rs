use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent: {}", path.display()),
        )
    })?;

    fs::create_dir_all(parent)?;

    let temp_path = unique_temp_path(parent, path.file_name().unwrap_or_default());
    let result = write_temp_and_replace(path, &temp_path, bytes);

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }

    result
}

fn write_temp_and_replace(path: &Path, temp_path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_new_file(temp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    fs::rename(temp_path, path)?;
    sync_dir(path.parent().expect("validated parent exists"))
}

fn create_new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn unique_temp_path(parent: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    parent.join(format!(
        ".{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        process::id(),
        nanos
    ))
}

fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
