// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    error::Error,
    fmt,
    fs::{canonicalize, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command as ExecProcess,
    time::SystemTime,
};

use clap::{crate_name, crate_version, Arg, ArgMatches};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use which::which;

pub type DanoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn parse_args() -> ArgMatches {
    clap::Command::new(crate_name!())
        .about("")
        .version(crate_version!())
        .arg(
            Arg::new("INPUT_FILES")
                .help("")
                .takes_value(true)
                .multiple_values(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(1),
        )
        .get_matches()
}

#[derive(Debug, Clone)]
pub struct Config {
    pwd: PathBuf,
    paths: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileInfo {
    path: PathBuf,
    metadata: Option<FileMetadata>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileMetadata {
    hash_algo: Box<str>,
    hash_string: Box<str>,
    timestamp: SystemTime,
}

impl FileInfo {
    fn new(path: &Path) -> DanoResult<Self> {
        fn get_file_info(path: &Path) -> DanoResult<FileInfo> {
            fn exec_ffmpeg(path: &Path, ffmpeg_command: &Path) -> DanoResult<FileInfo> {
                // all snapshots should have the same timestamp
                let timestamp = &SystemTime::now();
                let path_clone = path.to_string_lossy();

                let process_args = vec![
                    "-i",
                    path_clone.as_ref(),
                    "-codec",
                    "copy",
                    "-f",
                    "hash",
                    "-hash",
                    "murmur3",
                    "-",
                ];
                let process_output = ExecProcess::new(ffmpeg_command)
                    .args(&process_args)
                    .output()?;
                let stdout_string = std::str::from_utf8(&process_output.stdout)?.trim();

                // stderr_string is a string not an error, so here we build an err or output
                if stdout_string.is_empty() {
                    Err(DanoError::new("Unable to exec ffmpeg").into())
                } else {
                    match stdout_string.split_once('=') {
                        Some((first, last)) => Ok(FileInfo {
                            path: path.to_owned(),
                            metadata: Some(FileMetadata {
                                timestamp: timestamp.to_owned(),
                                hash_algo: first.into(),
                                hash_string: last.into(),
                            }),
                        }),
                        None => Ok(FileInfo {
                            path: path.to_owned(),
                            metadata: None,
                        }),
                    }
                }
            }

            if let Ok(ffmpeg_command) = which("ffmpeg") {
                exec_ffmpeg(path, &ffmpeg_command)
            } else {
                Err(DanoError::new(
                    "'ffmpeg' command not found. Make sure the command 'zfs' is in your path.",
                )
                .into())
            }
        }

        get_file_info(path)
    }
}

impl Config {
    fn new() -> DanoResult<Self> {
        let arg_matches = parse_args();
        Config::from_matches(arg_matches)
    }

    fn from_matches(matches: ArgMatches) -> DanoResult<Self> {
        // current working directory will be helpful in a number of places
        let pwd = if let Ok(pwd) = std::env::current_dir() {
            if let Ok(path) = PathBuf::from(&pwd).canonicalize() {
                PathBuf::from(path.as_path())
            } else {
                return Err(DanoError::new(
                    "Could not obtain a canonical path for your working directory",
                )
                .into());
            }
        } else {
            return Err(DanoError::new(
                "Working directory does not exist or your do not have permissions to access it.",
            )
            .into());
        };

        let mut paths: Vec<PathBuf> = if let Some(input_files) = matches.values_of_os("INPUT_FILES")
        {
            input_files
                .par_bridge()
                .map(Path::new)
                .filter(|path| {
                    if path.exists() {
                        true
                    } else {
                        eprintln!("Path {:?} does not exist", path);
                        false
                    }
                })
                .flat_map(canonicalize)
                .collect()
        } else {
            read_stdin()?
                .par_iter()
                .map(Path::new)
                .filter(|path| {
                    if path.exists() {
                        true
                    } else {
                        eprintln!("Path {:?} does not exist", path);
                        false
                    }
                })
                .flat_map(canonicalize)
                .collect()
        };

        // deduplicate path_buf and sort --
        // so input of ./.z* and ./.zshrc will only print ./.zshrc once
        paths = if paths.len() > 1 {
            paths.sort_unstable();
            // dedup needs to be sorted/ordered first to work (not like a BTreeMap)
            paths.dedup();

            paths
        } else {
            paths
        };

        Ok(Config { pwd, paths })
    }
}

fn main() {
    match exec() {
        Ok(_) => std::process::exit(0),
        Err(error) => {
            eprintln!("Error: {}", error);
            std::process::exit(1)
        }
    }
}

fn exec() -> DanoResult<()> {
    let config = Config::new()?;

    let hashes = get_hashes(&config)?;

    // creates script file in user's home dir or will fail if file already exists
    let mut dano_file = if let Ok(dano_file) = OpenOptions::new()
        // should overwrite the file always
        // FYI append() is for adding to the file
        .append(true)
        // create_new() will only create if DNE
        // create on a file that exists just opens
        .create(true)
        .open(config.pwd.join("dano_hashes.txt"))
    {
        dano_file
    } else {
        return Err(DanoError::new("dano could not open a file to write to...").into());
    };

    let res: DanoResult<()> = hashes
        .into_iter().try_for_each(|file_info| {
            let serialized = serialize_to_json(&file_info)?;
            let out_string = serialized + "\n";
            write_out(&out_string, &mut dano_file)?;
            Ok(())
        });

    res
}

fn write_out(out_string: &str, open_file: &mut File) -> DanoResult<()> {
    open_file
        .write_all(out_string.as_bytes())
        .map_err(|err| err.into())
}

fn serialize_to_json(file_info: &FileInfo) -> DanoResult<String> {
    serde_json::to_string(&file_info).map_err(|err| err.into())
}

fn get_hashes(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let mut hashes: Vec<FileInfo> = config
        .paths
        .clone()
        .par_iter()
        .flat_map(|path| FileInfo::new(path.as_path()))
        .collect();

    hashes.par_sort_unstable_by_key(|file_info| file_info.path.clone());

    Ok(hashes)
}

pub fn read_stdin() -> DanoResult<Vec<String>> {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buffer = Vec::new();
    stdin.read_to_end(&mut buffer)?;

    let broken_string: Vec<String> = std::str::from_utf8(&buffer)?
        .split_ascii_whitespace()
        .into_iter()
        .map(|i| i.to_owned())
        .collect();

    Ok(broken_string)
}

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
