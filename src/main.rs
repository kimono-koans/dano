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
use serde::{Deserialize, Serialize};

mod flac_import;
mod lookup_file_info;
mod output_file_info;
mod prepare_recorded;
mod prepare_requests;
mod process_file_info;
mod utility;
mod versions;

use lookup_file_info::exec_lookup_file_info;
use output_file_info::{write_file_info_bundle, write_new, WriteType};
use prepare_recorded::get_recorded_file_info;
use prepare_requests::get_file_info_requests;
use process_file_info::{process_file_info_exec, RemainderFilesBundle, RemainderType};
use utility::{print_err_buf, print_file_info, read_stdin, DanoError};

use crate::process_file_info::ProcessingRemainder;

pub type DanoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DANO_FILE_INFO_VERSION: usize = 3;
const DANO_XATTR_KEY_NAME: &str = "user.dano.checksum";
const DANO_DEFAULT_HASH_FILE_NAME: &str = "dano_hashes.txt";

const DANO_CLEAN_EXIT_CODE: i32 = 0i32;
const DANO_ERROR_EXIT_CODE: i32 = 1i32;
const DANO_DISORDER_EXIT_CODE: i32 = 2i32;

fn parse_args() -> ArgMatches {
    clap::Command::new(crate_name!())
        .about("dano is a wrapper for ffmpeg that checksums the internal file streams of certain media files, \
        and stores them in a format which can be used to verify such checksums later.  This is handy, because, \
        should you choose to change metadata tags, or change file names, the media checksums should remain the same.")
        .version(crate_version!())
        .arg(
            Arg::new("INPUT_FILES")
                .help("input files to be hashed or verified.  INPUT_FILES can also be read from stdin for NULL or NEWLINE delimited inputs.")
                .takes_value(true)
                .multiple_values(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(1),
        )
        .arg(
            Arg::new("OUTPUT_FILE")
                .help("output file which will contain the recorded file information. If not specified, 'dano_hashes.txt' in the current working directory will be used.")
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
                .help("file from which to read recorded file information.  If not specified, the output file will be used (or if not specified 'dano_hashes.txt' in the current working directory).")
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
                .help("write the new input files' hash information (and ignore files that already have file hashes).")
                .short('w')
                .long("write")
                .display_order(4))
        .arg(
            Arg::new("TEST")
                .help("verify the recorded file information.  Prints the pass/fail status, exits with a non-zero code if failed, and, potentially, performs write operations, like --write-new or --overwrite.")
                .short('t')
                .long("test")
                .alias("compare")
                .short_alias('c')
                .display_order(5))
        .arg(
            Arg::new("PRINT")
                .help("pretty print all recorded file information (in hash file and xattrs).")
                .short('p')
                .long("print")
                .display_order(6))
        .arg(
            Arg::new("DUMP")
                .help("dump the recorded file information (in hash file and xattrs) to the output file (don't test/compare).")
                .long("dump")
                .display_order(7))
        .arg(
            Arg::new("IMPORT_FLAC")
                .help("import flac checksums and write as dano recorded file information.")
                .long("import-flac")
                .conflicts_with_all(&["TEST", "PRINT", "DUMP"])
                .display_order(8))
        .arg(
            Arg::new("NUM_THREADS")
                .help("requested number of threads to use for file processing.  Default is twice the number of logical cores.")
                .short('j')
                .long("threads")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(9))
        .arg(
            Arg::new("SILENT")
                .help("quiet many informational messages (like \"OK\").")
                .short('s')
                .long("silent")
                .display_order(10),
        )
        .arg(
            Arg::new("WRITE_NEW")
                .help("if new files are present in TEST mode, append such file info.")
                .long("write-new")
                .requires("TEST")
                .display_order(11),
        )
        .arg(
            Arg::new("OVERWRITE_OLD")
                .help("if one file's hash matches another's, but they have different file name's, overwrite the old file's info with the most current.")
                .long("overwrite")
                .conflicts_with_all(&["PRINT", "DUMP"])
                .display_order(12),
        )
        .arg(
            Arg::new("DISABLE_FILTER")
                .help("by default, file extensions not recognized by ffmpeg are filtered.  Here, you may disable such filtering.")
                .long("disable-filter")
                .display_order(13),
        )
        .arg(
            Arg::new("CANONICAL_PATHS")
                .help("use canonical paths (instead of potentially relative paths).")
                .long("canonical-paths")
                .display_order(14),
        )
        .arg(
            Arg::new("XATTR")
                .help("try to write (dano will always try to read) hash to/from file's extended attributes.  \
                Can also be enabled by setting environment variable DANO_XATTR_WRITES to any value.  \
                When XATTR is enabled, if a write is requested, dano will always overwrite extended attributes previously written.")
                .short('x')
                .long("xattr")
                .display_order(15),
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
                .display_order(16))
        .arg(
            Arg::new("DECODE")
                .help("decode stream before hashing.  Much slower, but potentially useful for lossless formats.")
                .long("decode")
                .conflicts_with_all(&["PRINT", "DUMP"])
                .display_order(17))
        .arg(
            Arg::new("REWRITE_ALL")
                .help("rewrite all recorded hashes to the latest and greatest format version.  dano will ignore input files without recorded hashes.")
                .long("rewrite")
                .requires("WRITE")
                .display_order(18))
        .arg(
            Arg::new("ONLY")
                .help("hash selected stream only")
                .long("only")
                .takes_value(true)
                .require_equals(true)
                .possible_values(&["audio", "video"])
                .value_parser(clap::builder::ValueParser::os_string())
                .requires("WRITE")
                .display_order(19))
        .arg(
            Arg::new("DRY_RUN")
            .help("print the information to stdout that would be written to disk.")
            .long("dry-run")
            .conflicts_with_all(&["PRINT", "DUMP"])
            .display_order(20))
        .get_matches()
}

#[derive(Debug, Clone)]
pub struct FileInfoRequest {
    pub path: PathBuf,
    pub hash_algo: Option<Box<str>>,
    pub decoded: Option<bool>,
    pub selected_streams: Option<SelectedStreams>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WriteModeConfig {
    opt_rewrite: bool,
    opt_import_flac: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestModeConfig {
    opt_write_new: bool,
    opt_overwrite_old: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecMode {
    Test(TestModeConfig),
    Write(WriteModeConfig),
    Print,
    Dump,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum SelectedStreams {
    All,
    AudioOnly,
    VideoOnly,
}

#[derive(Debug, Clone)]
pub struct Config {
    exec_mode: ExecMode,
    opt_silent: bool,
    opt_decode: bool,
    opt_xattr: bool,
    opt_dry_run: bool,
    is_single_path: bool,
    opt_num_threads: Option<usize>,
    selected_streams: SelectedStreams,
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
        let opt_decode = matches.is_present("DECODE");
        let opt_import_flac = matches.is_present("IMPORT_FLAC");
        let opt_rewrite = matches.is_present("REWRITE_ALL");

        let exec_mode = if matches.is_present("TEST") {
            ExecMode::Test(TestModeConfig {
                opt_overwrite_old,
                opt_write_new,
            })
        } else if matches.is_present("WRITE") || opt_rewrite || opt_import_flac {
            ExecMode::Write(WriteModeConfig {
                opt_rewrite,
                opt_import_flac,
            })
        } else if matches.is_present("DUMP") {
            ExecMode::Dump
        } else if matches.is_present("PRINT") {
            ExecMode::Print
        } else {
            return Err(DanoError::new(
                "You must specify an execution mode: TEST, WRITE, PRINT or DUMP",
            )
            .into());
        };

        let selected_streams = if let Some(only_stream) = matches.value_of_os("ONLY") {
            if only_stream == OsStr::new("video") {
                SelectedStreams::VideoOnly
            } else if only_stream == OsStr::new("audio") {
                SelectedStreams::AudioOnly
            } else {
                SelectedStreams::All
            }
        } else {
            SelectedStreams::All
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

        let paths: Vec<PathBuf> = {
            let res: Vec<PathBuf> = if let Some(input_files) = matches.values_of_os("INPUT_FILES") {
                input_files.par_bridge().map(PathBuf::from).collect()
            } else {
                match &exec_mode {
                    ExecMode::Test(_) if hash_file.exists() => Vec::new(),
                    _ => read_stdin()?.par_iter().map(PathBuf::from).collect(),
                }
            };
            parse_paths(&res, opt_disable_filter, opt_canonical_paths, &hash_file)
        };

        if paths.is_empty() && !matches!(exec_mode, ExecMode::Test(_)) {
            return Err(DanoError::new("No valid paths to search.").into());
        }

        Ok(Config {
            exec_mode,
            opt_silent,
            opt_num_threads,
            opt_decode,
            opt_xattr,
            opt_dry_run,
            is_single_path: { paths.len() <= 1 },
            selected_streams,
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
                eprintln!("Error: Path does not exist: {:?}", path);
                false
            }
        })
        .filter(|path| match path.to_str() {
            Some(_) => true,
            None => {
                eprintln!("Error: Path cannot be serialized to string: {:?}", path);
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
    let exit_code = match exec() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("Error: {}", error);
            DANO_ERROR_EXIT_CODE
        }
    };

    std::process::exit(exit_code)
}

fn exec() -> DanoResult<i32> {
    let config = Config::new()?;

    let recorded_file_info = get_recorded_file_info(&config)?;

    let exit_code = match &config.exec_mode {
        ExecMode::Write(write_config)
            if write_config.opt_rewrite || write_config.opt_import_flac =>
        {
            // here we print_file_info because we don't run these opts through verify_file_info,
            // which would ordinary print this information
            recorded_file_info
                .iter()
                .try_for_each(|file_info| print_file_info(&config, file_info))?;

            let processing_remainder = if write_config.opt_rewrite {
                ProcessingRemainder {
                    file_bundle: vec![
                        RemainderFilesBundle {
                            files: Vec::new(),
                            remainder_type: RemainderType::NewFile,
                        },
                        RemainderFilesBundle {
                            files: recorded_file_info,
                            remainder_type: RemainderType::ModifiedFilename,
                        },
                    ],
                    exit_code: DANO_CLEAN_EXIT_CODE,
                }
            } else if write_config.opt_import_flac {
                ProcessingRemainder {
                    file_bundle: vec![
                        RemainderFilesBundle {
                            files: recorded_file_info,
                            remainder_type: RemainderType::NewFile,
                        },
                        RemainderFilesBundle {
                            files: Vec::new(),
                            remainder_type: RemainderType::ModifiedFilename,
                        },
                    ],
                    exit_code: DANO_CLEAN_EXIT_CODE,
                }
            } else {
                unreachable!()
            };

            write_file_info_bundle(&config, &processing_remainder.file_bundle)?;
            processing_remainder.exit_code
        }
        ExecMode::Write(_) => {
            let thread_pool = prepare_thread_pool(&config)?;

            let raw_file_info_requests = get_file_info_requests(&config, &recorded_file_info)?;
            // filter out files for which we already have a hash, only do requests on new files
            let file_info_requests: Vec<FileInfoRequest> = raw_file_info_requests
                .into_iter()
                .filter(|request| request.hash_algo.is_none())
                .collect();

            let rx_item = exec_lookup_file_info(&config, &file_info_requests, thread_pool)?;
            let processed_res = process_file_info_exec(&config, &recorded_file_info, rx_item)?;

            write_file_info_bundle(&config, &processed_res.file_bundle)?;
            processed_res.exit_code
        }
        ExecMode::Test(_) => {
            let thread_pool = prepare_thread_pool(&config)?;

            let file_info_requests = get_file_info_requests(&config, &recorded_file_info)?;
            let rx_item = exec_lookup_file_info(&config, &file_info_requests, thread_pool)?;
            let processed_res = process_file_info_exec(&config, &recorded_file_info, rx_item)?;

            write_file_info_bundle(&config, &processed_res.file_bundle)?;

            if !config.is_single_path {
                if processed_res.exit_code == DANO_CLEAN_EXIT_CODE {
                    let _ = print_err_buf("PASSED: File paths are consistent.  Paths contain no hash or filename mismatches.\n");
                } else if processed_res.exit_code == DANO_DISORDER_EXIT_CODE {
                    let _ = print_err_buf("FAILED: File paths are inconsistent.  Some hash or filename mismatch was detected.\n");
                }
            }

            processed_res.exit_code
        }
        ExecMode::Print => {
            if recorded_file_info.is_empty() {
                return Err(DanoError::new("No recorded file info is available to print.").into());
            } else {
                recorded_file_info
                    .iter()
                    .try_for_each(|file_info| print_file_info(&config, file_info))?;
            }

            DANO_CLEAN_EXIT_CODE
        }
        ExecMode::Dump => {
            if recorded_file_info.is_empty() {
                return Err(
                    DanoError::new("No recorded file info is available to dump to file.").into(),
                );
            } else if config.output_file.exists() {
                return Err(DanoError::new(
                    "Output file already exists.  Quitting without dumping to file.",
                )
                .into());
            } else {
                write_new(
                    &config,
                    recorded_file_info.as_slice(),
                    WriteType::OverwriteAll,
                )?;
                if !config.opt_silent {
                    print_err_buf("Dump to dano output file was successful.\n")?
                }
            }

            DANO_CLEAN_EXIT_CODE
        }
    };

    Ok(exit_code)
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
