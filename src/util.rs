// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICE&NSE file
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

pub fn overwrite_all_paths(config: &Config, new_files: &[FileInfo]) -> DanoResult<()> {
    let mut output_file = overwrite_output_file(config)?;

    new_files
        .iter()
        .try_for_each(|file_info| write_path(config, file_info, &mut output_file))
}

pub fn write_new_paths(config: &Config, new_files: &[FileInfo]) -> DanoResult<()> {
    let mut output_file = append_output_file(config)?;

    new_files
        .iter()
        .try_for_each(|file_info| write_path(config, file_info, &mut output_file))
}

fn write_path(config: &Config, file_info: &FileInfo, output_file: &mut File) -> DanoResult<()> {
    match &file_info.metadata {
        Some(_metadata) => {
            let serialized = if config.opt_xattr {
                // write empty path for path, because we have the actual path
                let rewrite = FileInfo {
                    version: file_info.version,
                    path: PathBuf::new(),
                    metadata: file_info.metadata.to_owned(),
                };
                serialize(&rewrite)?
            } else {
                serialize(file_info)?
            };

            let out_string = serialized + "\n";

            match &config.exec_mode {
                ExecMode::Write(dry_run) if dry_run == &DryRun::Enabled => {
                    print_out_buf(&out_string)?
                }
                ExecMode::Write(_) if config.opt_xattr => write_out_xattr(&out_string, file_info)?,
                _ => write_out(&out_string, output_file)?,
            }
            Ok(())
        }
        None => Ok(()),
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
    write_out(
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
