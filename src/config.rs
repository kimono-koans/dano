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
    borrow::Cow,
    collections::HashSet,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use clap::{crate_name, crate_version, Arg, ArgMatches};
use itertools::Either;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::utility::read_stdin;
use crate::{DanoError, DanoResult, DANO_DEFAULT_HASH_FILE_NAME};

fn parse_args() -> ArgMatches {
    clap::Command::new(crate_name!())
        .about("dano is a wrapper for ffmpeg that checksums the internal bitstreams of held within certain media files/containers, \
        and stores them in a format which can be used to verify such checksums later.  This is handy, because, \
        should you choose to change metadata tags, or change file names, the media checksums should remain the same.")
        .version(crate_version!())
        .arg(
            Arg::new("INPUT_FILES")
                .help("select the input files to be hashed or verified, etc.  INPUT_FILES can also be read from stdin for NULL or NEWLINE delimited inputs.  \
                By default, files which don't appear to be valid extensions for ffmpeg are filtered with a WARN message, unless the SILENT flag is enabled.  \
                Hidden files (so-called dot files), files with no name, or no extension are silently ignored.  The default behavior can be disabled with the DISABLE_FILTER flag.")
                .takes_value(true)
                .multiple_values(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(1),
        )
        .arg(
            Arg::new("OUTPUT_FILE")
                .help("select the output file to record the file information. If not specified, 'dano_hashes.txt' in the current working directory will be used.")
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
                .help("select the file from which to read recorded file information.  If not specified, the output file will be used (or if not specified, 'dano_hashes.txt' in the current working directory will be used).")
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
                .help("write the new input files' hash information.  If no other flags are specified, dano will ignore files which already have file hashes.")
                .short('w')
                .long("write")
                .display_order(4))
        .arg(
            Arg::new("TEST")
                .help("verify the recorded file information.  Prints the pass/fail status, and exits with a non-zero code, if failed, and can, potentially, performs write operations when specified, by the WRITE_NEW and OVERWRITE_OLD flags.")
                .short('t')
                .long("test")
                .alias("compare")
                .short_alias('c')
                .display_order(5))
        .arg(
            Arg::new("PRINT")
                .help("pretty print all recorded file information (discovered within both the hash file and any xattrs).")
                .short('p')
                .long("print")
                .display_order(6))
        .arg(
            Arg::new("DUMP")
                .help("dump the recorded file information (in hash file and xattrs) to the output file (don't test/compare).")
                .long("dump")
                .display_order(7))
        .arg(
            Arg::new("DUPLICATES")
                .help("show any hash value duplicates discovered when reading back recorded file information (in hash file and xattrs).")
                .long("duplicates")
                .aliases(&["dupes"])
                .display_order(8))
        .arg(
            Arg::new("IMPORT_FLAC")
                .help("import flac checksums and write such information as dano recorded file information.")
                .long("import-flac")
                .conflicts_with_all(&["TEST", "PRINT", "DUMP", "DUPLICATES"])
                .display_order(9))
        .arg(
            Arg::new("NUM_THREADS")
                .help("requested number of threads to use for file processing.  Default is the number of logical cores.")
                .short('j')
                .long("threads")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(10))
        .arg(
            Arg::new("SILENT")
                .help("quiet many informational messages (such as \"OK\").")
                .short('s')
                .long("silent")
                .display_order(11),
        )
        .arg(
            Arg::new("WRITE_NEW")
                .help("in TEST mode, if new files are present, write new file info.")
                .long("write-new")
                .requires("TEST")
                .conflicts_with_all(&["PRINT", "DUMP", "DUPLICATES", "WRITE"])
                .display_order(12),
        )
        .arg(
            Arg::new("OVERWRITE_OLD")
                .help("in TEST mode, if a file's hash matches a recorded hash, but that file now has a different file name, \
                overwrite the old file's recorded file info with the most current. OVERWRITE_OLD implies WRITE_NEW.")
                .long("overwrite")
                .requires("TEST")
                .conflicts_with_all(&["PRINT", "DUMP", "DUPLICATES", "WRITE"])
                .display_order(13),
        )
        .arg(
            Arg::new("DISABLE_FILTER")
                .help("disable the default filtering of file extensions which ffmpeg lists as \"common\" extensions for supported file formats.")
                .long("disable-filter")
                .display_order(14),
        )
        .arg(
            Arg::new("CANONICAL_PATHS")
                .help("use canonical paths (paths from the root directory) instead of potentially relative paths.")
                .long("canonical-paths")
                .display_order(15),
        )
        .arg(
            Arg::new("XATTR")
                .help("try to write (dano will always try to read) hash to any input file's extended attributes.  \
                Can also be enabled by setting environment variable DANO_XATTR_WRITES to any value (such as: export DANO_XATTR_WRITES=enabled).  \
                When XATTR is enabled, if a write is requested, dano will always overwrite extended attributes previously written.")
                .short('x')
                .long("xattr")
                .display_order(16),
        )
        .arg(
            Arg::new("HASH_ALGO")
                .help("specify the algorithm to use for hashing.  Default is 'murmur3'.")
                .long("hash-algo")
                .takes_value(true)
                .min_values(1)
                .require_equals(true)
                .possible_values(["murmur3", "md5", "crc32", "adler32", "sha1", "sha160", "sha256", "sha384", "sha512"])
                .value_parser(clap::builder::ValueParser::os_string())
                .display_order(17))
        .arg(
            Arg::new("DECODE")
                .help("decode internal bitstream before hashing.  This option makes testing and writes much slower, but this option is potentially useful for lossless formats.")
                .long("decode")
                .conflicts_with_all(&["PRINT", "DUMP", "DUPLICATES"])
                .display_order(18))
        .arg(
            Arg::new("REWRITE_ALL")
                .help("rewrite all recorded hashes to the latest and greatest format version.  \
                When specified, dano will silently ignore any input files without recorded hashes.")
                .long("rewrite")
                .requires("WRITE")
                .conflicts_with_all(&["PRINT", "DUMP", "DUPLICATES", "TEST"])
                .display_order(19))
        .arg(
            Arg::new("ONLY")
                .help("hash the an input file container's first audio or video stream only, if available.  \
                dano will fall back to default behavior, if no stream is available.")
                .long("only")
                .takes_value(true)
                .require_equals(true)
                .possible_values(["audio", "video"])
                .value_parser(clap::builder::ValueParser::os_string())
                .requires("WRITE")
                .display_order(20))
        .arg(
            Arg::new("DRY_RUN")
            .help("print the information to stdout that would be written to disk.")
            .long("dry-run")
            .conflicts_with_all(&["PRINT", "DUMP", "DUPLICATES"])
            .display_order(21))
        .get_matches()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteModeConfig {
    pub opt_rewrite: bool,
    pub opt_import_flac: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOpt {
    WriteNew,
    OverwriteAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecMode {
    Test(Option<WriteOpt>),
    Write(WriteModeConfig),
    Print,
    Dump,
    Duplicates,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum SelectedStreams {
    All,
    AudioOnly,
    VideoOnly,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub exec_mode: ExecMode,
    pub opt_silent: bool,
    pub opt_decode: bool,
    pub opt_xattr: bool,
    pub opt_dry_run: bool,
    pub is_single_path: bool,
    pub opt_num_threads: Option<usize>,
    pub selected_streams: SelectedStreams,
    pub selected_hash_algo: Box<str>,
    pub pwd: PathBuf,
    pub output_file: PathBuf,
    pub hash_file: PathBuf,
    pub paths: Vec<PathBuf>,
}

impl Config {
    pub fn new() -> DanoResult<Self> {
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
        let opt_silent = matches.is_present("SILENT");
        let opt_disable_filter = matches.is_present("DISABLE_FILTER");
        let opt_canonical_paths = matches.is_present("CANONICAL_PATHS");
        let opt_decode = matches.is_present("DECODE");
        let opt_import_flac = matches.is_present("IMPORT_FLAC");
        let opt_rewrite = matches.is_present("REWRITE_ALL");

        let exec_mode = if matches.is_present("TEST") {
            let opt_test_write_opt = if matches.is_present("OVERWRITE_OLD") {
                Some(WriteOpt::OverwriteAll)
            } else if matches.is_present("WRITE_NEW") {
                Some(WriteOpt::WriteNew)
            } else {
                None
            };

            ExecMode::Test(opt_test_write_opt)
        } else if matches.is_present("WRITE") || opt_rewrite || opt_import_flac {
            ExecMode::Write(WriteModeConfig {
                opt_rewrite,
                opt_import_flac,
            })
        } else if matches.is_present("DUMP") {
            ExecMode::Dump
        } else if matches.is_present("PRINT") {
            ExecMode::Print
        } else if matches.is_present("DUPLICATES") {
            ExecMode::Duplicates
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
                    _ => read_stdin()?,
                }
            };
            Self::parse_paths(
                &res,
                opt_disable_filter,
                opt_canonical_paths,
                opt_silent,
                &hash_file,
            )
        };

        if paths.is_empty() {
            return Err(DanoError::new("No valid paths given.  Exiting.").into());
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

    fn parse_paths(
        raw_paths: &[PathBuf],
        opt_disable_filter: bool,
        opt_canonical_paths: bool,
        opt_silent: bool,
        hash_file: &Path,
    ) -> Vec<PathBuf> {
        let auto_extension_filter = include_str!("../data/ffmpeg_extensions_list.txt");

        let (bad_extensions, valid_paths): (Vec<_>, Vec<_>) = raw_paths
            .into_par_iter()
            .filter(|path| {
                if path.exists() {
                    return true;
                }

                eprintln!("ERROR: Path does not exist: {:?}", path);
                false
            })
            .filter(|path| {
                if path.is_file() {
                    return true;
                }

                eprintln!("ERROR: Path is not a regular file: {:?}", path);
                false
            })
            .filter(|path| {
                if path.to_str().is_some() {
                    return true;
                }

                eprintln!("ERROR: Path cannot be serialized to string: {:?}", path);
                false
            })
            .filter(|path| {
                if path.file_name() == Some(OsStr::new(hash_file)) {
                    eprintln!(
                        "ERROR: File name is the name of a dano hash file: {:?}",
                        path
                    );
                    return false;
                }

                true
            })
            .filter_map(|path| {
                if !opt_disable_filter {
                    let opt_extension = path.extension();

                    if auto_extension_filter
                        .lines()
                        .any(|extension| opt_extension == Some(OsStr::new(extension)))
                    {
                        return Some(Either::Right(path.as_path()));
                    }

                    if let Some(ext) = opt_extension {
                        return Some(Either::Left(ext.to_string_lossy()));
                    }

                    // what are these None cases: hidden files (dot files),
                    // no file name, no extension
                    return None;
                }

                Some(Either::Right(path.as_path()))
            })
            .partition_map(|item| item);

        if !opt_silent && !bad_extensions.is_empty() {
            let unique: HashSet<Cow<str>> = bad_extensions.into_iter().collect();

            let buffer: String = unique.iter().map(|ext| format!("{} ", ext)).collect();

            eprintln!("WARN: The following are extensions which are unknown to dano: {:?}.  dano has excluded all files with these extensions.  If you know these file types are acceptable to ffmpeg, you may use --disable-filter to force dano to accept their use.", buffer.trim());
        }

        valid_paths
            .iter()
            .map(|path| {
                if opt_canonical_paths {
                    if let Ok(canonical) = path.canonicalize() {
                        return canonical;
                    }

                    eprintln!(
                        "WARN: Unable convert relative path to canonical path: {:?}",
                        path
                    );
                }

                path.to_path_buf()
            })
            .collect()
    }
}
