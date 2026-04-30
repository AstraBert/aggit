use std::{
    fmt, fs,
    path::{self, PathBuf},
};

use flate2::{Compression, read::ZlibEncoder};
use sha1_checked::{Digest, Sha1};
use std::io::Read;

const HASH_SEPARATOR: &str = "\x00";

/// Data for one entry in the git index (.git/index)
pub struct IndexEntry {
    /// ctime seconds (Unix timestamp)
    ctime_s: u32,
    /// ctime in nanoseconds
    ctime_n: u32,
    /// mtime seconds (Unix timestamp)
    mtime_s: u32,
    /// mtime in nanoseconds
    mtime_n: u32,
    /// Device number
    dev: u32,
    /// Inode number
    ino: u32,
    /// File mode/permissions
    mode: u32,
    /// Owner user ID
    uid: u32,
    /// Owner group ID
    gid: u32,
    /// File size in bytes
    size: u32,
    /// 20-bytes raw SHA1 hash
    sha1: [u8; 20],
    /// Git index flags
    flags: u16,
    /// File path relative to the repo root
    path: PathBuf,
}

/// Object type enum. There are other types too, but we don't need them.
/// See "enum object_type" in git's source (git/cache.h).
pub enum ObjectTipe {
    Commit,
    Tree,
    Blob,
}

impl fmt::Display for ObjectTipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Blob => "blob",
            Self::Commit => "commit",
            Self::Tree => "tree",
        };
        write!(f, "{}", s)
    }
}

/// Read file bytes
fn read_file(path: &PathBuf) -> Result<Vec<u8>, std::io::Error> {
    fs::read(path)
}

/// Write file with bytes data
fn write_file(path: &PathBuf, data: Vec<u8>) -> Result<(), std::io::Error> {
    fs::write(path, data)
}

/// Create directory for repo (if it does not already exist) and initialize .git directory
pub fn init(repository: PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(&repository)?;
    fs::create_dir_all(&repository.join(".git"))?;
    let git_folders: [&str; 3] = ["object", "refs", "refs/heads"];
    for g in git_folders {
        fs::create_dir_all(&repository.join(".git").join(g))?;
    }
    write_file(
        &repository.join(".git").join("HEAD"),
        "ref: refs/heads/main".into(),
    )?;
    println!(
        "\x1b[1;93mRepository {:?} successfully initialized",
        &repository
    );

    Ok(())
}

fn compress(data: &mut Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(&data[..], Compression::default());
    let mut compressed = Vec::new();
    encoder.read_to_end(&mut compressed)?;
    Ok(compressed)
}

pub fn hash_object(data: &mut Vec<u8>, object_type: ObjectTipe, write: bool) -> anyhow::Result<()> {
    let mut full_data = format!("{} {:?}", &object_type, &data.len()).into_bytes();
    full_data.append(&mut HASH_SEPARATOR.into());
    full_data.append(data);
    let sha1 = hex::encode(Sha1::digest(&full_data));
    if write {
        let path = PathBuf::from(".git")
            .join("objects")
            .join(&sha1[..2])
            .join(&sha1[2..]);
        if !fs::exists(&path)? {
            let dir = &path.parent().unwrap();
            fs::create_dir_all(dir)?;
            let compressed = compress(&mut full_data)?;
            write_file(&path, compressed)?;
        }
    }
    Ok(())
}
