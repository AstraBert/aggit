extern crate difflib;

use anyhow::anyhow;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use flate2::{
    Compression,
    read::{ZlibDecoder, ZlibEncoder},
};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use sha1_checked::{Digest, Sha1};
use std::{
    collections::HashMap,
    fs::Permissions,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
use std::{collections::HashSet, time::SystemTime};
use std::{fmt, fs, path::PathBuf, str::FromStr};
use std::{
    io::{Cursor, Read},
    time::UNIX_EPOCH,
};
use time::UtcOffset;

const HASH_SEPARATOR: &str = "\x00";

/// Data for one entry in the git index (.aggit/refs/{branch}/index)
#[derive(Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug)]
pub struct CommitAuthor {
    name: String,
    email: String,
}

impl fmt::Display for CommitAuthor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} <{}>", self.name, self.email)
    }
}

/// Object type enum. There are other types too, but we don't need them.
/// See "enum object_type" in git's source (git/cache.h).
#[derive(PartialEq, Eq, Debug)]
pub enum ObjectType {
    Commit,
    Tree,
    Blob,
}

impl fmt::Display for ObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Blob => "blob",
            Self::Commit => "commit",
            Self::Tree => "tree",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for ObjectType {
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

/// Create directory for repo (if it does not already exist) and initialize .aggit directory
pub fn init(repository: PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(&repository)?;
    fs::create_dir_all(repository.join(".aggit"))?;
    let git_folders: [&str; 4] = ["objects", "refs", "refs/heads", "refs/index"];
    for g in git_folders {
        fs::create_dir_all(repository.join(".aggit").join(g))?;
    }
    write_file(
        &repository.join(".aggit").join("HEAD"),
        "ref: refs/heads/main".into(),
    )?;
    println!("Repository {:?} successfully initialized", &repository);

    Ok(())
}

fn compress(data: &mut [u8]) -> anyhow::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(&data[..], Compression::default());
    let mut compressed = Vec::new();
    encoder.read_to_end(&mut compressed)?;
    Ok(compressed)
}

fn decompress(data: &mut [u8]) -> anyhow::Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(&data[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

pub fn hash_object(
    data: &mut Vec<u8>,
    object_type: ObjectType,
    write: bool,
) -> anyhow::Result<String> {
    let mut full_data = format!("{} {:?}", &object_type, &data.len()).into_bytes();
    full_data.append(&mut HASH_SEPARATOR.into());
    full_data.append(data);
    let sha1 = hex::encode(Sha1::digest(&full_data));
    if write {
        let path = PathBuf::from(".aggit")
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
    let obj_dir = PathBuf::from(".aggit")
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
            .starts_with(rest)
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
pub fn read_object(sha1_prefix: &str) -> anyhow::Result<(ObjectType, Vec<u8>)> {
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
        let object_type = ObjectType::from_str(spliced[0])?;
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
            if obj_data.0 != ObjectType::from_str(mode)? {
                return Err(anyhow!(
                    "Expected object type: {}, got {}",
                    obj_data.0,
                    mode
                ));
            }
            match obj_data.0 {
                ObjectType::Tree => {
                    let entries = read_tree(None, Some(obj_data.1))?;
                    for (mode, path, sha1) in entries {
                        println!("{:06o} blob {}\t{}", mode, sha1, path);
                    }
                }
                _ => {
                    let mut str_data = String::new();
                    obj_data.1.as_slice().read_to_string(&mut str_data)?;
                    println!("{}", str_data);
                }
            }
        }
        CatFileMode::Size => {
            println!("{:?}", obj_data.1.len());
        }
        CatFileMode::Type => {
            println!("{}", obj_data.0);
        }
        CatFileMode::Pretty => match obj_data.0 {
            ObjectType::Blob => {
                let mut str_data = String::new();
                obj_data.1.as_slice().read_to_string(&mut str_data)?;
                println!("{}", str_data);
            }
            ObjectType::Commit => {
                let mut str_data = String::new();
                obj_data.1.as_slice().read_to_string(&mut str_data)?;
                println!("{}", str_data);
            }
            ObjectType::Tree => {
                let tree_objs = read_tree(None, Some(obj_data.1))?;
                for (mode, path, sha1) in tree_objs {
                    let type_str = if (mode & 0o170000) == 0o040000 {
                        "tree"
                    } else {
                        "blob"
                    };
                    println!("{:o} {} {}\t{}", mode, type_str, sha1, path);
                }
            }
        },
    }
    Ok(())
}

/// Read git index file and return list of IndexEntry objects.
pub fn read_index() -> anyhow::Result<Vec<IndexEntry>> {
    let current_branch = get_current_branch()?;
    let index_path = index_path_for_branch(&current_branch);
    let data = match read_file(&index_path) {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };
    if data.is_empty() {
        return Ok(vec![]);
    }

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
    if details {
        println!("MODE\tSHA1\tSTAGE\tPATH");
    } else {
        println!("SHA1\tPATH")
    }
    for entry in entries {
        if details {
            let stage = (entry.flags >> 12) & 3;
            println!(
                "{:o}\t{}\t{:?}\t{}",
                entry.mode,
                hex::encode(entry.sha1),
                stage,
                entry.path
            );
        } else {
            println!("{}\t{}", hex::encode(entry.sha1), entry.path);
        }
    }
    Ok(())
}

/// Get status of working copy, return tuple of (changed_paths, new_paths,
///  deleted_paths).
pub fn get_status() -> anyhow::Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let mut paths: HashSet<String> = HashSet::new();
    let walker = WalkBuilder::new(".")
        .add_custom_ignore_filename(".aggitignore") // checks .aggitignore first
        .hidden(false) // include dotfiles if needed
        .build();
    for entry in walker {
        let entry = entry?;
        let path = entry.into_path();
        if path.is_file() {
            let new_path = path.to_string_lossy().replace("\\", "/");
            let repl = new_path.trim_start_matches("./").to_string();
            paths.insert(repl);
        }
    }
    let entries = read_index()?;
    let entries_by_path: HashMap<String, IndexEntry> = entries
        .into_iter()
        .filter(|e| PathBuf::from(&e.path).is_file())
        .map(|e| (e.path.clone(), e))
        .collect();
    let entries_paths: HashSet<String> = entries_by_path.keys().map(|s| s.to_owned()).collect();
    let changed: HashSet<String> = entries_paths
        .intersection(&paths)
        .filter(|p| {
            let mut data = read_file(&PathBuf::from(p)).unwrap();
            let obj_hash = hash_object(&mut data, ObjectType::Blob, false).unwrap();
            let entry = entries_by_path.get(*p).unwrap();
            obj_hash != hex::encode(entry.sha1)
        })
        .map(|e| e.to_string())
        .collect();
    let new: HashSet<String> = paths
        .difference(&entries_paths)
        .map(|e| e.to_string())
        .collect();
    let deleted: HashSet<String> = entries_paths
        .difference(&paths)
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
            ObjectType::Blob => {}
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
        data.extend(std::iter::repeat_n(0u8, padding));
    }

    // Append SHA1 digest of everything written so far
    let digest = Sha1::digest(&data);
    data.extend_from_slice(&digest);
    let current_branch = get_current_branch()?;
    let index_path = index_path_for_branch(&current_branch);

    write_file(&index_path, data)?;
    Ok(())
}

/// Add all file paths to git index.
pub fn add(paths: Vec<String>) -> anyhow::Result<()> {
    let replaced: Vec<String> = paths.iter().map(|p| p.replace("\\", "/")).collect();
    let all_entries = read_index()?;
    let mut entries: Vec<IndexEntry> = all_entries
        .iter()
        .filter(|e| !replaced.contains(&e.path))
        .cloned()
        .collect();
    for path in replaced {
        let mut data = read_file(&PathBuf::from(&path))?;
        let sha1 = hash_object(&mut data, ObjectType::Blob, true)?;
        let st = fs::metadata(&path)?;
        let flags = path.len() as u16;
        if flags >= (1 << 12) {
            return Err(anyhow!("Invalid flags"));
        }
        let sha1_bytes: [u8; 20] = hex::decode(&sha1)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("SHA1 hash must be 20 bytes"))?;
        let entry = IndexEntry {
            ctime_s: st.ctime() as u32,
            ctime_n: st.ctime_nsec() as u32,
            mtime_n: st.mtime() as u32,
            mtime_s: st.mtime_nsec() as u32,
            dev: st.dev() as u32,
            ino: st.ino() as u32,
            size: st.size() as u32,
            gid: st.gid(),
            uid: st.uid(),
            mode: st.mode(),
            flags,
            sha1: sha1_bytes,
            path,
        };
        entries.push(entry);
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    write_index(entries.as_slice())?;
    Ok(())
}

pub fn read_tree(
    sha1: Option<String>,
    data: Option<Vec<u8>>,
) -> anyhow::Result<Vec<(u32, String, String)>> {
    read_tree_recursive(sha1, data, "")
}

fn read_tree_recursive(
    sha1: Option<String>,
    data: Option<Vec<u8>>,
    prefix: &str,
) -> anyhow::Result<Vec<(u32, String, String)>> {
    let actual_data;
    if let Some(sh) = sha1 {
        let (obj_type, data) = read_object(&sh)?;
        actual_data = data;
        match obj_type {
            ObjectType::Tree => {}
            _ => return Err(anyhow!("Expected object type to be 'tree'")),
        }
    } else if let Some(d) = data {
        actual_data = d;
    } else {
        return Err(anyhow!(
            "At least one between SHA1 and data should be non-null"
        ));
    }

    let mut i = 0;
    let mut entries = Vec::new();

    while i < actual_data.len() {
        // Find the null separator
        let Some(null_pos) = actual_data[i..].iter().position(|&b| b == 0) else {
            break;
        };
        let null_pos = i + null_pos;

        let header = std::str::from_utf8(&actual_data[i..null_pos])?;
        let (mode_str, name) = header
            .split_once(' ')
            .ok_or(anyhow!("Invalid tree entry header"))?;

        let mode = u32::from_str_radix(mode_str, 8)?; // mode is octal
        let full_path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}{}", prefix, name)
        };

        let digest = &actual_data[null_pos + 1..null_pos + 21];
        let sha1_hex = hex::encode(digest);

        if mode == 0o40000 {
            entries.push((mode, full_path.clone(), sha1_hex.clone()));
            // It's a subtree: recurse with updated prefix
            let mut sub_entries =
                read_tree_recursive(Some(sha1_hex), None, &format!("{}/", full_path))?;
            entries.append(&mut sub_entries);
        } else {
            entries.push((mode, full_path, sha1_hex));
        }

        i = null_pos + 1 + 20;
    }

    Ok(entries)
}

pub fn write_tree() -> anyhow::Result<String> {
    let entries = read_index()?;
    write_tree_recursive(&entries, "")
}

fn write_tree_recursive(entries: &[IndexEntry], prefix: &str) -> anyhow::Result<String> {
    let mut tree_entries: Vec<u8> = Vec::new();

    // Collect direct children (files) and subdirectory groups
    let mut subdirs: std::collections::BTreeMap<String, Vec<&IndexEntry>> = Default::default();
    let mut files: Vec<&IndexEntry> = Vec::new();

    for entry in entries {
        let relative = entry.path.strip_prefix(prefix).unwrap_or(&entry.path);
        if let Some((dir, _)) = relative.split_once('/') {
            subdirs.entry(dir.to_string()).or_default().push(entry);
        } else {
            files.push(entry);
        }
    }

    // Write file blobs directly
    for entry in files {
        let mut te = format!(
            "{:o} {}",
            entry.mode,
            entry.path.strip_prefix(prefix).unwrap_or(&entry.path)
        )
        .into_bytes();
        te.push(0); // null separator
        te.extend_from_slice(&entry.sha1);
        tree_entries.extend(te);
    }

    // Recursively write subtrees
    for (dir, sub_entries) in &subdirs {
        let new_prefix = if prefix.is_empty() {
            format!("{}/", dir)
        } else {
            format!("{}{}/", prefix, dir)
        };
        let entr: Vec<IndexEntry> = sub_entries.iter().map(|i| (*i).clone()).collect();
        let sub_sha1_hex = write_tree_recursive(entr.as_slice(), &new_prefix)?;
        let sub_sha1 = hex::decode(sub_sha1_hex)?;
        let mut te = format!("40000 {}", dir).into_bytes(); // dir mode, no leading 0
        te.push(0);
        te.extend_from_slice(&sub_sha1);
        tree_entries.extend(te);
    }

    hash_object(&mut tree_entries, ObjectType::Tree, true)
}

/// Get current commit hash (SHA-1 string) of local main branch.
pub fn get_local_current_hash() -> anyhow::Result<(Option<String>, String)> {
    let current_branch = get_current_branch()?;
    let main_path = PathBuf::from(".aggit")
        .join("refs")
        .join("heads")
        .join(&current_branch);
    let result = read_file(&main_path);
    match result {
        Ok(d) => {
            let mut content = String::new();
            d.as_slice().read_to_string(&mut content)?;
            content = content.trim().to_string();
            Ok((Some(content), current_branch))
        }
        Err(_) => Ok((None, current_branch)),
    }
}

pub fn config_author(name: String, email: String) -> anyhow::Result<()> {
    let config_file = dirs::config_dir()
        .expect("Should have a global confing directory")
        .join(".aggit")
        .join("author.toml");
    fs::create_dir_all(config_file.parent().unwrap())?;
    let author = CommitAuthor { name, email };
    let toml_str = toml::to_string(&author)?;
    fs::write(&config_file, toml_str)?;
    println!(
        "Successfully set global commit author to {} ({}). Everything is saved (and can be modified) in {:?}",
        author.name, author.email, &config_file
    );
    Ok(())
}

pub fn read_author() -> anyhow::Result<CommitAuthor> {
    let config_file = dirs::config_dir()
        .expect("Should have a global config directory")
        .join(".aggit")
        .join("author.toml");
    let content = fs::read_to_string(config_file).map_err(|_| {
        anyhow!(
            "The commit author should be globally configured. Use the `author` command to do so."
        )
    })?;
    let author: CommitAuthor = toml::from_str(&content)?;
    Ok(author)
}

/// Commit the current state of the index to master with given message.
/// Return hash of commit object.
pub fn commit(message: &str) -> anyhow::Result<String> {
    let tree = write_tree()?;
    let (parent, current_branch) = get_local_current_hash()?;
    let author = read_author()?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let offset = UtcOffset::current_local_offset().unwrap();
    let total_seconds = offset.whole_seconds();
    let sign = if total_seconds >= 0 { '+' } else { '-' };
    let hours = total_seconds.abs() / 3600;
    let minutes = (total_seconds.abs() / 60) % 60;

    let author_time = format!("{} {}{:02}{:02}", timestamp, sign, hours, minutes);
    let mut lines = vec!["tree ".to_string() + &tree];
    if let Some(par) = parent {
        lines.push("parent ".to_string() + &par);
    }
    lines.push(format!("author {} {}", author, author_time));
    lines.push(format!("committer {} {}", author, author_time));
    lines.push(String::new());
    lines.push(message.to_string());
    lines.push(String::new());
    let mut data = lines.join("\n").into_bytes();
    let sha1 = hash_object(&mut data, ObjectType::Commit, true)?;
    let main_path = PathBuf::from(".aggit")
        .join("refs")
        .join("heads")
        .join(&current_branch);
    write_file(&main_path, (format!("{}\n", sha1)).into_bytes())?;
    println!("Committed to {}: {}", &current_branch, &sha1[..7]);
    Ok(sha1)
}

pub fn get_current_branch() -> anyhow::Result<String> {
    let head_path = PathBuf::from(".aggit").join("HEAD");
    let current_ref = fs::read_to_string(&head_path)?;
    let branch = current_ref
        .strip_prefix("ref: refs/heads/")
        .unwrap()
        .trim_end_matches("\n");
    Ok(branch.to_string())
}

pub fn list_branches() -> anyhow::Result<()> {
    let current_branch = get_current_branch()?;
    let entries = fs::read_dir(PathBuf::from(".aggit").join("refs").join("heads"))?;
    let mut other_branches = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_str().unwrap() != current_branch {
            other_branches.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    other_branches.insert(0, format!("\x1b[1;32m* {}\x1b[1;37m", current_branch));
    println!("{}", other_branches.join("\n"));

    Ok(())
}

pub fn index_path_for_branch(branch: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(".aggit/refs/index/{}", branch))
}

pub fn head_path_for_branch(branch: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(".aggit/refs/heads/{}", branch))
}

/// Switch to a different branch. If `create` is true and the target branch does
/// not exist, create it from the current branch and commit.
pub fn switch_branch(name: &str, create: bool) -> anyhow::Result<()> {
    let head_path = PathBuf::from(".aggit").join("HEAD");
    let branch = get_current_branch()?;
    if name == branch {
        println!("Already on branch {}", branch);
        return Ok(());
    }
    // check if branch is dirty (uncommitted changes)
    let (changed, new, deleted) = get_status()?;
    if !changed.is_empty() || !new.is_empty() || !deleted.is_empty() {
        return Err(anyhow!(
            "Current working tree has uncommitted changes. Please commit them and re-try"
        ));
    }
    let branch_path = PathBuf::from(".aggit")
        .join("refs")
        .join("heads")
        .join(name);
    if branch_path.exists() && create {
        println!("{} already exists, `--create/-c` has no effect", name);
        // set branch as current
        fs::write(&head_path, format!("ref: refs/heads/{}", name))?;
        println!("Switched to branch {}", name);
    } else if !branch_path.exists() && !create {
        return Err(anyhow!(
            "{} does not exist, please pass the `--create/-c` flag explicitly to create it",
            name
        ));
    } else if branch_path.exists() && !create {
        // restore working tree
        restore_branch_working_tree(name)?;
        // set branch as current
        fs::write(&head_path, format!("ref: refs/heads/{}", name))?;
        println!("Switched to branch {}", name);
    } else {
        // create directory
        fs::create_dir_all(branch_path.parent().unwrap())?;
        // Copy current branch's index to the new branch
        let current_index_path = index_path_for_branch(&branch);
        let new_index_path = index_path_for_branch(name);
        if let Some(parent) = new_index_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if current_index_path.exists() {
            fs::copy(&current_index_path, &new_index_path)?;
        } else {
            fs::write(&new_index_path, "")?;
        }
        // get current hash
        let (current_hash, _) = get_local_current_hash()?;
        if let Some(hash) = current_hash {
            // write current hash
            fs::write(branch_path, format!("{}\n", hash))?;
            println!(
                "Created branch {} from commit {} on branch {}",
                name, hash, branch
            );
        } else {
            // write empty file (no current hash)
            fs::write(branch_path, "")?;
            println!("Created branch {} from branch {}", name, branch);
        }
        // set branch as current
        fs::write(&head_path, format!("ref: refs/heads/{}", name))?;
    }

    Ok(())
}

pub fn restore_branch_working_tree(branch_name: &str) -> anyhow::Result<()> {
    let branch_path = PathBuf::from(".aggit/refs/heads").join(branch_name);
    let commit_hash = fs::read_to_string(&branch_path)?.trim().to_string();

    if commit_hash.is_empty() {
        // Branch has no commits yet, nothing to restore
        return Ok(());
    }

    restore_working_tree(commit_hash)
}

pub fn restore_working_tree(commit_hash: String) -> anyhow::Result<()> {
    // Read the commit object to get the tree SHA1
    let (obj_type, commit_data) = read_object(&commit_hash)?;
    if !matches!(obj_type, ObjectType::Commit) {
        return Err(anyhow!("Expected commit object"));
    }

    // Parse "tree <sha1>" from the first line of the commit
    let commit_str = String::from_utf8(commit_data)?;
    let tree_sha1 = commit_str
        .lines()
        .find(|l| l.starts_with("tree "))
        .ok_or(anyhow!("No tree in commit"))?
        .strip_prefix("tree ")
        .unwrap()
        .to_string();

    // Read the tree to get file entries
    let tree_entries = read_tree(Some(tree_sha1), None)?;

    // Restore each file in the working tree
    for (mode, path, sha1) in &tree_entries {
        if *mode == 0o40000 {
            fs::create_dir_all(path)?;
        } else {
            let (_, file_data) = read_object(sha1)?;
            if let Some(par) = PathBuf::from(path).parent()
                && !par.exists()
            {
                fs::create_dir_all(par)?;
            }
            fs::write(path, &file_data)?;
            fs::set_permissions(path, Permissions::from_mode(*mode))?;
        }
    }

    // Rebuild the index from the tree entries to match
    let index_entries: Vec<IndexEntry> = tree_entries
        .iter()
        .map(|(mode, path, sha1)| {
            let flags = path.len() as u16;
            if flags >= (1 << 12) {
                return Err(anyhow!("Invalid flags"));
            }
            let meta = fs::metadata(path)?; // stat the just-written file
            Ok(IndexEntry {
                mode: *mode,
                path: path.clone(),
                sha1: hex::decode(sha1)?.try_into().unwrap(),
                ctime_s: meta.ctime() as u32,
                ctime_n: meta.ctime_nsec() as u32,
                mtime_s: meta.mtime() as u32,
                mtime_n: meta.mtime_nsec() as u32,
                size: meta.size() as u32,
                ino: meta.ino() as u32,
                dev: meta.dev() as u32,
                uid: meta.uid(),
                gid: meta.gid(),
                flags,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    write_index(&index_entries)?;
    Ok(())
}

/// Collect all reachable objects between the current local head (included) and the remote head (excluded)
pub fn collect_reachable_objects(
    head_sha1: &str,
    remote_head: Option<&str>,
) -> anyhow::Result<HashSet<String>> {
    let mut visited = HashSet::new();
    let mut queue = vec![head_sha1.to_string()];

    while let Some(sha1) = queue.pop() {
        // Stop if we've reached the commit the remote already has
        if remote_head == Some(sha1.as_str()) {
            continue;
        }
        // Already visited, skip
        if !visited.insert(sha1.clone()) {
            continue;
        }
        let (obj_type, data) = read_object(&sha1)?;
        match obj_type {
            ObjectType::Commit => {
                let text = String::from_utf8(data)?;
                for line in text.lines() {
                    if let Some(s) = line.strip_prefix("tree ") {
                        queue.push(s.to_string());
                    }
                    if let Some(s) = line.strip_prefix("parent ") {
                        queue.push(s.to_string());
                    }
                }
            }
            ObjectType::Tree => {
                let entries = read_tree(None, Some(data))?;
                for (_, _, sha1) in entries {
                    queue.push(sha1);
                }
            }
            ObjectType::Blob => {}
        }
    }
    Ok(visited)
}

pub fn checkout_commit(commit_hash: String) -> anyhow::Result<()> {
    // Check that current working tree is not dirty
    let (changed, new, deleted) = get_status()?;
    if !changed.is_empty() || !new.is_empty() || !deleted.is_empty() {
        return Err(anyhow!(
            "Current working tree has uncommitted changes. Please commit them and re-try"
        ));
    }

    // Validate SHA1 exists and is a commit
    match read_object(&commit_hash) {
        Err(_) => {
            return Err(anyhow!("commit {commit_hash} not found"));
        }
        Ok((obj_type, _)) => {
            if obj_type != ObjectType::Commit {
                return Err(anyhow!("{commit_hash} is not a commit, it's a {obj_type}"));
            }
        }
    }

    let (head, _) = get_local_current_hash()?;
    if let Some(h) = head
        && h == commit_hash
    {
        println!("Already at {commit_hash}");
        return Ok(());
    }

    match get_current_branch() {
        Ok(branch) => {
            let branch_path = PathBuf::from(".aggit")
                .join("refs")
                .join("heads")
                .join(branch);
            fs::write(branch_path, format!("{commit_hash}\n"))?;
        }
        Err(_) => {
            // detached HEAD — no branch ref to update
        }
    }

    restore_working_tree(commit_hash)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_test_repo() {
        let _ = fs::remove_dir_all(".aggit");
        init(PathBuf::from(".")).unwrap();
    }

    fn cleanup_test_repo() {
        let _ = fs::remove_dir_all(".aggit");
    }

    #[test]
    #[serial_test::serial]
    fn test_init_creates_directories() {
        cleanup_test_repo();
        let repo = PathBuf::from(".aggit_test_repo");
        let _ = fs::remove_dir_all(&repo);
        init(repo.clone()).unwrap();
        assert!(repo.join(".aggit").exists());
        assert!(repo.join(".aggit/objects").exists());
        assert!(repo.join(".aggit/refs/heads").exists());
        assert!(repo.join(".aggit/refs/index").exists());
        let head = fs::read_to_string(repo.join(".aggit/HEAD")).unwrap();
        assert_eq!(head, "ref: refs/heads/main");
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    #[serial_test::serial]
    fn test_hash_object_without_write() {
        cleanup_test_repo();
        setup_test_repo();
        let mut data = b"hello world".to_vec();
        let hash = hash_object(&mut data, ObjectType::Blob, false).unwrap();
        assert_eq!(hash.len(), 40);
        let path = PathBuf::from(".aggit/objects")
            .join(&hash[..2])
            .join(&hash[2..]);
        assert!(!path.exists());
        cleanup_test_repo();
    }

    #[test]
    #[serial_test::serial]
    fn test_hash_object_with_write() {
        cleanup_test_repo();
        setup_test_repo();
        let mut data = b"hello world".to_vec();
        let hash = hash_object(&mut data, ObjectType::Blob, true).unwrap();
        let path = PathBuf::from(".aggit/objects")
            .join(&hash[..2])
            .join(&hash[2..]);
        assert!(path.exists());
        cleanup_test_repo();
    }

    #[test]
    #[serial_test::serial]
    fn test_find_object_success() {
        cleanup_test_repo();
        setup_test_repo();
        let mut data = b"find me".to_vec();
        let hash = hash_object(&mut data, ObjectType::Blob, true).unwrap();
        let found = find_object(&hash).unwrap();
        assert!(found.to_string_lossy().contains(&hash[..2]));
        cleanup_test_repo();
    }

    #[test]
    #[serial_test::serial]
    fn test_find_object_not_found() {
        cleanup_test_repo();
        setup_test_repo();
        let result = find_object("0000000000000000000000000000000000000000");
        assert!(result.is_err());
        cleanup_test_repo();
    }

    #[test]
    fn test_read_object_roundtrip() {
        let data = b"roundtrip data".to_vec();
        let hash = hash_object(&mut data.clone(), ObjectType::Blob, true).unwrap();
        let (obj_type, read_data) = read_object(&hash).unwrap();
        assert_eq!(obj_type, ObjectType::Blob);
        assert_eq!(read_data, b"roundtrip data");
    }

    #[test]
    #[serial_test::serial]
    fn test_get_current_branch() {
        cleanup_test_repo();
        setup_test_repo();
        let branch = get_current_branch().unwrap();
        assert_eq!(branch, "main");
        cleanup_test_repo();
    }

    #[test]
    fn test_index_path_for_branch() {
        let path = index_path_for_branch("main");
        assert_eq!(path, PathBuf::from(".aggit/refs/index/main"));
    }

    #[test]
    fn test_head_path_for_branch() {
        let path = head_path_for_branch("main");
        assert_eq!(path, PathBuf::from(".aggit/refs/heads/main"));
    }

    #[test]
    fn test_object_type_from_str() {
        assert_eq!(ObjectType::from_str("blob").unwrap(), ObjectType::Blob);
        assert_eq!(ObjectType::from_str("commit").unwrap(), ObjectType::Commit);
        assert_eq!(ObjectType::from_str("tree").unwrap(), ObjectType::Tree);
        assert!(ObjectType::from_str("unknown").is_err());
    }

    #[test]
    fn test_cat_file_mode_from_str() {
        assert!(CatFileMode::from_str("blob").is_ok());
        assert!(CatFileMode::from_str("size").is_ok());
        assert!(CatFileMode::from_str("type").is_ok());
        assert!(CatFileMode::from_str("pretty").is_ok());
        assert!(CatFileMode::from_str("invalid").is_err());
    }

    #[test]
    #[serial_test::serial]
    fn test_write_and_read_index() {
        cleanup_test_repo();
        setup_test_repo();
        let entries = vec![IndexEntry {
            ctime_s: 0,
            ctime_n: 0,
            mtime_s: 0,
            mtime_n: 0,
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            size: 5,
            sha1: [0u8; 20],
            flags: 4,
            path: "test.txt".to_string(),
        }];
        write_index(&entries).unwrap();
        let read = read_index().unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].path, "test.txt");
        cleanup_test_repo();
    }

    #[test]
    fn test_read_tree_with_data() {
        let mut tree_data = Vec::new();
        tree_data.extend_from_slice(b"100644 file.txt\x00");
        tree_data.extend_from_slice(&[0u8; 20]);
        let entries = read_tree(None, Some(tree_data)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "file.txt");
    }

    #[test]
    #[serial_test::serial]
    fn test_get_local_current_hash_no_commits() {
        cleanup_test_repo();
        setup_test_repo();
        let (hash, branch) = get_local_current_hash().unwrap();
        assert!(hash.is_none());
        assert_eq!(branch, "main");
        cleanup_test_repo();
    }

    #[test]
    #[serial_test::serial]
    fn test_list_branches_no_error() {
        cleanup_test_repo();
        setup_test_repo();
        list_branches().unwrap();
        cleanup_test_repo();
    }

    #[test]
    #[serial_test::serial]
    fn test_collect_reachable_objects_blob_only() {
        cleanup_test_repo();
        setup_test_repo();
        let mut data = b"blob content".to_vec();
        let hash = hash_object(&mut data, ObjectType::Blob, true).unwrap();
        let objects = collect_reachable_objects(&hash, None).unwrap();
        assert!(objects.contains(&hash));
        cleanup_test_repo();
    }
}
