// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.
use std::{
    ffi::OsStr,
    fs::read_dir,
    io::Read,
    path::{Path, PathBuf},
};

use clap::{crate_name, crate_version, Arg, ArgMatches};
use rayon::prelude::*;

mod lookup_file_info;
mod process_file_info;
mod util;

use crate::lookup_file_info::{exec_lookup_file_info, FileInfo};
use crate::util::{deserialize, print_file_info, read_input_file, read_stdin, DanoError};
use process_file_info::{exec_process_file_info, write_to_file};

pub type DanoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const FILE_INFO_VERSION: usize = 1;

fn parse_args() -> ArgMatches {
    clap::Command::new(crate_name!())
        .about("dano is a wrapper for ffmpeg that hashes the internal file streams of certain media files, \
        and stores them in a format which can be used to verify such hashes later.  This is handy, because, \
        should you choose to change metadata tags, or change file names, the media hashes will remain the same.")
        .version(crate_version!())
        .arg(
            Arg::new("INPUT_FILES")
                .help("input files to be hashed.  INPUT_FILE can also be read from stdin for NULL or NEWLINE delimited inputs.")
                .takes_value(true)
                .multiple_values(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(1),
        )
        .arg(
            Arg::new("OUTPUT_FILE")
                .help("output file which will hold the hashes. If not specified, the 'dano_hashes.txt' will be used in the PWD.")
                .long("output-file")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(2),
        )
        .arg(
            Arg::new("HASH_FILE")
                .help("file from which to read the hashes.  If not specified, the output file will be used (or if not specified 'dano_hashes.txt' in the PWD).")
                .long("hash-file")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(3),
        )
        .arg(
            Arg::new("WRITE")
                .help("write the input files hashes to disk.")
                .short('w')
                .long("write")
                .display_order(4))
        .arg(Arg::new("TEST").short('t').long("test").display_order(5))
        .arg(
            Arg::new("COMPARE")
                .help("compare the input files to the hashes located in the hash file.")
                .short('c')
                .long("compare")
                .display_order(6),
        )
        .arg(
            Arg::new("PRINT")
            .help("pretty print the hashes in the hash file.")
            .short('p')
            .long("print")
            .display_order(7))
        .arg(
            Arg::new("SILENT")
                .help("quiet many informational messages while in WRITE mode.")
                .short('s')
                .long("silent")
                .requires("WRITE")
                .display_order(8),
        )
        .arg(
            Arg::new("OVERWRITE_OLD")
                .help("if one file's hash matches another's, but they have different file name's, overwrite the old file info with the most current file info.")
                .long("overwrite")
                .conflicts_with_all(&["TEST", "PRINT"])
                .display_order(9),
        )
        .arg(
            Arg::new("WRITE_NEW")
                .help("if new files are present in COMPARE mode, append such file info to the hash file.")
                .long("write-new")
                .requires("COMPARE")
                .display_order(10),
        )
        .arg(
            Arg::new("DISABLE_FILTER")
                .help("by default, dano filters file extensions recognized by ffmpeg.  Disable such filtering here.")
                .long("disable-filter")
                .display_order(11),
        )
        .arg(
            Arg::new("CANONICAL_PATHS")
                .help("use canonical paths instead of potentially relative paths.")
                .long("canonical-paths")
                .display_order(12),
        )
        .arg(
            Arg::new("DRY_RUN")
            .help("print the information to stdout that would be written to disk.")
            .long("dry-run")
            .requires("WRITE")
            .display_order(13))
        .get_matches()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DryRun {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecMode {
    Test,
    Compare,
    Write(DryRun),
    Print,
}

#[derive(Debug, Clone)]
pub struct Config {
    exec_mode: ExecMode,
    opt_write_new: bool,
    opt_silent: bool,
    opt_overwrite_old: bool,
    pwd: PathBuf,
    output_file: PathBuf,
    hash_file: PathBuf,
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
        } else if matches.is_present("PRINT") && !matches.is_present("WRITE") {
            ExecMode::Print
        } else if matches.is_present("DRY_RUN")
            || (matches.is_present("PRINT") && matches.is_present("WRITE"))
        {
            ExecMode::Write(DryRun::Enabled)
        } else {
            ExecMode::Write(DryRun::Disabled)
        };

        let opt_write_new = matches.is_present("WRITE_NEW");
        let opt_silent = matches.is_present("SILENT");
        let opt_overwrite_old = matches.is_present("OVERWRITE_OLD");
        let opt_disable_filter = matches.is_present("DISABLE_FILTER");
        let opt_canonical_paths = matches.is_present("CANONICAL_PATHS");

        let output_file = if let Some(output_file) = matches.value_of_os("OUTPUT_FILE") {
            PathBuf::from(output_file)
        } else {
            pwd.join("dano_hashes.txt")
        };

        let hash_file = if let Some(hash_file) = matches.value_of_os("HASH_FILE") {
            PathBuf::from(hash_file)
        } else {
            output_file.clone()
        };

        let paths: Vec<PathBuf> = {
            let res: Vec<PathBuf> = if let Some(input_files) = matches.values_of_os("INPUT_FILES") {
                input_files.par_bridge().map(PathBuf::from).collect()
            } else {
                match &exec_mode {
                    ExecMode::Write(_) => read_stdin()?.par_iter().map(PathBuf::from).collect(),
                    ExecMode::Compare => read_dir(&pwd)?
                        .par_bridge()
                        .flatten()
                        .map(|dir_entry| dir_entry.path())
                        .collect(),
                    ExecMode::Test | ExecMode::Print => Vec::new(),
                }
            };
            parse_paths(&res, opt_disable_filter, opt_canonical_paths, &hash_file)
        };

        if paths.is_empty() && matches!(exec_mode, ExecMode::Write(_) | ExecMode::Compare) {
            return Err(DanoError::new("No valid paths to search.").into());
        }

        Ok(Config {
            exec_mode,
            opt_silent,
            opt_write_new,
            opt_overwrite_old,
            pwd,
            output_file,
            hash_file,
            paths,
        })
    }
}

fn parse_paths(
    raw_paths: &[PathBuf],
    opt_disable_filter: bool,
    opt_canonical_paths: bool,
    hash_file: &Path,
) -> Vec<PathBuf> {
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
        .filter(|path| path.file_name() != Some(OsStr::new(hash_file)))
        .filter(|path| {
            if !opt_disable_filter {
                auto_extension_filter
                    .lines()
                    .any(|extension| path.extension() == Some(OsStr::new(extension)))
            } else {
                true
            }
        })
        .map(|path| {
            if opt_canonical_paths {
                path.canonicalize().unwrap_or_else(|_| path.to_owned())
            } else {
                path.to_owned()
            }
        })
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

    let paths_from_file: Vec<FileInfo> = if config.output_file.exists() {
        let mut input_file = read_input_file(&config)?;
        let mut buffer = String::new();
        input_file.read_to_string(&mut buffer)?;
        buffer.lines().flat_map(deserialize).collect()
    } else {
        Vec::new()
    };

    match &config.exec_mode {
        ExecMode::Write(_) | ExecMode::Compare => {
            if paths_from_file.is_empty() && matches!(config.exec_mode, ExecMode::Compare) {
                return Err(DanoError::new(
                    "Nothing to check or print.  Hash file does not exist.",
                )
                .into());
            }

            let rx_item = exec_lookup_file_info(&config.paths)?;

            let compare_hashes_bundle =
                exec_process_file_info(&config, &config.paths, &paths_from_file, rx_item)?;

            write_to_file(&config, &compare_hashes_bundle)
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

            let rx_item = exec_lookup_file_info(&paths_to_test)?;

            let _ = exec_process_file_info(&config, &paths_to_test, &paths_from_file, rx_item)?;

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

            paths_from_file.iter().try_for_each(print_file_info)?;

            Ok(())
        }
    }
}
