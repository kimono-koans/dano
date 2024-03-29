//       ___           ___           ___           ___
//      /\  \         /\  \         /\__\         /\  \
//     /::\  \       /::\  \       /::|  |       /::\  \
//    /:/\:\  \     /:/\:\  \     /:|:|  |      /:/\:\  \
//   /:/  \:\__\   /::\~\:\  \   /:/|:|  |__   /:/  \:\  \
//  /:/__/ \:|__| /:/\:\ \:\__\ /:/ |:| /\__\ /:/__/ \:\__\
//  \:\  \ /:/  / \/__\:\/:/  / \/__|:|/:/  / \:\  \ /:/  /
//   \:\  /:/  /       \::/  /      |:/:/  /   \:\  /:/  /
//    \:\/:/  /        /:/  /       |::/  /     \:\/:/  /
//     \::/__/        /:/  /        /:/  /       \::/  /
//      ~~            \/__/         \/__/         \/__/
//
// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    error::Error,
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use rayon::{prelude::*, ThreadPool};
use serde_json::Value;

use crate::lookup::FileInfo;
use crate::output::WriteType;
use crate::versions::LegacyVersion;
use crate::{Config, ExecMode, DANO_FILE_INFO_VERSION, DANO_XATTR_KEY_NAME};

pub type DanoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// u128::MAX to LowerHex to String len is 32usize
// this is one of those things one can't make a const function
const HASH_VALUE_MIN_WIDTH: usize = 32;
const TMP_SUFFIX: &str = ".tmp";

pub fn prepare_thread_pool(config: &Config) -> DanoResult<ThreadPool> {
    let num_threads = if let Some(num_threads) = config.opt_num_threads {
        num_threads
    } else {
        num_cpus::get()
    };

    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Could not initialize rayon thread pool");

    Ok(thread_pool)
}

#[derive(Debug, Clone, Default)]
pub struct DanoError {
    pub details: String,
}

impl DanoError {
    pub fn new(msg: &str) -> Self {
        DanoError {
            details: msg.to_owned(),
        }
    }
    #[allow(dead_code)]
    pub fn with_context(msg: &str, err: Box<dyn Error + Send + Sync>) -> Self {
        let msg_plus_context = format!("{} : {:?}", msg, err);
        DanoError {
            details: msg_plus_context,
        }
    }
}

impl fmt::Display for DanoError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for DanoError {
    fn description(&self) -> &str {
        &self.details
    }
}

pub fn make_tmp_file(path: &Path) -> PathBuf {
    let path_string = path.to_string_lossy().to_string();
    let res = path_string + TMP_SUFFIX;
    PathBuf::from(res)
}

pub fn write_file(file_info: &FileInfo, output_file: &mut File) -> DanoResult<()> {
    let serialized = serialize(file_info)?;
    write_out_file(&serialized, output_file)
}

pub fn write_non_file(file_info: &FileInfo) -> DanoResult<()> {
    // write empty path for path, because we a re writing to an actual path
    // that may change if the file name is changed
    let rewrite = FileInfo {
        version: file_info.version,
        path: PathBuf::new(),
        metadata: file_info.metadata.to_owned(),
    };

    let serialized = serialize(&rewrite)?;
    write_out_xattr(&serialized, file_info)
}

pub fn remove_dano_xattr(path: &Path) -> DanoResult<()> {
    xattr::remove(path, DANO_XATTR_KEY_NAME).map_err(|err| err.into())
}

fn write_out_xattr(out_string: &str, file_info: &FileInfo) -> DanoResult<()> {
    let _ = xattr::remove(&file_info.path, DANO_XATTR_KEY_NAME);
    xattr::set(&file_info.path, DANO_XATTR_KEY_NAME, out_string.as_bytes())
        .map_err(|err| err.into())
}

pub fn print_err_buf(err_buf: &str) -> DanoResult<()> {
    // mutex keeps threads from writing over each other
    let err = std::io::stderr();
    let mut err_locked = err.lock();
    err_locked.write_all(err_buf.as_bytes())?;
    err_locked.flush().map_err(|err| err.into())
}

pub fn print_out_buf(output_buf: &str) -> DanoResult<()> {
    // mutex keeps threads from writing over each other
    let out = std::io::stdout();
    let mut out_locked = out.lock();
    out_locked.write_all(output_buf.as_bytes())?;
    out_locked.flush().map_err(|err| err.into())
}

pub fn print_file_info(config: &Config, file_info: &FileInfo) -> DanoResult<()> {
    let buffer = match &file_info.metadata {
        Some(metadata) => {
            let hash_value_as_hex = format!("{}", metadata.hash_value.value);

            format!(
                "{}={:<width$} : {:?}\n",
                metadata.hash_algo,
                hash_value_as_hex,
                file_info.path,
                width = HASH_VALUE_MIN_WIDTH
            )
        }
        None => {
            let msg = format!("Could not find file metadata for: {:?}\n", file_info.path);
            return Err(DanoError::new(&msg).into());
        }
    };

    // why?  b/c the writing of the file is the thing in write and dump mode and
    // this fn used then is just to print info about the hash.  we may wish to send to dev null
    match config.exec_mode {
        ExecMode::Print | ExecMode::Duplicates | ExecMode::Test(_) => print_out_buf(&buffer),
        ExecMode::Write(_) | ExecMode::Dump | ExecMode::Clean => print_err_buf(&buffer),
    }
}

pub fn get_hash_file(config: &Config) -> DanoResult<File> {
    if let Ok(input_file) = OpenOptions::new().read(true).open(&config.hash_file) {
        Ok(input_file)
    } else {
        Err(DanoError::new("dano could not open a file to write to").into())
    }
}

fn print_file_header(config: &Config, output_file: &mut File) -> DanoResult<()> {
    write_out_file(
        format!("// DANO, Invoked from: {:?}\n", config.pwd).as_str(),
        output_file,
    )
}

pub fn get_output_file(config: &Config, write_type: WriteType) -> DanoResult<File> {
    let output_file = match write_type {
        WriteType::Append => config.output_file.clone(),
        WriteType::Overwrite => make_tmp_file(&config.output_file),
    };

    let is_first_run = !output_file.exists();

    let mut output_file = OpenOptions::new()
        // should overwrite the file always
        // FYI append() is for adding to the file
        .create(true)
        .append(true)
        // create_new() will only create if DNE
        // create on a file that exists just opens
        .open(&output_file)?;

    if is_first_run {
        print_file_header(config, &mut output_file)?
    }

    Ok(output_file)
}

fn write_out_file(out_string: &str, open_file: &mut File) -> DanoResult<()> {
    open_file
        .write_all(out_string.as_bytes())
        .map_err(|err| err.into())
}

pub fn serialize(file_info: &FileInfo) -> DanoResult<String> {
    match serde_json::to_string(&file_info) {
        Ok(s) => Ok(s + "\n"),
        Err(err) => Err(err.into()),
    }
}

pub fn deserialize(line: &str) -> DanoResult<FileInfo> {
    let root: Value = serde_json::from_str(line)?;
    let value = root
        .get("version")
        .ok_or_else(|| DanoError::new("Could not get version value from JSON."))?
        .to_owned();

    let version: usize = serde_json::from_value(value)?;

    if version == DANO_FILE_INFO_VERSION {
        serde_json::from_str(line).map_err(|err| err.into())
    } else {
        LegacyVersion::into_latest(line)
    }
}

pub fn read_file_info_from_file(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let mut input_file = get_hash_file(config)?;
    let mut buffer = String::new();
    input_file.read_to_string(&mut buffer)?;
    Ok(buffer.par_lines().flat_map(deserialize).collect())
}

pub fn read_stdin() -> DanoResult<Vec<PathBuf>> {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buffer = Vec::new();
    stdin.read_to_end(&mut buffer)?;

    let buffer_string = std::str::from_utf8(&buffer)?;

    let broken_string = if buffer_string.contains(['\n', '\0']) {
        // always split on newline or null char, if available
        buffer_string
            .split(&['\n', '\0'])
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned())
            .map(PathBuf::from)
            .collect()
    } else if buffer_string.contains('\"') {
        buffer_string
            .split('\"')
            // unquoted paths should have excess whitespace trimmed
            .map(|s| s.trim())
            // remove any empty strings
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect()
    } else {
        buffer_string
            .split_ascii_whitespace()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect()
    };

    Ok(broken_string)
}
