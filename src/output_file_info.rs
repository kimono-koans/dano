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
use crate::process_file_info::{BundleType, NewFileBundle};
use crate::utility::{
    get_output_file, make_tmp_file, print_err_buf, read_file_info_from_file, write_file,
    write_non_file, DanoError,
};

const WRITE_NEW_PREFIX: &str = "Writing dano hash for: ";
const EMPTY_STR: &str = "";
const OVERWRITE_OLD_PREFIX: &str = "Overwriting dano hash for: ";

const NOT_WRITE_NEW_PREFIX: &str = "Not writing dano hash for: ";
const NOT_WRITE_NEW_SUFFIX: &str = ", --write-new was not specified.";

const NOT_OVERWRITE_OLD_PREFIX: &str = "Not overwriting dano hash for: ";
const NOT_OVERWRITE_OLD_SUFFIX: &str = ", --overwrite was not specified.";

pub enum WriteType {
    Append,
    OverwriteAll,
}

pub fn write_file_info_exec(config: &Config, new_files_bundle: &[NewFileBundle]) -> DanoResult<()> {
    new_files_bundle.iter().try_for_each(|file_bundle| {
        if !file_bundle.files.is_empty() {
            write_new_files(config, &file_bundle.files, &file_bundle.bundle_type)
        } else {
            print_bundle_empty(config, &file_bundle.bundle_type);
            Ok(())
        }
    })
}

fn write_new_files(
    config: &Config,
    files_bundle: &[FileInfo],
    bundle_type: &BundleType,
) -> DanoResult<()> {
    match &config.exec_mode {
        ExecMode::Write(_) => match bundle_type {
            BundleType::NewFiles => exec_write_action(
                config,
                files_bundle,
                NOT_WRITE_NEW_PREFIX,
                WRITE_NEW_PREFIX,
                bundle_type,
            )?,
            &BundleType::NewFileNames => exec_write_action(
                config,
                files_bundle,
                NOT_OVERWRITE_OLD_PREFIX,
                OVERWRITE_OLD_PREFIX,
                bundle_type,
            )?,
        },
        ExecMode::Compare(compare_config) => {
            if compare_config.opt_write_new && matches!(bundle_type, BundleType::NewFiles) {
                exec_write_action(
                    config,
                    files_bundle,
                    NOT_WRITE_NEW_PREFIX,
                    WRITE_NEW_PREFIX,
                    bundle_type,
                )?
            } else if compare_config.opt_overwrite_old
                && matches!(bundle_type, BundleType::NewFileNames)
            {
                exec_write_action(
                    config,
                    files_bundle,
                    NOT_OVERWRITE_OLD_PREFIX,
                    OVERWRITE_OLD_PREFIX,
                    bundle_type,
                )?
            } else {
                match bundle_type {
                    BundleType::NewFiles => print_write_action(
                        NOT_WRITE_NEW_PREFIX,
                        NOT_WRITE_NEW_SUFFIX,
                        files_bundle,
                    )?,
                    BundleType::NewFileNames => print_write_action(
                        NOT_OVERWRITE_OLD_PREFIX,
                        NOT_OVERWRITE_OLD_SUFFIX,
                        files_bundle,
                    )?,
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn print_bundle_empty(config: &Config, bundle_type: &BundleType) {
    if let ExecMode::Compare(compare_config) = &config.exec_mode {
        match bundle_type {
            BundleType::NewFiles => {
                if compare_config.opt_write_new {
                    eprintln!("No new file paths to write.");
                } else {
                    eprintln!("No new file paths to write, and --write-new was not specified");
                }
            }
            BundleType::NewFileNames => {
                if compare_config.opt_overwrite_old {
                    eprintln!("No old file data to overwrite.");
                } else {
                    eprintln!("No old file data to overwrite, and --overwrite was not specified.");
                }
            }
        }
    } else {
        match bundle_type {
            BundleType::NewFiles => {
                eprintln!("No new file paths to write.");
            }
            BundleType::NewFileNames => {
                eprintln!("No old file data to overwrite.");
            }
        }
    }
}

fn exec_write_action(
    config: &Config,
    files_bundle: &[FileInfo],
    dry_prefix: &str,
    wet_prefix: &str,
    bundle_type: &BundleType,
) -> DanoResult<()> {
    if config.opt_dry_run {
        print_write_action(dry_prefix, EMPTY_STR, files_bundle)
    } else {
        print_write_action(wet_prefix, EMPTY_STR, files_bundle)?;

        match bundle_type {
            BundleType::NewFileNames => overwrite_old_file_info(config, files_bundle),
            BundleType::NewFiles => write_all_new_paths(config, files_bundle, WriteType::Append),
        }
    }
}

fn print_write_action(prefix: &str, suffix: &str, file_bundle: &[FileInfo]) -> DanoResult<()> {
    file_bundle.iter().try_for_each(|file_info| {
        print_err_buf(&format!("{}{:?}{}\n", prefix, file_info.path, suffix))
    })
}

pub fn write_all_new_paths(
    config: &Config,
    new_files: &[FileInfo],
    write_type: WriteType,
) -> DanoResult<()> {
    // ExecMode::Dump is about writing to a file always want to skip xattrs
    // can always be enabled by env var so ad hoc debugging can be tricky
    if !config.opt_dry_run {
        if config.opt_xattr && !matches!(config.exec_mode, ExecMode::Dump) {
            new_files.iter().try_for_each(write_non_file)
        } else {
            match write_type {
                WriteType::Append => {
                    let mut output_file = get_output_file(config, WriteType::Append)?;
                    new_files
                        .iter()
                        .try_for_each(|file_info| write_file(file_info, &mut output_file))
                }
                WriteType::OverwriteAll => {
                    let mut output_file = get_output_file(config, WriteType::OverwriteAll)?;

                    new_files
                        .iter()
                        .try_for_each(|file_info| write_file(file_info, &mut output_file))?;

                    std::fs::rename(
                        make_tmp_file(config.output_file.as_path()),
                        config.output_file.clone(),
                    )
                    .map_err(|err| err.into())
                }
            }
        }
    } else {
        Ok(())
    }
}

pub fn overwrite_old_file_info(config: &Config, files_bundle: &[FileInfo]) -> DanoResult<()> {
    // append new paths
    write_all_new_paths(config, files_bundle, WriteType::Append)?;

    // overwrite all paths if in non-xattr/file write mode
    match &config.exec_mode {
        ExecMode::Write(_) if !config.opt_xattr => {
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
