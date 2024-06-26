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

mod config;
mod flac;
mod ingest;
mod lookup;
mod output;
mod process;
mod requests;
mod utility;
mod versions;

use std::collections::BTreeMap;
use std::path::PathBuf;

use itertools::Itertools;

use crate::lookup::FileInfo;
use config::{Config, ExecMode};
use ingest::RecordedFileInfo;
use lookup::FileInfoLookup;
use output::WriteableFileInfo;
use process::{ProcessedFiles, RemainderBundle};
use requests::{FileInfoRequest, RequestBundle};
use utility::{
    prepare_thread_pool, print_err_buf, print_file_info, remove_dano_xattr, DanoError, DanoResult,
};

const DANO_FILE_INFO_VERSION: usize = 5;
const HEXADECIMAL_RADIX: u32 = 16;
const DANO_XATTR_KEY_NAME: &str = "user.dano.checksum";
const DANO_DEFAULT_HASH_FILE_NAME: &str = "dano_hashes.txt";

const DANO_CLEAN_EXIT_CODE: i32 = 0i32;
const DANO_ERROR_EXIT_CODE: i32 = 1i32;
const DANO_DISORDER_EXIT_CODE: i32 = 2i32;

fn main() {
    let exit_code = match exec() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("ERROR: {}", error);
            DANO_ERROR_EXIT_CODE
        }
    };

    std::process::exit(exit_code)
}

fn exec() -> DanoResult<i32> {
    let config = Config::new()?;

    let recorded_file_info = RecordedFileInfo::new(&config)?;

    let exit_code = match &config.exec_mode {
        ExecMode::Clean => {
            // dano_hashes.txt is removed during recorded_file_info ingest
            let errors: Vec<&PathBuf> = config
                .paths
                .iter()
                .filter(|path| match remove_dano_xattr(path) {
                    Ok(_) => {
                        println!(
                            "dano successfully removed extended attribute from: {:?}",
                            path
                        );
                        false
                    }
                    Err(err) if err.to_string().contains("No data available") => false,
                    Err(err) => {
                        eprintln!("ERROR: {}", err);
                        true
                    }
                })
                .collect();

            if errors.is_empty() {
                println!("All dano extended attributes successfully cleaned.");
                DANO_CLEAN_EXIT_CODE
            } else {
                println!(
                    "ERROR: Could not clean extended attributes form the following paths: {:?}",
                    errors
                );
                DANO_ERROR_EXIT_CODE
            }
        }
        ExecMode::Write(write_config)
            if write_config.opt_rewrite || write_config.opt_import_flac =>
        {
            // here we print_file_info because we don't run these opts through verify_file_info,
            // which would ordinary print this information
            recorded_file_info
                .iter()
                .try_for_each(|file_info| print_file_info(&config, file_info))?;

            let processed_files = if write_config.opt_rewrite {
                ProcessedFiles {
                    new_files: RemainderBundle::NewFile(Vec::new()),
                    modified_file_names: RemainderBundle::ModifiedFilename(
                        recorded_file_info.into_inner(),
                    ),
                    exit_code: DANO_CLEAN_EXIT_CODE,
                }
            } else if write_config.opt_import_flac {
                ProcessedFiles {
                    new_files: RemainderBundle::NewFile(recorded_file_info.into_inner()),
                    modified_file_names: RemainderBundle::ModifiedFilename(Vec::new()),
                    exit_code: DANO_CLEAN_EXIT_CODE,
                }
            } else {
                unreachable!()
            };

            processed_files.write_out(&config)?
        }
        ExecMode::Write(_) => {
            let thread_pool = prepare_thread_pool(&config)?;

            let raw_file_info_requests = RequestBundle::new(&config, &recorded_file_info)?;
            // filter out files for which we already have a hash, only do requests on new files
            let file_info_requests: Vec<FileInfoRequest> = raw_file_info_requests
                .into_inner()
                .into_iter()
                .filter(|request| request.hash_algo.is_none())
                .collect();

            let rx_item = FileInfoLookup::exec(&config, file_info_requests.into(), thread_pool)?;
            let processed_files = ProcessedFiles::new(&config, recorded_file_info, rx_item)?;

            processed_files.write_out(&config)?
        }
        ExecMode::Test(_) => {
            let thread_pool = prepare_thread_pool(&config)?;

            let file_info_requests = RequestBundle::new(&config, &recorded_file_info)?;
            let rx_item = FileInfoLookup::exec(&config, file_info_requests, thread_pool)?;
            let processed_files = ProcessedFiles::new(&config, recorded_file_info, rx_item)?;

            let exit_code = processed_files.write_out(&config)?;

            if !config.is_single_path {
                match exit_code {
                    i if i == DANO_CLEAN_EXIT_CODE => {
                        print_err_buf("PASSED: File paths are consistent.  Paths contain no hash or filename mismatches.\n")?
                    }
                    i if i == DANO_DISORDER_EXIT_CODE => {
                        print_err_buf("FAILED: File paths are inconsistent.  Some hash or filename mismatch was detected.\n")?
                    }
                    _ => {}
                }
            }

            exit_code
        }
        ExecMode::Print => {
            if recorded_file_info.is_empty() {
                return Err(DanoError::new("No recorded file info is available to print.").into());
            }

            recorded_file_info
                .iter()
                .try_for_each(|file_info| print_file_info(&config, file_info))?;

            DANO_CLEAN_EXIT_CODE
        }
        ExecMode::Duplicates => {
            if recorded_file_info.is_empty() {
                return Err(DanoError::new(
                    "No recorded file info is available for duplicate comparison.",
                )
                .into());
            }

            if recorded_file_info.len() == 1 {
                return Err(DanoError::new(
                    "Duplicate comparison requires more than one instance of recorded file info.",
                )
                .into());
            }

            let sorted_group_map: BTreeMap<Box<str>, Vec<FileInfo>> = recorded_file_info
                .into_inner()
                .into_iter()
                .filter(|value| value.metadata.is_some())
                .into_group_map_by(|value| {
                    value.metadata.as_ref().unwrap().hash_value.value.clone()
                })
                .drain()
                .collect();

            let duplicates: Vec<FileInfo> = sorted_group_map
                .into_values()
                .filter(|value| value.len() > 1)
                .flatten()
                .collect();

            if duplicates.is_empty() {
                if !config.opt_silent {
                    eprintln!("No duplicates found.");
                }
                DANO_CLEAN_EXIT_CODE
            } else {
                duplicates
                    .iter()
                    .try_for_each(|file_info| print_file_info(&config, file_info))?;
                if !config.opt_silent {
                    eprintln!("WARN: Duplicates found.");
                }
                DANO_DISORDER_EXIT_CODE
            }
        }
        ExecMode::Dump => {
            if recorded_file_info.is_empty() {
                return Err(
                    DanoError::new("No recorded file info is available to dump to file.").into(),
                );
            }

            if config.output_file.exists() {
                return Err(DanoError::new(
                    "Output file already exists.  Quitting without dumping to file.",
                )
                .into());
            }

            let writable_file_info: WriteableFileInfo = recorded_file_info.into();

            const DUMP_PREFIX: &str = "Dumping dano hash for: ";
            const NOT_DUMP_PREFIX: &str =
                "WARN: Not dumping dano hash for (because dry run was specified): ";

            match writable_file_info.exec(&config, NOT_DUMP_PREFIX, DUMP_PREFIX) {
                Ok(_) if config.opt_dry_run => {
                    print_err_buf("Dry run dump was successful.\n")?;
                    DANO_CLEAN_EXIT_CODE
                }
                Ok(_) => {
                    if !config.opt_silent {
                        print_err_buf("Dump to dano output file was successful.\n")?;
                    }
                    DANO_CLEAN_EXIT_CODE
                }
                Err(err) => {
                    let msg = format!("ERROR: Dump to dano output file was unsuccessful for the following reason: {:?}\n", err);
                    print_err_buf(&msg)?;
                    DANO_ERROR_EXIT_CODE
                }
            }
        }
    };

    Ok(exit_code)
}
