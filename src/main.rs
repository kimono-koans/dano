// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    ffi::OsStr,
    fs::{canonicalize, read_dir},
    io::Read,
    path::PathBuf,
};

use clap::{crate_name, crate_version, Arg, ArgMatches};
use rayon::prelude::*;

mod check_test_write;
mod lookup_file_info;
mod util;

use crate::lookup_file_info::FileInfo;
use crate::util::{deserialize, display_file_info, read_input_file, read_stdin, DanoError};
use check_test_write::{file_info_from_paths, overwrite_and_write_new};

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

        let paths: Vec<PathBuf> = {
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
            let new_file_bundle = file_info_from_paths(&config, &config.paths, &paths_from_file)?;

            overwrite_and_write_new(&config, &new_file_bundle)
        }
        ExecMode::Compare => {
            if paths_from_file.is_empty() {
                return Err(DanoError::new(
                    "Nothing to check or print.  Hash file does not exist.",
                )
                .into());
            }

            let new_file_bundle = file_info_from_paths(&config, &config.paths, &paths_from_file)?;

            overwrite_and_write_new(&config, &new_file_bundle)
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
            let _ = file_info_from_paths(&config, &paths_to_test, &paths_from_file)?;

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

            paths_from_file.iter().for_each(display_file_info);

            Ok(())
        }
    }
}
