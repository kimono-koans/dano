// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    error::Error,
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use crate::lookup_file_info::FileInfo;
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

pub fn overwrite_all_paths(config: &Config, new_files: &[FileInfo]) -> DanoResult<()> {
    let mut output_file = overwrite_output_file(&config.pwd)?;

    new_files
        .iter()
        .try_for_each(|file_info| write_path(file_info, &mut output_file))
}

pub fn write_new_paths(config: &Config, new_files: &[FileInfo]) -> DanoResult<()> {
    let mut output_file = append_output_file(&config.pwd)?;

    new_files
        .iter()
        .try_for_each(|file_info| write_path(file_info, &mut output_file))
}

fn write_path(file_info: &FileInfo, output_file: &mut File) -> DanoResult<()> {
    match &file_info.metadata {
        Some(_metadata) => {
            let serialized = serialize(file_info)?;
            let out_string = serialized + "\n";
            write_out(&out_string, output_file)?;
            Ok(())
        }
        None => Ok(()),
    }
}

pub fn display_file_info(file_info: &FileInfo) {
    match &file_info.metadata {
        Some(metadata) => {
            eprintln!(
                "{}={:x} : {:?}",
                metadata.hash_algo, metadata.hash_value, file_info.path
            );
        }
        None => {
            eprintln!(
                "WARNING: Could not generate checksum for: {:?}",
                file_info.path
            );
        }
    }
}

pub fn read_input_file(pwd: &Path) -> DanoResult<File> {
    if let Ok(input_file) = OpenOptions::new()
        .read(true)
        .open(pwd.join("dano_hashes.txt"))
    {
        Ok(input_file)
    } else {
        Err(DanoError::new("dano could not open a file to write to").into())
    }
}

pub fn overwrite_output_file(pwd: &Path) -> DanoResult<File> {
    // creates script file in user's home dir or will fail if file already exists
    if let Ok(output_file) = OpenOptions::new()
        // should overwrite the file always
        // FYI append() is for adding to the file
        .write(true)
        // create_new() will only create if DNE
        // create on a file that exists just opens
        .truncate(true)
        .open(pwd.join("dano_hashes.txt"))
    {
        Ok(output_file)
    } else {
        Err(DanoError::new("dano could not open a file to write to.").into())
    }
}

fn append_output_file(pwd: &Path) -> DanoResult<File> {
    // creates script file in user's home dir or will fail if file already exists
    if let Ok(output_file) = OpenOptions::new()
        // should overwrite the file always
        // FYI append() is for adding to the file
        .append(true)
        // create_new() will only create if DNE
        // create on a file that exists just opens
        .create(true)
        .open(pwd.join("dano_hashes.txt"))
    {
        Ok(output_file)
    } else {
        Err(DanoError::new("dano could not open a file to write to.").into())
    }
}

fn write_out(out_string: &str, open_file: &mut File) -> DanoResult<()> {
    open_file
        .write_all(out_string.as_bytes())
        .map_err(|err| err.into())
}

pub fn serialize(file_info: &FileInfo) -> DanoResult<String> {
    serde_json::to_string(&file_info).map_err(|err| err.into())
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
        .split(&['\n', '\0'])
        .filter(|i| !i.is_empty())
        .into_iter()
        .map(|i| i.to_owned())
        .collect();

    Ok(broken_string)
}
