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
    path::{Path, PathBuf},
};

use clap::{crate_name, crate_version, Arg, ArgMatches};
use rayon::{prelude::*, ThreadPool};

mod lookup_file_info;
mod output_file_info;
mod prepare_recorded;
mod prepare_requests;
mod process_file_info;
mod utility;
mod versions;

use lookup_file_info::exec_lookup_file_info;
use output_file_info::write_file_info_exec;
use prepare_recorded::get_recorded_file_info;
use prepare_requests::get_file_info_requests;
use process_file_info::{process_file_info_exec, NewFilesBundle};
use utility::{print_file_info, read_stdin, DanoError};

pub type DanoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DANO_FILE_INFO_VERSION: usize = 2;
const DANO_XATTR_KEY_NAME: &str = "user.dano.checksum";
const DANO_DEFAULT_HASH_FILE_NAME: &str = "dano_hashes.txt";

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
                .short('o')
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
            Arg::new("COMPARE")
                .help("compare the input files to the hashes in a hash file (or in a xattr).")
                .short('c')
                .long("compare")
                .display_order(6),
            )
        .arg(
            Arg::new("TEST")
                .help("compare the hashes in a hash file to the files currently on disk.")
                .short('t')
                .long("test")
                .conflicts_with("INPUT_FILES")
                .display_order(5))
        .arg(
            Arg::new("PRINT")
                .help("pretty print the file hashes.")
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
                .possible_values(&["murmur3", "md5", "crc32", "adler32", "sha1", "sha160", "sha256", "sha384", "sha512"])
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(13))
        .arg(
            Arg::new("DECODE")
                .help("decode stream before hashing.  Much slower, but potentially useful for lossless formats.")
                .long("decode")
                .display_order(14))
        .arg(
            Arg::new("REWRITE_ALL")
                .help("rewrite all recorded hashes to the latest and greatest format version.  dano will ignore input files without recorded hashes.")
                .long("rewrite")
                .requires("WRITE")
                .display_order(15))
        .arg(
            Arg::new("DRY_RUN")
            .help("print the information to stdout that would be written to disk.")
            .long("dry-run")
            .requires("WRITE")
            .display_order(16))
        .get_matches()
}

#[derive(Debug, Clone)]
pub struct FileInfoRequest {
    pub path: PathBuf,
    pub hash_algo: Option<Box<str>>,
    pub decoded: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WriteModeConfig {
    opt_xattr: bool,
    opt_dry_run: bool,
    opt_rewrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompareModeConfig {
    opt_test_mode: bool,
    opt_write_new: bool,
    opt_overwrite_old: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecMode {
    Compare(CompareModeConfig),
    Write(WriteModeConfig),
    Print,
}

#[derive(Debug, Clone)]
pub struct Config {
    exec_mode: ExecMode,
    opt_silent: bool,
    opt_decode: bool,
    opt_num_threads: Option<usize>,
    selected_hash_algo: Box<str>,
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
                "Working directory does not exist or you do not have permissions to access it.",
            )
            .into());
        };

        let opt_xattr =
            matches.is_present("XATTR") || std::env::var_os("DANO_XATTR_WRITES").is_some();
        let opt_dry_run = matches.is_present("DRY_RUN")
            || (matches.is_present("PRINT") && matches.is_present("WRITE"));
        let opt_num_threads = matches
            .value_of_lossy("NUM_THREADS")
            .and_then(|num_threads_str| num_threads_str.parse::<usize>().ok());
        let opt_write_new = matches.is_present("WRITE_NEW");
        let opt_silent = matches.is_present("SILENT");
        let opt_overwrite_old = matches.is_present("OVERWRITE_OLD");
        let opt_disable_filter = matches.is_present("DISABLE_FILTER");
        let opt_canonical_paths = matches.is_present("CANONICAL_PATHS");
        let opt_test_mode = matches.is_present("TEST");
        let opt_decode = matches.is_present("DECODE");
        let opt_rewrite = matches.is_present("REWRITE_ALL");

        let exec_mode = if matches.is_present("COMPARE") || opt_test_mode {
            ExecMode::Compare(CompareModeConfig {
                opt_test_mode,
                opt_overwrite_old,
                opt_write_new,
            })
        } else if matches.is_present("PRINT") && !matches.is_present("WRITE") {
            ExecMode::Print
        } else if matches.is_present("WRITE") {
            ExecMode::Write(WriteModeConfig {
                opt_xattr,
                opt_dry_run,
                opt_rewrite,
            })
        } else {
            return Err(DanoError::new(
                "You must specify an execution mode: COMPARE, TEST, WRITE, or PRINT",
            )
            .into());
        };

        let output_file = if let Some(output_file) = matches.value_of_os("OUTPUT_FILE") {
            PathBuf::from(output_file)
        } else {
            pwd.join(DANO_DEFAULT_HASH_FILE_NAME)
        };

        let selected_hash_algo = if let Some(hash_algo) = matches.value_of_os("HASH_ALGO") {
            if hash_algo == OsStr::new("sha1") {
                "sha160".into()
            } else {
                hash_algo.to_string_lossy().into()
            }
        } else {
            "murmur3".into()
        };

        let hash_file = if let Some(hash_file) = matches.value_of_os("HASH_FILE") {
            PathBuf::from(hash_file)
        } else {
            output_file.clone()
        };

        if !hash_file.exists() && opt_test_mode {
            return Err(DanoError::new("Test mode requires the user specify a hash file.").into());
        }

        let paths: Vec<PathBuf> = {
            let res: Vec<PathBuf> = if let Some(input_files) = matches.values_of_os("INPUT_FILES") {
                input_files.par_bridge().map(PathBuf::from).collect()
            } else {
                match &exec_mode {
                    ExecMode::Compare(compare_config) if compare_config.opt_test_mode => Vec::new(),
                    _ => read_stdin()?.par_iter().map(PathBuf::from).collect(),
                }
            };
            parse_paths(&res, opt_disable_filter, opt_canonical_paths, &hash_file)
        };

        if paths.is_empty() && (matches!(exec_mode, ExecMode::Write(_)) || !opt_test_mode) {
            return Err(DanoError::new("No valid paths to search.").into());
        }

        Ok(Config {
            exec_mode,
            opt_silent,
            opt_num_threads,
            opt_decode,
            selected_hash_algo,
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

    let thread_pool = prepare_thread_pool(&config)?;

    match &config.exec_mode {
        ExecMode::Write(write_config) => {
            let file_bundle = if write_config.opt_rewrite {
                NewFilesBundle {
                    new_files: Vec::new(),
                    new_filenames: recorded_file_info,
                }
            } else {
                let raw_file_info_requests = get_file_info_requests(&config, &recorded_file_info)?;

                // filter out files for which we already have a hash, only do requests on new files
                let file_info_requests: Vec<FileInfoRequest> = raw_file_info_requests
                    .into_iter()
                    .filter(|request| request.hash_algo.is_none())
                    .collect();

                let rx_item = exec_lookup_file_info(&config, &file_info_requests, thread_pool)?;
                let compare_hashes_bundle =
                    process_file_info_exec(&config, &recorded_file_info, rx_item)?;

                compare_hashes_bundle
            };

            write_file_info_exec(&config, &file_bundle)
        }
        ExecMode::Compare(_) => {
            let file_info_requests = get_file_info_requests(&config, &recorded_file_info)?;
            let rx_item = exec_lookup_file_info(&config, &file_info_requests, thread_pool)?;
            let compare_hashes_bundle =
                process_file_info_exec(&config, &recorded_file_info, rx_item)?;

            write_file_info_exec(&config, &compare_hashes_bundle)
        }
        ExecMode::Print => {
            if recorded_file_info.is_empty() {
                return Err(DanoError::new("No recorded file info is available to print.").into());
            } else {
                recorded_file_info
                    .iter()
                    .try_for_each(|file_info| print_file_info(&config, file_info))?;
            }

            Ok(())
        }
    }
}

fn prepare_thread_pool(config: &Config) -> DanoResult<ThreadPool> {
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
