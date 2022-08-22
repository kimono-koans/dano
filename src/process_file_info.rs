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

use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::Arc, time::SystemTime};

use crossbeam::channel::Receiver;
use itertools::{Either, Itertools};
use rayon::prelude::*;
use rug::Integer;

use crate::{Config, DanoResult, ExecMode};

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::util::{
    deserialize, print_err_buf, print_file_info, print_out_buf, read_input_file,
    write_all_new_paths, DanoError, WriteType,
};

#[derive(Debug, Clone)]
pub struct NewFilesBundle {
    pub new_filenames: Vec<FileInfo>,
    pub new_files: Vec<FileInfo>,
}

pub fn exec_process_file_info(
    config: &Config,
    recorded_file_info: &[FileInfo],
    rx_item: Receiver<FileInfo>,
) -> DanoResult<NewFilesBundle> {
    // prepare for loop
    let file_map = Arc::new(get_file_map(recorded_file_info)?);
    let mut exit_code = 0;
    // L
    let mut new_filenames = Vec::new();
    // R
    let mut new_files = Vec::new();

    // loop while recv from channel
    while let Ok(file_info) = rx_item.recv() {
        match config.exec_mode {
            ExecMode::Write(_) | ExecMode::Compare(_) => {
                if let (Some(either), test_exit_code) =
                    verify_file_info(config, &file_info, file_map.clone())?
                {
                    match either {
                        Either::Left(file_info) => new_filenames.push(file_info),
                        Either::Right(file_info) => new_files.push(file_info),
                    }
                    if test_exit_code != 0 {
                        exit_code += test_exit_code
                    }
                }
            }
            ExecMode::Print => unreachable!(),
        }
    }

    // exit with non-zero status is test is not "OK"
    if let ExecMode::Compare(compare_config) = &config.exec_mode {
        if compare_config.opt_test_mode {
            std::process::exit(exit_code)
        }
    }

    // sort new paths before writing to file, threads may complete in non-sorted order
    new_filenames.par_sort_unstable_by_key(|file_info| file_info.clone().path);
    new_files.par_sort_unstable_by_key(|file_info| file_info.clone().path);

    Ok(NewFilesBundle {
        new_filenames,
        new_files,
    })
}

pub fn write_new_file_info(config: &Config, new_files_bundle: &NewFilesBundle) -> DanoResult<()> {
    // write new files - no hash match in record
    if !new_files_bundle.new_files.is_empty() {
        let write_new = || -> DanoResult<()> {
            new_files_bundle
                .new_files
                .iter()
                .try_for_each(|file_info| {
                    print_err_buf(&format!("Writing dano hash for: {:?}\n", file_info.path))
                })?;
            write_all_new_paths(config, &new_files_bundle.new_files, WriteType::Append)
        };
        match &config.exec_mode {
            ExecMode::Write(_) => write_new()?,
            ExecMode::Compare(compare_config) => {
                if compare_config.opt_write_new {
                    write_new()?
                } else {
                    new_files_bundle
                        .new_files
                        .iter()
                        .try_for_each(|file_info| {
                            print_err_buf(&format!(
                                "Not writing dano hash for: {:?}, --write-new was not specified.\n",
                                file_info.path
                            ))
                        })?
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

    // write old files with new names - hash matches
    if !new_files_bundle.new_filenames.is_empty() {
        let overwrite_old = || -> DanoResult<()> {
            new_files_bundle
                .new_filenames
                .iter()
                .try_for_each(|file_info| {
                    print_err_buf(&format!(
                        "Overwriting dano hash for: {:?}\n",
                        file_info.path
                    ))
                })?;
            overwrite_old_file_info(config, new_files_bundle)
        };

        match &config.exec_mode {
            ExecMode::Write(_) => overwrite_old()?,
            ExecMode::Compare(compare_config) => {
                if compare_config.opt_overwrite_old {
                    overwrite_old()?
                } else {
                    new_files_bundle
                        .new_filenames
                        .iter()
                        .try_for_each(|file_info| {
                            print_err_buf(&format!(
                                "Not overwriting dano hash for: {:?}, --overwrite was not specified.\n",
                                file_info.path
                            ))
                        })?
                }
            }
            _ => unreachable!(),
        }
    } else if let ExecMode::Compare(compare_config) = &config.exec_mode {
        if compare_config.opt_overwrite_old {
            eprintln!("No old file hashes to overwrite.");
        } else {
            eprintln!("No old file hash to overwrite and --overwrite was not specified.");
        }
    } else {
        eprintln!("No old file hashes to overwrite.");
    }

    Ok(())
}

fn overwrite_old_file_info(config: &Config, new_files_bundle: &NewFilesBundle) -> DanoResult<()> {
    // append new paths
    write_all_new_paths(config, &new_files_bundle.new_filenames, WriteType::Append)?;

    // overwrite all paths if in non-xattr/file write mode
    match &config.exec_mode {
        ExecMode::Write(write_config) if !write_config.opt_xattr => {
            // read back
            let recorded_file_info_with_duplicates: Vec<FileInfo> = if config.output_file.exists() {
                let mut input_file = read_input_file(config)?;
                let mut buffer = String::new();
                input_file.read_to_string(&mut buffer)?;
                // important this blows up because if you change the struct it can't deserialize
                buffer.par_lines().flat_map(deserialize).collect()
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

fn is_same_hash(file_map: &BTreeMap<PathBuf, Option<FileMetadata>>, path: &FileInfo) -> bool {
    file_map
        .clone()
        .into_par_iter()
        .filter_map(|(_file_map_path, file_map_metadata)| file_map_metadata)
        .any(|file_map_metadata| match &path.metadata {
            Some(path_metadata) => path_metadata.hash_value == file_map_metadata.hash_value,
            None => false,
        })
}

fn is_same_filename(file_map: &BTreeMap<PathBuf, Option<FileMetadata>>, path: &FileInfo) -> bool {
    file_map.contains_key(&path.path)
}

fn get_file_map(
    recorded_file_info: &[FileInfo],
) -> DanoResult<BTreeMap<PathBuf, Option<FileMetadata>>> {
    let recorded_file_info_map = recorded_file_info
        .par_iter()
        .cloned()
        .map(|file_info| (file_info.path, file_info.metadata))
        .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>();

    Ok(recorded_file_info_map)
}

fn verify_file_info(
    config: &Config,
    file_info: &FileInfo,
    file_map: Arc<BTreeMap<PathBuf, Option<FileMetadata>>>,
) -> DanoResult<(Option<Either<FileInfo, FileInfo>>, i32)> {
    let is_same_hash = is_same_hash(&file_map, file_info);
    let is_same_filename = is_same_filename(&file_map, file_info);
    let mut test_exit_code = 0;

    // must check whether metadata is none first
    let opt_file_info = if file_info.metadata.is_none() {
        // always print, even in silent
        match config.exec_mode {
            ExecMode::Compare(_) => {
                print_out_buf(&format!(
                    "{:?}: WARNING, path does not exist.\n",
                    &file_info.path
                ))?;
            }
            ExecMode::Write(_) => {
                print_file_info(config, file_info)?;
            }
            _ => unreachable!(),
        }
        test_exit_code = 2;
        None
    } else if !is_same_filename && !is_same_hash {
        if !config.opt_silent {
            match config.exec_mode {
                ExecMode::Compare(_) => {
                    print_out_buf(&format!("{:?}: Path is a new file.\n", file_info.path))?;
                }
                ExecMode::Write(_) => {
                    print_file_info(config, file_info)?;
                }
                _ => unreachable!(),
            }
        }
        Some(Either::Right(file_info.clone()))
    } else if is_same_filename && is_same_hash {
        if !config.opt_silent {
            match config.exec_mode {
                ExecMode::Compare(_) => {
                    print_out_buf(&format!("{:?}: OK\n", &file_info.path))?;
                }
                ExecMode::Write(_) => {
                    print_file_info(config, file_info)?;
                }
                _ => unreachable!(),
            }
        }
        None
    } else if is_same_hash {
        match &config.exec_mode {
            ExecMode::Compare(compare_config) => {
                if compare_config.opt_write_new && compare_config.opt_overwrite_old {
                    print_out_buf(format!(
                        "{:?}: OK, but path has same hash for new filename.  Old file info has been overwritten.\n",
                        file_info.path
                    ).as_ref())?;
                } else {
                    print_out_buf(
                        format!(
                            "{:?}: OK, but path has same hash for new filename.\n",
                            file_info.path
                        )
                        .as_ref(),
                    )?;
                }
            }
            ExecMode::Write(_) => {
                print_file_info(config, file_info)?;
            }
            _ => unreachable!(),
        }
        Some(Either::Left(file_info.clone()))
    } else if is_same_filename {
        // always print, even in silent
        match config.exec_mode {
            ExecMode::Compare(_) => {
                print_out_buf(&format!(
                    "{:?}: WARNING, path has new hash for same filename.\n",
                    file_info.path
                ))?;
            }
            ExecMode::Write(_) => {
                print_file_info(config, file_info)?;
            }
            _ => unreachable!(),
        }
        None
    } else {
        unreachable!()
    };

    Ok((opt_file_info, test_exit_code))
}
