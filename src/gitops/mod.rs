extern crate difflib;

use anyhow::anyhow;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use flate2::{
    Compression,
    read::{ZlibDecoder, ZlibEncoder},
};
use sha1_checked::{Digest, Sha1};
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{Cursor, Read};
use std::{fmt, fs, path::PathBuf, str::FromStr};
use walkdir::WalkDir;

const HASH_SEPARATOR: &str = "\x00";

/// Data for one entry in the git index (.git/index)
#[derive(Debug)]
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
    path: String,
}

/// Object type enum. There are other types too, but we don't need them.
/// See "enum object_type" in git's source (git/cache.h).
#[derive(PartialEq, Eq, Debug)]
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

impl FromStr for ObjectTipe {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "blob" => Ok(Self::Blob),
            "commit" => Ok(Self::Commit),
            "tree" => Ok(Self::Tree),
            _ => anyhow::bail!("unknown object type: {}", s),
        }
    }
}

/// Different mode to cat a file
pub enum CatFileMode {
    /// A mode that overlaps with the ObjectType enum
    ObjType,
    /// Print the size of the object
    Size,
    /// Print the type of the object
    Type,
    /// Pretty-print the object
    Pretty,
}

impl FromStr for CatFileMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "blob" => Ok(Self::ObjType),
            "commit" => Ok(Self::ObjType),
            "tree" => Ok(Self::ObjType),
            "size" => Ok(Self::Size),
            "type" => Ok(Self::Type),
            "pretty" => Ok(Self::Pretty),
            _ => anyhow::bail!("unknown cat file mode: {}", s),
        }
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

fn decompress(data: &mut Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(&data[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

pub fn hash_object(
    data: &mut Vec<u8>,
    object_type: ObjectTipe,
    write: bool,
) -> anyhow::Result<String> {
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
    Ok(sha1)
}

/// Find object with given SHA-1 prefix and return path to object in object
/// store, or return an error if there are no objects or multiple objects
/// with this prefix.
pub fn find_object(sha1_prefix: &str) -> anyhow::Result<PathBuf> {
    if sha1_prefix.len() < 2 {
        return Err(anyhow!("Invalid SHA1 prefix (less than 2 letters)"));
    }
    let obj_dir = PathBuf::from(".git")
        .join("objects")
        .join(&sha1_prefix[..2]);
    let rest = &sha1_prefix[2..];
    let mut objects = Vec::new();
    for entry in fs::read_dir(&obj_dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .to_string()
            .starts_with(&rest)
        {
            objects.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    if objects.is_empty() {
        return Err(anyhow!("Object {} not found", sha1_prefix));
    }
    if objects.len() >= 2 {
        return Err(anyhow!(
            "Multiple objects ({:?}) with prefix {}",
            objects.len(),
            sha1_prefix
        ));
    }
    let path = obj_dir.join(&objects[0]);

    Ok(path)
}

/// Read object with given SHA-1 prefix and return tuple of
/// (object_type, data_bytes), or raise ValueError if not found.
pub fn read_object(sha1_prefix: &str) -> anyhow::Result<(ObjectTipe, Vec<u8>)> {
    let path = find_object(sha1_prefix)?;
    let full_data = decompress(&mut read_file(&path)?)?;
    let nul_index = full_data
        .iter()
        .position(|x| *x == HASH_SEPARATOR.as_bytes()[0]);
    if let Some(idx) = nul_index {
        let mut header = &full_data[..idx];
        let mut header_str = String::new();
        header.read_to_string(&mut header_str)?;
        let spliced: Vec<&str> = header_str.split_ascii_whitespace().collect();
        if spliced.len() != 2 {
            return Err(anyhow!(
                "Expected header to contain exactly two pieces (object type, data length), it contains {:?}",
                spliced.len()
            ));
        }
        let object_type = ObjectTipe::from_str(spliced[0])?;
        let size = spliced[1].parse::<usize>()?;
        let data = &full_data[idx + 1..];
        if size != data.len() {
            return Err(anyhow!(
                "Expected size of {:?}, got {:?} bytes",
                size,
                data.len()
            ));
        }
        return Ok((object_type, data.to_vec()));
    }

    Err(anyhow!(
        "Could not find the hash separator pattern ({}) within the data",
        HASH_SEPARATOR
    ))
}

///Write the contents of (or info about) object with given SHA-1 prefix to
///stdout. If mode is 'commit', 'tree', or 'blob', print raw data bytes of
///object. If mode is 'size', print the size of the object. If mode is
///'type', print the type of the object. If mode is 'pretty', print a
///prettified version of the object.
pub fn cat_file(mode: &str, sha1_prefix: &str) -> anyhow::Result<()> {
    let obj_data = read_object(sha1_prefix)?;
    let valid_mode = CatFileMode::from_str(mode)?;
    match valid_mode {
        CatFileMode::ObjType => {
            if obj_data.0 != ObjectTipe::from_str(mode)? {
                return Err(anyhow!(
                    "Expected object type: {}, got {}",
                    obj_data.0,
                    mode
                ));
            }
            let mut str_data = String::new();
            obj_data.1.as_slice().read_to_string(&mut str_data)?;
            println!("{}", str_data);
        }
        CatFileMode::Size => {
            println!("{:?}", obj_data.1.len());
        }
        CatFileMode::Type => {
            println!("{}", obj_data.0);
        }
        CatFileMode::Pretty => match obj_data.0 {
            ObjectTipe::Blob => {
                let mut str_data = String::new();
                obj_data.1.as_slice().read_to_string(&mut str_data)?;
                println!("{}", str_data);
            }
            ObjectTipe::Commit => {
                let mut str_data = String::new();
                obj_data.1.as_slice().read_to_string(&mut str_data)?;
                println!("{}", str_data);
            }
            ObjectTipe::Tree => {} // TODO: handle tree when read_tree function is ready
        },
    }
    Ok(())
}

/// Read git index file and return list of IndexEntry objects.
pub fn read_index() -> anyhow::Result<Vec<IndexEntry>> {
    let data = match read_file(&PathBuf::from(".git/index")) {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };

    // Validate SHA1 checksum
    let (body, checksum) = data.split_at(data.len() - 20);
    let digest = Sha1::digest(body);
    anyhow::ensure!(digest.as_slice() == checksum, "invalid index checksum");

    // Parse 12-byte header: "DIRC" + version + num_entries, all big-endian
    let mut cursor = Cursor::new(body);
    let mut signature = [0u8; 4];
    cursor.read_exact(&mut signature)?;
    anyhow::ensure!(&signature == b"DIRC", "invalid index signature");

    let version = cursor.read_u32::<BigEndian>()?;
    anyhow::ensure!(version == 2, "unknown index version {}", version);

    let num_entries = cursor.read_u32::<BigEndian>()? as usize;

    // Parse entries
    let mut entries = Vec::with_capacity(num_entries);
    while (cursor.position() as usize) + 62 < body.len() {
        let entry_start = cursor.position() as usize;

        let ctime_s = cursor.read_u32::<BigEndian>()?;
        let ctime_n = cursor.read_u32::<BigEndian>()?;
        let mtime_s = cursor.read_u32::<BigEndian>()?;
        let mtime_n = cursor.read_u32::<BigEndian>()?;
        let dev = cursor.read_u32::<BigEndian>()?;
        let ino = cursor.read_u32::<BigEndian>()?;
        let mode = cursor.read_u32::<BigEndian>()?;
        let uid = cursor.read_u32::<BigEndian>()?;
        let gid = cursor.read_u32::<BigEndian>()?;
        let size = cursor.read_u32::<BigEndian>()?;

        let mut sha1 = [0u8; 20];
        cursor.read_exact(&mut sha1)?;

        let flags = cursor.read_u16::<BigEndian>()?;

        // Read null-terminated path
        let mut path_bytes = vec![];
        loop {
            let b = cursor.read_u8()?;
            if b == 0 {
                break;
            }
            path_bytes.push(b);
        }
        let path = String::from_utf8(path_bytes)?;

        // Align to 8-byte boundary
        let entry_len = ((62 + path.len() + 8) / 8) * 8;
        let next = entry_start + entry_len;
        cursor.set_position(next as u64);

        entries.push(IndexEntry {
            ctime_s,
            ctime_n,
            mtime_s,
            mtime_n,
            dev,
            ino,
            mode,
            uid,
            gid,
            size,
            sha1,
            flags,
            path,
        });
    }

    anyhow::ensure!(entries.len() == num_entries, "entry count mismatch");
    Ok(entries)
}

/// Print list of files in index (including mode, SHA-1, and stage number
/// if "details" is True).
pub fn ls_files(details: bool) -> anyhow::Result<()> {
    let entries = read_index()?;
    for entry in entries {
        if details {
            let stage = (entry.flags >> 12) & 3;
            println!(
                "{:#o} {} {:?}\t{}",
                entry.mode,
                hex::encode(entry.sha1),
                stage,
                entry.path
            );
        }
    }
    Ok(())
}

/// Get status of working copy, return tuple of (changed_paths, new_paths,
///  deleted_paths).
pub fn get_status() -> anyhow::Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let mut paths: HashSet<String> = HashSet::new();
    let walker = WalkDir::new(".").into_iter();
    for entry in walker.filter_entry(|e| e.file_name().to_str().unwrap_or_default() != ".git") {
        let entry = entry?;
        let path = entry.into_path();
        if path.is_file() {
            let mut new_path = PathBuf::from(".").join(&path).to_string_lossy().to_string();
            new_path = new_path.replace("\\", "/");
            let repl = if new_path.starts_with("./") {
                new_path[2..].to_string()
            } else {
                new_path
            };
            paths.insert(repl);
        }
    }
    let entries = read_index()?;
    let entries_by_path: HashMap<String, IndexEntry> =
        entries.into_iter().map(|e| (e.path.clone(), e)).collect();
    let entries_paths: HashSet<String> = entries_by_path.keys().map(|s| s.to_owned()).collect();
    let changed: HashSet<String> = entries_paths
        .intersection(&paths)
        .into_iter()
        .filter(|p| {
            let mut data = read_file(&PathBuf::from(p)).unwrap();
            let obj_hash = hash_object(&mut data, ObjectTipe::Blob, false).unwrap();
            let entry = entries_by_path.get(*p).unwrap();
            obj_hash != hex::encode(entry.sha1)
        })
        .map(|e| e.to_string())
        .collect();
    let new: HashSet<String> = paths
        .difference(&entries_paths)
        .into_iter()
        .map(|e| e.to_string())
        .collect();
    let deleted: HashSet<String> = entries_paths
        .difference(&paths)
        .into_iter()
        .map(|e| e.to_string())
        .collect();
    let mut changed_vec: Vec<String> = changed.into_iter().collect();
    changed_vec.sort();
    let mut new_vec: Vec<String> = new.into_iter().collect();
    new_vec.sort();
    let mut deleted_vec: Vec<String> = deleted.into_iter().collect();
    deleted_vec.sort();
    Ok((changed_vec, new_vec, deleted_vec))
}

/// Show status of working copy.
pub fn status() -> anyhow::Result<()> {
    let status = get_status()?;
    // changed
    if !status.0.is_empty() {
        println!("changed files:");
        for path in status.0 {
            println!("    {}", path);
        }
    }
    if !status.1.is_empty() {
        println!("new files:");
        for path in status.1 {
            println!("    {}", path);
        }
    }
    if !status.2.is_empty() {
        println!("deleted files:");
        for path in status.2 {
            println!("    {}", path);
        }
    }
    Ok(())
}

/// Show diff of files changed (between index and working copy).
pub fn diff() -> anyhow::Result<()> {
    let (changed, _, _) = get_status()?;
    let entries = read_index()?;
    let entries_by_path: HashMap<String, IndexEntry> =
        entries.into_iter().map(|e| (e.path.clone(), e)).collect();
    let mut i = 0;
    while i < changed.len() {
        let sha1 = hex::encode(entries_by_path.get(&changed[i]).unwrap().sha1);
        let (obj_type, data) = read_object(&sha1)?;
        match obj_type {
            ObjectTipe::Blob => {}
            _ => return Err(anyhow!("Expected object type to be 'blob'")),
        }
        let mut full_str = String::new();
        data.as_slice().read_to_string(&mut full_str)?;
        let index_lines: Vec<&str> = full_str.split("\n").collect();
        let actual_data = read_file(&PathBuf::from(&changed[i]))?;
        let mut actual_str = String::new();
        actual_data.as_slice().read_to_string(&mut actual_str)?;
        let working_lines: Vec<&str> = actual_str.split("\n").collect();
        let diff_lines = difflib::unified_diff(
            &index_lines,
            &working_lines,
            &format!("{} (index)", changed[i]),
            &format!("{} (working copy)", changed[i]),
            "",
            "",
            3,
        );
        for line in diff_lines {
            println!("{}", line);
        }
        if i < changed.len() - 1 {
            println!("{}", "*".repeat(70));
        }
        i += 1;
    }

    Ok(())
}

/// Write list of IndexEntry objects to git index file.
pub fn write_index(entries: &[IndexEntry]) -> anyhow::Result<()> {
    let mut data: Vec<u8> = Vec::new();

    // Header: "DIRC" + version 2 + entry count
    data.extend_from_slice(b"DIRC");
    data.write_u32::<BigEndian>(2)?;
    data.write_u32::<BigEndian>(entries.len() as u32)?;

    // Pack each entry
    for entry in entries {
        data.write_u32::<BigEndian>(entry.ctime_s)?;
        data.write_u32::<BigEndian>(entry.ctime_n)?;
        data.write_u32::<BigEndian>(entry.mtime_s)?;
        data.write_u32::<BigEndian>(entry.mtime_n)?;
        data.write_u32::<BigEndian>(entry.dev)?;
        data.write_u32::<BigEndian>(entry.ino)?;
        data.write_u32::<BigEndian>(entry.mode)?;
        data.write_u32::<BigEndian>(entry.uid)?;
        data.write_u32::<BigEndian>(entry.gid)?;
        data.write_u32::<BigEndian>(entry.size)?;
        data.extend_from_slice(&entry.sha1);
        data.write_u16::<BigEndian>(entry.flags)?;

        // Null-terminated path, padded to 8-byte boundary
        let path = entry.path.as_bytes();
        data.extend_from_slice(path);
        let length = ((62 + path.len() + 8) / 8) * 8;
        let padding = length - 62 - path.len();
        data.extend(std::iter::repeat(0u8).take(padding));
    }

    // Append SHA1 digest of everything written so far
    let digest = Sha1::digest(&data);
    data.extend_from_slice(&digest);

    write_file(&std::path::PathBuf::from(".git/index"), data)?;
    Ok(())
}
