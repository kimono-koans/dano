// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    fs::{canonicalize, read_dir, File, OpenOptions},
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
        .arg(Arg::new("WRITE").short('w').long("write").display_order(2))
        .arg(Arg::new("READ").short('r').long("read").display_order(3))
        .arg(
            Arg::new("WRITE_NEW")
                .short('n')
                .long("write-new")
                .display_order(4),
        )
        .get_matches()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileInfo {
    path: PathBuf,
    metadata: Option<FileMetadata>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileMetadata {
    hash_algo: Box<str>,
    hash_value: u128,
    last_checked: SystemTime,
    modify_time: SystemTime,
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
                                last_checked: timestamp.to_owned(),
                                hash_algo: first.into(),
                                hash_value: { u128::from_str_radix(last, 16)? },
                                modify_time: path.metadata()?.modified()?,
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

#[derive(Debug, Clone)]
enum ExecMode {
    ReadHashes,
    WriteHashes,
}

#[derive(Debug, Clone)]
pub struct Config {
    exec_mode: ExecMode,
    write_new: bool,
    pwd: PathBuf,
    paths: Vec<PathBuf>,
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

        let exec_mode = if matches.is_present("READ") {
            ExecMode::ReadHashes
        } else {
            ExecMode::WriteHashes
        };

        let parse_paths = |raw_paths: Vec<&Path>| -> Vec<PathBuf> {
            raw_paths
                .into_par_iter()
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

        let mut paths: Vec<PathBuf> = if let Some(input_files) = matches.values_of_os("INPUT_FILES")
        {
            parse_paths(input_files.par_bridge().map(Path::new).collect())
        } else {
            match &exec_mode {
                ExecMode::WriteHashes => {
                    parse_paths(read_stdin()?.par_iter().map(Path::new).collect())
                }
                ExecMode::ReadHashes => read_dir(&pwd)?
                    .par_bridge()
                    .flatten()
                    .map(|dir_entry| dir_entry.path())
                    .collect(),
            }
        };

        if paths.is_empty() {
            return Err(DanoError::new("No valid paths to search.").into());
        }

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

        let write_new = matches.is_present("WRITE_NEW");

        Ok(Config {
            exec_mode,
            write_new,
            pwd,
            paths,
        })
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

    let mut input_file = read_input_file(&config.pwd)?;
    let mut buffer = String::new();
    input_file.read_to_string(&mut buffer)?;

    let hashes_from_file: Vec<FileInfo> = buffer.lines().flat_map(deserialize).collect();
    let hashes_from_paths = hashes_from_paths(&config.paths)?;

    match &config.exec_mode {
        ExecMode::WriteHashes => {
            let (new_files, _) = partition_new_old_files(&hashes_from_file, &hashes_from_paths);

            write_new_files(&config, &new_files)
        }
        ExecMode::ReadHashes => {
            let new_files = compare_hash_collections(&hashes_from_file, &hashes_from_paths)?;

            if !new_files.is_empty() && config.write_new {
                write_new_files(&config, &new_files)?;
            }

            Ok(())
        }
    }
}

fn write_new_files(config: &Config, new_files: &[FileInfo]) -> DanoResult<()> {
    let mut output_file = write_output_file(&config.pwd)?;

    new_files.iter().try_for_each(|file_info| {
        eprintln!("Writing new file: {:?}", file_info.path);
        let serialized = serialize(file_info)?;
        let out_string = serialized + "\n";
        write_out(&out_string, &mut output_file)?;
        Ok(())
    })
}

fn partition_new_old_files(
    hashes_from_file: &[FileInfo],
    hashes_from_paths: &[FileInfo],
) -> (Vec<FileInfo>, Vec<FileInfo>) {
    let hashes_from_file_map: BTreeMap<PathBuf, Option<FileMetadata>> = hashes_from_file
        .iter()
        .cloned()
        .map(|file_info| (file_info.path, file_info.metadata))
        .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>();

    hashes_from_paths
        .iter()
        .cloned()
        .partition(|file_info| !hashes_from_file_map.contains_key(&file_info.path))
}

fn compare_hash_collections(
    hashes_from_file: &[FileInfo],
    hashes_from_paths: &[FileInfo],
) -> DanoResult<Vec<FileInfo>> {
    let hashes_from_file_map: BTreeMap<PathBuf, Option<FileMetadata>> = hashes_from_file
        .iter()
        .cloned()
        .map(|file_info| (file_info.path, file_info.metadata))
        .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>();

    let (new_files, _): (Vec<FileInfo>, Vec<FileInfo>) =
        partition_new_old_files(hashes_from_file, hashes_from_paths);

    let phantom_files: Vec<FileInfo> = hashes_from_file
        .iter()
        .chain(hashes_from_paths.iter())
        .filter(|file_info| file_info.metadata.is_none())
        .cloned()
        .collect();

    let (modified_files, suspicious_modified_files): (Vec<FileInfo>, Vec<FileInfo>) =
        hashes_from_paths
            .iter()
            .filter(|file_info| file_info.metadata.is_some())
            .filter(|file_info| hashes_from_file_map.get(&file_info.path).is_some())
            .filter(|file_info| hashes_from_file_map.get(&file_info.path).unwrap().is_some())
            .cloned()
            // known okay to unwrap because we filter on the two conditions above
            .partition(|file_info| {
                let map_entry = hashes_from_file_map
                    .get(&file_info.path)
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap();
                map_entry.modify_time == file_info.to_owned().metadata.unwrap().modify_time
            });

    eprintln!("New files:");
    new_files
        .iter()
        .for_each(|file_info| eprintln!("\t{:?}", file_info.path));

    eprintln!("Phantom files:\n{:?}", phantom_files);
    phantom_files
        .iter()
        .for_each(|file_info| eprintln!("\t{:?}", file_info.path));

    eprintln!("Modified files:\n{:?}", modified_files);
    modified_files
        .iter()
        .for_each(|file_info| eprintln!("\t{:?}", file_info.path));

    eprintln!("Suspicious files:\n{:?}", suspicious_modified_files);
    suspicious_modified_files
        .iter()
        .for_each(|file_info| eprintln!("\t{:?}", file_info.path));

    Ok(new_files)
}

fn read_input_file(pwd: &Path) -> DanoResult<File> {
    if let Ok(input_file) = OpenOptions::new()
        .read(true)
        .open(pwd.join("dano_hashes.txt"))
    {
        Ok(input_file)
    } else {
        Err(DanoError::new("dano could not open a file to write to...").into())
    }
}

fn write_output_file(pwd: &Path) -> DanoResult<File> {
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
        Err(DanoError::new("dano could not open a file to write to...").into())
    }
}

fn write_out(out_string: &str, open_file: &mut File) -> DanoResult<()> {
    open_file
        .write_all(out_string.as_bytes())
        .map_err(|err| err.into())
}

fn serialize(file_info: &FileInfo) -> DanoResult<String> {
    serde_json::to_string(&file_info).map_err(|err| err.into())
}

fn deserialize(line: &str) -> DanoResult<FileInfo> {
    serde_json::from_str(line).map_err(|err| err.into())
}

fn hashes_from_paths(paths: &[PathBuf]) -> DanoResult<Vec<FileInfo>> {
    let mut hashes: Vec<FileInfo> = paths
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
