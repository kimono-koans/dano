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
    ffi::OsStr,
    fs::{read_dir},
    io::Read,
    path::{Path, PathBuf}, collections::BTreeMap,
};

use clap::{crate_name, crate_version, Arg, ArgMatches};
use rayon::{prelude::*, ThreadPool};

mod lookup_file_info;
mod process_file_info;
mod util;

use crate::lookup_file_info::{exec_lookup_file_info, FileInfo};
use crate::util::{deserialize, print_file_info, read_input_file, read_stdin, DanoError};
use process_file_info::{exec_process_file_info, write_new_file_info};

pub type DanoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DANO_FILE_INFO_VERSION: usize = 1;
const DANO_XATTR_KEY_NAME: &str = "user.dano.checksum";
const DANO_DEFAULT_HASH_FILE: &str = "dano_hashes.txt";

fn parse_args() -> ArgMatches {
    clap::Command::new(crate_name!())
        .about("dano is a wrapper for ffmpeg that checksums the internal file streams of certain media files, \
        and stores them in a format which can be used to verify such checksums later.  This is handy, because, \
        should you choose to change metadata tags, or change file names, the media checksums *should* remain the same.")
        .version(crate_version!())
        .arg(
            Arg::new("INPUT_FILES")
                .help("input files to be hashed or checked.  INPUT_FILES can also be read from stdin for NULL or NEWLINE delimited inputs.")
                .takes_value(true)
                .multiple_values(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(1),
        )
        .arg(
            Arg::new("OUTPUT_FILE")
                .help("output file which will hold the hashes. If not specified, 'dano_hashes.txt' in the PWD will be used.")
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
                .short('k')
                .long("hash-file")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(3),
        )
        .arg(
            Arg::new("WRITE")
                .help("write the input files' hashes.")
                .short('w')
                .long("write")
                .display_order(4))
        .arg(
            Arg::new("TEST")
                .help("compare the hashes in a hash file to the files currently on disk.")
                .short('t')
                .long("test")
                .display_order(5))
        .arg(
            Arg::new("COMPARE")
                .help("compare the input files to the hashes.")
                .short('c')
                .long("compare")
                .display_order(6),
        )
        .arg(
            Arg::new("PRINT")
            .help("pretty print hashes.")
            .short('p')
            .long("print")
            .display_order(7))
        .arg(
            Arg::new("NUM_THREADS")
                .help("requested number of threads to use for file processing.  Default is twice the number of logical cores.")
                .short('j')
                .long("threads")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(7))
        .arg(
            Arg::new("SILENT")
                .help("quiet many informational messages (like \"OK\").")
                .short('s')
                .long("silent")
                .requires("WRITE")
                .display_order(8),
        )
        .arg(
            Arg::new("WRITE_NEW")
                .help("if new files are present in COMPARE mode, append such file info.")
                .long("write-new")
                .requires("COMPARE")
                .display_order(9),
        )
        .arg(
            Arg::new("OVERWRITE_OLD")
                .help("if one file's hash matches another's, but they have different file name's, overwrite the old file's info with the most current.")
                .long("overwrite")
                .conflicts_with_all(&["TEST", "PRINT"])
                .display_order(10),
        )
        .arg(
            Arg::new("DISABLE_FILTER")
                .help("by default, dano filters file extensions not recognized by ffmpeg.  Here, you may disable such filtering.")
                .long("disable-filter")
                .display_order(11),
        )
        .arg(
            Arg::new("CANONICAL_PATHS")
                .help("use canonical paths (instead of potentially relative paths).")
                .long("canonical-paths")
                .display_order(12),
        )
        .arg(
            Arg::new("XATTR")
                .help("try to write (dano will always try to read) hash to/from file's extended attributes.  \
                Can also be enabled by setting environment variable DANO_XATTR_WRITES to any value.")
                .short('x')
                .long("xattr")
                .display_order(12),
        )
        .arg(
            Arg::new("HASH_ALGO")
                .help("specify the algorithm to use for hashing.  Default is 'murmur3'.")
                .long("hash-algo")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .possible_values(&["murmur3", "MD5", "CRC32", "adler32"])
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(13))
        .arg(
            Arg::new("DRY_RUN")
            .help("print the information to stdout that would be written to disk.")
            .long("dry-run")
            .requires("WRITE")
            .display_order(14))
        .get_matches()
}

#[derive(Debug, Clone)]
pub struct FileInfoRequest {
    pub path: PathBuf,
    pub hash_algo: Option<Box<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DryRunMode {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum XattrMode {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WriteModeConfig {
    opt_xattr: XattrMode,
    opt_dry_run: DryRunMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecMode {
    Test,
    Compare,
    Write(WriteModeConfig),
    Print,
}

#[derive(Debug, Clone)]
pub struct Config {
    exec_mode: ExecMode,
    opt_num_threads: Option<usize>,
    opt_write_new: bool,
    opt_silent: bool,
    opt_overwrite_old: bool,
    hash_algo: Box<str>,
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

        let opt_xattr =
            if matches.is_present("XATTR") || std::env::var_os("DANO_XATTR_WRITES").is_some() {
                XattrMode::Enabled
            } else {
                XattrMode::Disabled
            };

        let opt_dry_run = if matches.is_present("DRY_RUN")
            || (matches.is_present("PRINT") && matches.is_present("WRITE"))
        {
            DryRunMode::Enabled
        } else {
            DryRunMode::Disabled
        };

        let exec_mode = if matches.is_present("COMPARE") {
            ExecMode::Compare
        } else if matches.is_present("TEST") {
            ExecMode::Test
        } else if matches.is_present("PRINT") && !matches.is_present("WRITE") {
            ExecMode::Print
        } else if matches.is_present("WRITE") {
            ExecMode::Write(WriteModeConfig {
                opt_xattr,
                opt_dry_run,
            })
        } else {
            return Err(DanoError::new(
                "You must specify an execution mode: COMPARE, TEST, WRITE, or PRINT",
            )
            .into());
        };

        let opt_num_threads = matches
            .value_of_lossy("NUM_THREADS")
            .and_then(|num_threads_str| num_threads_str.parse::<usize>().ok());
        let opt_write_new = matches.is_present("WRITE_NEW");
        let opt_silent = matches.is_present("SILENT");
        let opt_overwrite_old = matches.is_present("OVERWRITE_OLD");
        let opt_disable_filter = matches.is_present("DISABLE_FILTER");
        let opt_canonical_paths = matches.is_present("CANONICAL_PATHS");

        let output_file = if let Some(output_file) = matches.value_of_os("OUTPUT_FILE") {
            PathBuf::from(output_file)
        } else {
            pwd.join(DANO_DEFAULT_HASH_FILE)
        };

        let hash_algo = if let Some(hash_algo) = matches.value_of_os("HASH_ALGO") {
            hash_algo.to_string_lossy().into()
        } else {
            "murmur3".into()
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
                    ExecMode::Test | ExecMode::Write(_) => {
                        read_stdin()?.par_iter().map(PathBuf::from).collect()
                    }
                    ExecMode::Compare => read_dir(&pwd)?
                        .par_bridge()
                        .flatten()
                        .map(|dir_entry| dir_entry.path())
                        .collect(),
                    ExecMode::Print => Vec::new(),
                }
            };
            parse_paths(&res, opt_disable_filter, opt_canonical_paths, &hash_file)
        };

        if paths.is_empty() && matches!(exec_mode, ExecMode::Write(_) | ExecMode::Compare) {
            return Err(DanoError::new("No valid paths to search.").into());
        }

        Ok(Config {
            exec_mode,
            opt_num_threads,
            opt_silent,
            opt_write_new,
            opt_overwrite_old,
            hash_algo,
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

    let recorded_file_info = get_recorded_file_info(&config)?;

    if recorded_file_info.is_empty() && !matches!(config.exec_mode, ExecMode::Write(_)) {
        return Err(DanoError::new(
            "Nothing to check or print.  No record of file checksums present.",
        )
        .into());
    }

    let thread_pool = get_thread_pool(&config)?;

    match &config.exec_mode {
        ExecMode::Write(_) | ExecMode::Compare => {
            let opt_requested_paths = Some(&config.paths);
            let file_info_requests = get_file_info_requests(&recorded_file_info, opt_requested_paths)?;
            let rx_item = exec_lookup_file_info(&config,&file_info_requests, thread_pool)?;
            let compare_hashes_bundle =
                exec_process_file_info(&config, &file_info_requests, &recorded_file_info, rx_item)?;

            write_new_file_info(&config, &compare_hashes_bundle)
        }
        ExecMode::Test => {
            let opt_requested_paths = None;
            let file_info_requests = get_file_info_requests(&recorded_file_info, opt_requested_paths)?;
            let rx_item = exec_lookup_file_info(&config, &file_info_requests, thread_pool)?;
            let _ = exec_process_file_info(&config, &file_info_requests, &recorded_file_info, rx_item)?;

            // test will exit on file DNE with special exit code so we don't return here
            unreachable!()
        }
        ExecMode::Print => {
            recorded_file_info.iter().try_for_each(print_file_info)?;

            Ok(())
        }
    }
}

fn get_thread_pool(config: &Config) -> DanoResult<ThreadPool> {
    let num_threads = if let Some(num_threads) = config.opt_num_threads {
        num_threads
    } else {
        num_cpus::get() * 2usize
    };

    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Could not initialize rayon thread pool");

    Ok(thread_pool)
}

fn get_recorded_file_info(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let mut file_info_from_xattrs: Vec<FileInfo> = {
        config
            .paths
            .par_iter()
            .flat_map(|path| xattr::get(path, DANO_XATTR_KEY_NAME).map(|opt| (path, opt)))
            .flat_map(|(path, opt)| opt.map(|s| (path, s)))
            .flat_map(|(path, s)| std::str::from_utf8(&s).map(|i| (path, i.to_owned())))
            .flat_map(|(path, s)| deserialize(&s).map(|i| (path, i)))
            .map(|(path, file_info)| {
                // use the actual path name always
                if path != &file_info.path {
                    FileInfo {
                        version: file_info.version,
                        path: path.to_owned(),
                        metadata: file_info.metadata,
                    }
                } else {
                    file_info
                }
            })
            .collect()
    };

    let file_info_from_file = if config.output_file.exists() {
        let mut input_file = read_input_file(config)?;
        let mut buffer = String::new();
        input_file.read_to_string(&mut buffer)?;
        buffer.par_lines().flat_map(deserialize).collect()
    } else {
        Vec::new()
    };

    // combine
    file_info_from_xattrs.extend(file_info_from_file);
    let mut recorded_file_info: Vec<FileInfo> = file_info_from_xattrs;

    // sort and dedup in case we have paths in hash file and xattrs
    recorded_file_info.sort_by_key(|file_info| file_info.path.clone());
    recorded_file_info.dedup_by_key(|file_info| file_info.path.clone());

    Ok(recorded_file_info)
}

fn get_file_info_requests(
    recorded_file_info: &Vec<FileInfo>,
    opt_requested_paths: Option<&Vec<PathBuf>>,
) -> DanoResult<Vec<FileInfoRequest>> {
    let recorded_file_info_requests: BTreeMap<PathBuf, FileInfoRequest> = recorded_file_info
        .par_iter()
        .map(|file_info| {
            match &file_info.metadata {
                Some(metadata) => (file_info.path.clone(), FileInfoRequest { path: file_info.path.clone(), hash_algo: Some(metadata.hash_algo.clone()) }),
                None => (file_info.path.clone(), FileInfoRequest { path: file_info.path.clone(), hash_algo: None })
            }
        }).collect();
 
    let selected = if let Some(requested_paths) = opt_requested_paths {
        requested_paths
            .iter()
            .map(|path| FileInfoRequest { path: path.clone(), hash_algo: None })
            .map(|request|  {
                match recorded_file_info_requests.get_key_value(&request.path) {
                    Some((_key, value)) => value.to_owned(),
                    None => request,
                }
            })
            .collect()
    } else {
        recorded_file_info_requests.into_values().collect()
    };
    
    Ok(selected)
}
