// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{canonicalize, read_dir},
    io::Read,
    path::PathBuf,
    time::SystemTime,
};

use clap::{crate_name, crate_version, Arg, ArgMatches};
use itertools::{Either, Itertools};
use rayon::prelude::*;
use util::overwrite_all_paths;

mod lookup_file_info;
mod util;

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::util::{
    deserialize, display_output_path, read_input_file, read_stdin, write_new_paths, DanoError,
};

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
        .arg(Arg::new("TEST").short('t').long("test").display_order(3))
        .arg(
            Arg::new("COMPARE")
                .short('c')
                .long("compare")
                .display_order(4),
        )
        .arg(Arg::new("PRINT").short('p').long("print").display_order(5))
        .arg(
            Arg::new("SILENT")
                .short('s')
                .long("silent")
                .requires("CHECK")
                .requires("WRITE")
                .display_order(6),
        )
        .arg(
            Arg::new("OVERWRITE_OLD")
                .short('o')
                .long("overwrite")
                .conflicts_with_all(&["TEST", "PRINT"])
                .display_order(7),
        )
        .arg(
            Arg::new("WRITE_NEW")
                .short('n')
                .long("write-new")
                .requires("COMPARE")
                .display_order(8),
        )
        .arg(
            Arg::new("DISABLE_FILTER")
                .short('d')
                .long("disable-filter")
                .display_order(9),
        )
        .get_matches()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecMode {
    Test,
    Compare,
    Write,
    Print,
}

#[derive(Debug, Clone)]
pub struct Config {
    exec_mode: ExecMode,
    opt_write_new: bool,
    opt_silent: bool,
    opt_overwrite_old: bool,
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

        let exec_mode = if matches.is_present("COMPARE") {
            ExecMode::Compare
        } else if matches.is_present("TEST") {
            ExecMode::Test
        } else if matches.is_present("PRINT") {
            ExecMode::Print
        } else {
            ExecMode::Write
        };

        let mut paths: Vec<PathBuf> = {
            let res: Vec<PathBuf> = if let Some(input_files) = matches.values_of_os("INPUT_FILES") {
                input_files.par_bridge().map(PathBuf::from).collect()
            } else {
                match &exec_mode {
                    ExecMode::Write => read_stdin()?.par_iter().map(PathBuf::from).collect(),
                    ExecMode::Compare => read_dir(&pwd)?
                        .par_bridge()
                        .flatten()
                        .map(|dir_entry| dir_entry.path())
                        .collect(),
                    ExecMode::Test | ExecMode::Print => Vec::new(),
                }
            };
            parse_paths(&res)
        };

        if paths.is_empty() && matches!(exec_mode, ExecMode::Write | ExecMode::Compare) {
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

        let opt_write_new = matches.is_present("WRITE_NEW");
        let opt_silent = matches.is_present("SILENT");
        let opt_overwrite_old = matches.is_present("OVERWRITE_OLD");

        Ok(Config {
            exec_mode,
            opt_silent,
            opt_write_new,
            opt_overwrite_old,
            pwd,
            paths,
        })
    }
}

fn parse_paths(raw_paths: &[PathBuf]) -> Vec<PathBuf> {
    let auto_extension_filter = include_str!("../data/ffmpeg_extensions_list.txt");

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
        .filter(|path| path.file_name() != Some(OsStr::new("dano_hashes.txt")))
        .filter(|path| {
            auto_extension_filter
                .lines()
                .any(|extension| path.extension() == Some(OsStr::new(extension)))
        })
        .flat_map(canonicalize)
        .collect()
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

    let paths_from_file: Vec<FileInfo> = if config.pwd.join("dano_hashes.txt").exists() {
        let mut input_file = read_input_file(&config.pwd)?;
        let mut buffer = String::new();
        input_file.read_to_string(&mut buffer)?;
        buffer.lines().flat_map(deserialize).collect()
    } else {
        Vec::new()
    };

    match &config.exec_mode {
        ExecMode::Write => {
            let paths_from_input = file_info_from_paths(&config, &config.paths)?;

            let (new_filenames, new_files) =
                compare_hash_collections(&config, &paths_from_file, &paths_from_input)?;

            overwrite_and_write_new(&config, new_filenames, new_files)
        }
        ExecMode::Compare => {
            if paths_from_file.is_empty() {
                return Err(DanoError::new(
                    "Nothing to check or print.  Hash file does not exist.",
                )
                .into());
            }

            let paths_from_input = file_info_from_paths(&config, &config.paths)?;

            let (new_filenames, new_files) =
                compare_hash_collections(&config, &paths_from_file, &paths_from_input)?;

            overwrite_and_write_new(&config, new_filenames, new_files)
        }
        ExecMode::Test => {
            if paths_from_file.is_empty() {
                return Err(DanoError::new(
                    "Nothing to check or print.  Hash file does not exist.",
                )
                .into());
            }

            // recreate the FileInfo struct for the structs retrieved from file to compare
            let paths_to_test: Vec<PathBuf> = paths_from_file
                .iter()
                .map(|file_info| file_info.path.clone())
                .collect();
            let file_info_to_test = file_info_from_paths(&config, &paths_to_test)?;

            // then compare
            let _ = compare_hash_collections(&config, &paths_from_file, &file_info_to_test)?;

            // test will exit on file dne with special exit code so we don't return here
            unreachable!()
        }
        ExecMode::Print => {
            if paths_from_file.is_empty() {
                return Err(DanoError::new(
                    "Nothing to check or print.  Hash file does not exist.",
                )
                .into());
            }

            paths_from_file.iter().try_for_each(display_output_path)?;

            Ok(())
        }
    }
}

fn file_info_from_paths(config: &Config, paths: &[PathBuf]) -> DanoResult<Vec<FileInfo>> {
    let mut hashes: Vec<FileInfo> = paths
        .par_iter()
        .flat_map(|path| FileInfo::new(config, path.as_path()))
        .collect();

    hashes.par_sort_unstable_by_key(|file_info| file_info.path.clone());

    Ok(hashes)
}

fn is_same_hash(
    paths_from_file_map: &BTreeMap<PathBuf, Option<FileMetadata>>,
    path: &FileInfo,
) -> bool {
    let paths_from_file_map_by_hash = paths_from_file_map
        .iter()
        .filter_map(|(path, metadata)| {
            metadata
                .as_ref()
                .map(|metadata| (metadata.hash_value, path))
        })
        .collect::<BTreeMap<u128, &PathBuf>>();

    match &path.metadata {
        Some(metadata) => paths_from_file_map_by_hash.contains_key(&metadata.hash_value),
        None => false,
    }
}

fn overwrite_and_write_new(
    config: &Config,
    new_filenames: Vec<FileInfo>,
    new_files: Vec<FileInfo>,
) -> DanoResult<()> {
    if config.exec_mode == ExecMode::Write
        || (config.exec_mode == ExecMode::Compare && config.opt_write_new) && !new_files.is_empty()
    {
        write_new_paths(config, &new_files)?
    } else if !config.opt_silent {
        eprintln!("No new paths to write.");
    }

    if !new_filenames.is_empty()
        && (config.exec_mode == ExecMode::Write && config.opt_overwrite_old)
        || (config.exec_mode == ExecMode::Compare
            && config.opt_overwrite_old
            && config.opt_write_new)
    {
        // append new paths
        write_new_paths(config, &new_filenames)?;

        // read back
        let paths_from_file_with_duplicates: Vec<FileInfo> =
            if config.pwd.join("dano_hashes.txt").exists() {
                let mut input_file = read_input_file(&config.pwd)?;
                let mut buffer = String::new();
                input_file.read_to_string(&mut buffer)?;
                buffer.lines().flat_map(deserialize).collect()
            } else {
                Vec::new()
            };

        // then dedup
        let unique_paths: Vec<FileInfo> = paths_from_file_with_duplicates
            .iter()
            .into_group_map_by(|file_info| match &file_info.metadata {
                Some(metadata) => metadata.hash_value,
                None => u128::MIN,
            })
            .into_iter()
            .flat_map(|(_hash, group_file_info)| {
                group_file_info
                    .into_iter()
                    .max_by_key(|file_info| match &file_info.metadata {
                        Some(metadata) => metadata.last_checked,
                        None => SystemTime::UNIX_EPOCH,
                    })
            })
            .cloned()
            .collect();

        // and overwrite
        overwrite_all_paths(config, &unique_paths)
    } else {
        Ok(())
    }
}

fn is_same_filename(
    paths_from_file_map: &BTreeMap<PathBuf, Option<FileMetadata>>,
    path: &FileInfo,
) -> bool {
    paths_from_file_map.contains_key(&path.path)
}

fn compare_hash_collections(
    config: &Config,
    paths_from_file: &[FileInfo],
    requested_paths: &[FileInfo],
) -> DanoResult<(Vec<FileInfo>, Vec<FileInfo>)> {
    let (new_filenames, new_files) = match config.exec_mode {
        ExecMode::Compare | ExecMode::Write => {
            compare_check(config, paths_from_file, requested_paths)
        }
        ExecMode::Test => {
            let exit_code = test_check(paths_from_file, requested_paths);
            // exit with code 2 on finding a file that DNE
            std::process::exit(exit_code);
        }
        ExecMode::Print => unreachable!(),
    };

    Ok((new_filenames, new_files))
}

fn compare_check(
    config: &Config,
    paths_from_file: &[FileInfo],
    requested_paths: &[FileInfo],
) -> (Vec<FileInfo>, Vec<FileInfo>) {
    let paths_from_file_map: BTreeMap<PathBuf, Option<FileMetadata>> = paths_from_file
        .par_iter()
        .cloned()
        .map(|file_info| (file_info.path, file_info.metadata))
        .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>();

    let (new_filenames, new_files) = requested_paths
        .par_iter()
        .filter_map(|file_info| {
            let is_same_hash = is_same_hash(&paths_from_file_map, file_info);
            let is_same_filename = is_same_filename(&paths_from_file_map, file_info);

            if is_same_filename && is_same_hash {
                if config.exec_mode != ExecMode::Write {
                    eprintln!("{:?}: OK", file_info.path);
                }
                None
            } else if is_same_filename {
                if config.exec_mode != ExecMode::Write {
                    eprintln!(
                        "{:?}: WARNING, path has new hash for same filename",
                        file_info.path
                    );
                }
                None
            } else if is_same_hash {
                if config.exec_mode != ExecMode::Write {
                    // know we are in Compare mode, so require write_new and overwrite_old
                    // to specify things will be overwritten
                    if config.opt_write_new && config.opt_overwrite_old {
                        eprintln!(
                            "{:?}: OK, but path has same hash for new filename.  Hash data will be overwritten.",
                            file_info.path
                        );
                    } else {
                        eprintln!(
                            "{:?}: OK, but path has same hash for new filename",
                            file_info.path
                        );
                    }
                }
                Some((file_info.clone(), is_same_hash))
            } else {
                if config.exec_mode != ExecMode::Write {
                    eprintln!("{:?}: Path is a new file", file_info.path);
                }
                Some((file_info.clone(), is_same_hash))
            }
        })
        .partition_map(|(file_info, same_hash)| {
            if same_hash {
                Either::Left(file_info)
            } else {
                Either::Right(file_info)
            }
        });
    (new_filenames, new_files)
}

fn test_check(paths_from_file: &[FileInfo], requested_paths: &[FileInfo]) -> i32 {
    let requested_paths_map: BTreeMap<PathBuf, Option<FileMetadata>> = requested_paths
        .par_iter()
        .cloned()
        .map(|file_info| (file_info.path, file_info.metadata))
        .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>();

    let exit_code = paths_from_file.iter().fold(0, |mut exit_code, file_info| {
        let is_same_hash = is_same_hash(&requested_paths_map, file_info);
        let is_same_filename = is_same_filename(&requested_paths_map, file_info);

        if is_same_filename && is_same_hash {
            eprintln!("{:?}: OK", file_info.path);
        } else if is_same_filename {
            eprintln!(
                "{:?}: WARNING, path has new hash for same filename",
                file_info.path
            );
        } else if is_same_hash {
            eprintln!(
                "{:?}: OK, but path has same hash for new filename",
                file_info.path
            );
        } else {
            eprintln!("{:?}: WARNING, path does not exist", file_info.path);
            exit_code = 2;
        }

        exit_code
    });

    exit_code
}
