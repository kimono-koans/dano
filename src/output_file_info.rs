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

use std::time::SystemTime;

use itertools::Itertools;
use rug::Integer;

use crate::{Config, DanoResult, ExecMode};

use crate::lookup_file_info::FileInfo;
use crate::process_file_info::NewFilesBundle;
use crate::util::{
    print_err_buf, read_file_info_from_file, write_all_new_paths, DanoError, WriteType,
};

const WRITE_NEW_PREFIX: &str = "Writing dano hash for: ";
const EMPTY_STR: &str = "";
const OVERWRITE_OLD_PREFIX: &str = "Overwriting dano hash for: ";

const NOT_WRITE_NEW_PREFIX: &str = "Not writing dano hash for: ";
const NOT_WRITE_NEW_SUFFIX: &str = ", --write-new was not specified.";

const NOT_OVERWRITE_OLD_PREFIX: &str = "Not overwriting dano hash for: {";
const NOT_OVERWRITE_OLD_SUFFIX: &str = ", --overwrite was not specified.";

pub fn write_file_info_exec(config: &Config, new_files_bundle: &NewFilesBundle) -> DanoResult<()> {
    write_new_files(config, new_files_bundle)?;
    write_new_filenames(config, new_files_bundle)?;

    Ok(())
}

fn write_new_files(config: &Config, new_files_bundle: &NewFilesBundle) -> DanoResult<()> {
    // write new files - no hash match in record
    if !new_files_bundle.new_files.is_empty() {
        let write_new = || -> DanoResult<()> {
            print_write_action(WRITE_NEW_PREFIX, EMPTY_STR, &new_files_bundle.new_files)?;
            write_all_new_paths(config, &new_files_bundle.new_files, WriteType::Append)
        };

        match &config.exec_mode {
            ExecMode::Write(_) => write_new()?,
            ExecMode::Compare(compare_config) => {
                if compare_config.opt_write_new {
                    write_new()?
                } else {
                    print_write_action(
                        NOT_WRITE_NEW_PREFIX,
                        NOT_WRITE_NEW_SUFFIX,
                        &new_files_bundle.new_files,
                    )?;
                }
            }
            _ => unreachable!(),
        }
    } else if let ExecMode::Compare(compare_config) = &config.exec_mode {
        if compare_config.opt_write_new {
            eprintln!("No new file paths to write.");
        } else {
            eprintln!("No new file paths to write, and --write-new was not specified");
        }
    } else {
        eprintln!("No new file paths to write.");
    }

    Ok(())
}

fn write_new_filenames(config: &Config, new_files_bundle: &NewFilesBundle) -> DanoResult<()> {
    // write old files with new names - hash matches
    if !new_files_bundle.new_filenames.is_empty() {
        let overwrite_old = || -> DanoResult<()> {
            print_write_action(
                OVERWRITE_OLD_PREFIX,
                EMPTY_STR,
                &new_files_bundle.new_filenames,
            )?;
            overwrite_old_file_info(config, new_files_bundle)
        };

        match &config.exec_mode {
            ExecMode::Write(_) => overwrite_old()?,
            ExecMode::Compare(compare_config) => {
                if compare_config.opt_overwrite_old {
                    overwrite_old()?
                } else {
                    print_write_action(
                        NOT_OVERWRITE_OLD_PREFIX,
                        NOT_OVERWRITE_OLD_SUFFIX,
                        &new_files_bundle.new_filenames,
                    )?;
                }
            }
            _ => unreachable!(),
        }
    } else if let ExecMode::Compare(compare_config) = &config.exec_mode {
        if compare_config.opt_overwrite_old {
            eprintln!("No old file hashes to overwrite.");
        } else {
            eprintln!("No old file hash to overwrite, and --overwrite was not specified.");
        }
    } else {
        eprintln!("No old file hashes to overwrite.");
    }

    Ok(())
}

fn print_write_action(prefix: &str, suffix: &str, file_bundle: &[FileInfo]) -> DanoResult<()> {
    file_bundle.iter().try_for_each(|file_info| {
        print_err_buf(&format!("{}{:?}{}\n", prefix, file_info.path, suffix))
    })
}

pub fn overwrite_old_file_info(
    config: &Config,
    new_files_bundle: &NewFilesBundle,
) -> DanoResult<()> {
    // append new paths
    write_all_new_paths(config, &new_files_bundle.new_filenames, WriteType::Append)?;

    // overwrite all paths if in non-xattr/file write mode
    match &config.exec_mode {
        ExecMode::Write(write_config) if !write_config.opt_xattr => {
            // read back
            let recorded_file_info_with_duplicates: Vec<FileInfo> = if config.output_file.exists() {
                read_file_info_from_file(config)?
            } else {
                return Err(DanoError::new("No valid output file exists").into());
            };

            // then dedup
            let unique_paths: Vec<FileInfo> = recorded_file_info_with_duplicates
                .iter()
                .into_group_map_by(|file_info| match &file_info.metadata {
                    Some(metadata) => metadata.hash_value.clone(),
                    None => Integer::ZERO,
                })
                .into_iter()
                .flat_map(|(_hash, group_file_info)| {
                    group_file_info
                        .into_iter()
                        .max_by_key(|file_info| match &file_info.metadata {
                            Some(metadata) => metadata.last_written,
                            None => SystemTime::UNIX_EPOCH,
                        })
                })
                .cloned()
                .collect();

            // and overwrite
            write_all_new_paths(config, &unique_paths, WriteType::OverwriteAll)
        }
        _ => Ok(()),
    }
}
