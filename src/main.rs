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

use std::ops::Deref;

mod config;
mod flac_import;
mod lookup_file_info;
mod output_file_info;
mod prepare_recorded;
mod prepare_requests;
mod process_file_info;
mod utility;
mod versions;

use config::{Config, ExecMode};
use lookup_file_info::FileInfoLookup;
use output_file_info::{write_new, PrintBundle, WriteType};
use prepare_recorded::RecordedFileInfo;
use prepare_requests::{FileInfoRequest, RequestBundle};
use process_file_info::{ProcessedFiles, RemainderBundle, RemainderType};
use utility::{prepare_thread_pool, print_err_buf, print_file_info, DanoError};

pub type DanoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DANO_FILE_INFO_VERSION: usize = 3;
const DANO_XATTR_KEY_NAME: &str = "user.dano.checksum";
const DANO_DEFAULT_HASH_FILE_NAME: &str = "dano_hashes.txt";

const DANO_CLEAN_EXIT_CODE: i32 = 0i32;
const DANO_ERROR_EXIT_CODE: i32 = 1i32;
const DANO_DISORDER_EXIT_CODE: i32 = 2i32;

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

    let recorded_file_info = RecordedFileInfo::new(&config)?;

    let exit_code = match &config.exec_mode {
        ExecMode::Write(write_config)
            if write_config.opt_rewrite || write_config.opt_import_flac =>
        {
            // here we print_file_info because we don't run these opts through verify_file_info,
            // which would ordinary print this information
            recorded_file_info
                .deref()
                .iter()
                .try_for_each(|file_info| print_file_info(&config, file_info))?;

            let processed_files = if write_config.opt_rewrite {
                ProcessedFiles {
                    file_bundle: vec![
                        RemainderBundle {
                            files: Vec::new(),
                            remainder_type: RemainderType::NewFile,
                        },
                        RemainderBundle {
                            files: recorded_file_info.into_inner(),
                            remainder_type: RemainderType::ModifiedFilename,
                        },
                    ],
                    exit_code: DANO_CLEAN_EXIT_CODE,
                }
            } else if write_config.opt_import_flac {
                ProcessedFiles {
                    file_bundle: vec![
                        RemainderBundle {
                            files: recorded_file_info.into_inner(),
                            remainder_type: RemainderType::NewFile,
                        },
                        RemainderBundle {
                            files: Vec::new(),
                            remainder_type: RemainderType::ModifiedFilename,
                        },
                    ],
                    exit_code: DANO_CLEAN_EXIT_CODE,
                }
            } else {
                unreachable!()
            };

            let new_files_bundle: PrintBundle = processed_files.file_bundle.into();
            new_files_bundle.write_out(&config)?;
            processed_files.exit_code
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

            let rx_item = FileInfoLookup::exec(&config, &file_info_requests.into(), thread_pool)?;
            let processed_files = ProcessedFiles::new(&config, &recorded_file_info, rx_item)?;

            let new_files_bundle: PrintBundle = processed_files.file_bundle.into();
            new_files_bundle.write_out(&config)?;
            processed_files.exit_code
        }
        ExecMode::Test(_) => {
            let thread_pool = prepare_thread_pool(&config)?;

            let file_info_requests = RequestBundle::new(&config, &recorded_file_info)?;
            let rx_item = FileInfoLookup::exec(&config, &file_info_requests, thread_pool)?;
            let processed_files = ProcessedFiles::new(&config, &recorded_file_info, rx_item)?;

            let new_files_bundle: PrintBundle = processed_files.file_bundle.into();
            new_files_bundle.write_out(&config)?;

            if !config.is_single_path {
                if processed_files.exit_code == DANO_CLEAN_EXIT_CODE {
                    let _ = print_err_buf("PASSED: File paths are consistent.  Paths contain no hash or filename mismatches.\n");
                } else if processed_files.exit_code == DANO_DISORDER_EXIT_CODE {
                    let _ = print_err_buf("FAILED: File paths are inconsistent.  Some hash or filename mismatch was detected.\n");
                }
            }

            processed_files.exit_code
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
