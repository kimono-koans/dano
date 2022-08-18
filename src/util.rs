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
    path::PathBuf,
};

use crate::{
    lookup_file_info::FileInfo, DryRun, ExecMode, DANO_FILE_INFO_VERSION, DANO_XATTR_KEY_NAME,
};
use crate::{Config, DanoResult};

#[derive(Debug, Clone)]
pub struct DanoError {
    pub details: String,
}

impl DanoError {
    pub fn new(msg: &str) -> Self {
        DanoError {
            details: msg.to_owned(),
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

pub enum WriteType {
    Append,
    OverwriteAll,
}

pub fn write_all_new_paths(
    config: &Config,
    new_files: &[FileInfo],
    write_type: WriteType,
) -> DanoResult<()> {
    match &config.exec_mode {
        ExecMode::Write(dry_run) if dry_run == &DryRun::Enabled || config.opt_xattr => new_files
            .iter()
            .try_for_each(|file_info| write_non_file(config, file_info)),
        _ => {
            let mut output_file = match write_type {
                WriteType::Append => append_output_file(config)?,
                WriteType::OverwriteAll => overwrite_output_file(config)?,
            };

            // why not a closure?! long story!
            // it seems with a closure we can't capture this &mut output_file
            // as an env var, and therefore we can't open the file once in the iter
            for file_info in new_files {
                write_file(file_info, &mut output_file)?
            }

            Ok(())
        }
    }
}

fn write_file(file_info: &FileInfo, output_file: &mut File) -> DanoResult<()> {
    let serialized = serialize(file_info)?;
    write_out_file(&serialized, output_file)
}

fn write_non_file(config: &Config, file_info: &FileInfo) -> DanoResult<()> {
    match &config.exec_mode {
        ExecMode::Write(dry_run) if dry_run == &DryRun::Enabled => {
            let serialized = serialize(file_info)?;
            print_out_buf(&serialized)
        }
        ExecMode::Write(_) if config.opt_xattr => {
            // write empty path for path, because we have the actual path
            let rewrite = FileInfo {
                version: file_info.version,
                path: PathBuf::new(),
                metadata: file_info.metadata.to_owned(),
            };

            let serialized = serialize(&rewrite)?;
            write_out_xattr(&serialized, file_info)
        }
        _ => unreachable!(),
    }
}

fn write_out_xattr(out_string: &str, file_info: &FileInfo) -> DanoResult<()> {
    // this is a relatively large xattr(?), may need to change later
    xattr::set(&file_info.path, DANO_XATTR_KEY_NAME, out_string.as_bytes())
        .map_err(|err| err.into())
}

pub fn print_err_buf(err_buf: &str) -> DanoResult<()> {
    // mutex keeps threads from writing over each other
    let err = std::io::stderr();
    let mut err_locked = err.lock();
    err_locked.write_all(err_buf.as_bytes())?;
    err_locked.flush()?;

    Ok(())
}

pub fn print_out_buf(output_buf: &str) -> DanoResult<()> {
    // mutex keeps threads from writing over each other
    let out = std::io::stdout();
    let mut out_locked = out.lock();
    out_locked.write_all(output_buf.as_bytes())?;
    out_locked.flush()?;

    Ok(())
}

pub fn print_file_info(file_info: &FileInfo) -> DanoResult<()> {
    let err_buf = match &file_info.metadata {
        Some(metadata) => {
            format!(
                "{}={:x} : {:?}\n",
                metadata.hash_algo, metadata.hash_value, file_info.path
            )
        }
        None => {
            format!(
                "WARNING: Could not generate checksum for: {:?}\n",
                file_info.path
            )
        }
    };

    print_err_buf(&err_buf)
}

pub fn read_input_file(config: &Config) -> DanoResult<File> {
    if let Ok(input_file) = OpenOptions::new().read(true).open(&config.hash_file) {
        Ok(input_file)
    } else {
        Err(DanoError::new("dano could not open a file to write to").into())
    }
}

fn print_file_header(config: &Config, output_file: &mut File) -> DanoResult<()> {
    write_out_file(
        format!(
            "// DANO, FILE FORMAT VERSION:{}\n// Invoked from: {:?}\n",
            DANO_FILE_INFO_VERSION, config.pwd
        )
        .as_str(),
        output_file,
    )
}

pub fn overwrite_output_file(config: &Config) -> DanoResult<File> {
    if let Ok(mut output_file) = OpenOptions::new()
        // should overwrite the file always
        // FYI append() is for adding to the file
        .write(true)
        // create_new() will only create if DNE
        // create on a file that exists just opens
        .truncate(true)
        .open(&config.output_file)
    {
        print_file_header(config, &mut output_file)?;
        Ok(output_file)
    } else {
        Err(DanoError::new("dano could not open a file to overwrite.").into())
    }
}

fn append_output_file(config: &Config) -> DanoResult<File> {
    // check if output file DNE/is first run
    let is_first_run = !config.output_file.exists();

    if let Ok(mut output_file) = OpenOptions::new()
        // should overwrite the file always
        // FYI append() is for adding to the file
        .append(true)
        // create_new() will only create if DNE
        // create on a file that exists just opens
        .create(true)
        .open(&config.output_file)
    {
        if is_first_run {
            print_file_header(config, &mut output_file)?
        }
        Ok(output_file)
    } else {
        Err(DanoError::new("dano could not open a file to append.").into())
    }
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
    serde_json::from_str(line).map_err(|err| err.into())
}

pub fn read_stdin() -> DanoResult<Vec<String>> {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buffer = Vec::new();
    stdin.read_to_end(&mut buffer)?;

    let broken_string: Vec<String> = std::str::from_utf8(&buffer)?
        // always split on newline or null char
        .split(&['\n', '\0'])
        .into_iter()
        .flat_map(|s| {
            // hacky quote parsing is better than nothing?
            if s.contains('\"') {
                s.split('\"')
                    // unquoted paths should have excess whitespace trimmed
                    .map(|s| s.trim())
                    // remove any empty strings
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<&str>>()
            } else {
                s.split_ascii_whitespace().collect::<Vec<&str>>()
            }
        })
        .map(|i| i.to_owned())
        .collect();

    Ok(broken_string)
}
