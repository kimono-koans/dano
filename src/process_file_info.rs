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

use crate::{Config, DanoResult, ExecMode, XattrMode};

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::util::{
    deserialize, print_file_info, print_out_buf, read_input_file, write_all_new_paths, WriteType,
};

pub struct NewFilesBundle {
    hash_matches: Vec<FileInfo>,
    hash_non_matches: Vec<FileInfo>,
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
    let mut hash_matches = Vec::new();
    // R
    let mut hash_non_matches = Vec::new();

    // loop while recv from channel
    while let Ok(file_info) = rx_item.recv() {
        match config.exec_mode {
            ExecMode::Write(_) | ExecMode::Compare => {
                if let (Some(either), _) = verify_file_info(config, &file_info, file_map.clone())? {
                    match either {
                        Either::Left(file_info) => hash_matches.push(file_info),
                        Either::Right(file_info) => hash_non_matches.push(file_info),
                    }
                }
            }
            ExecMode::Test => {
                let (_, test_exit_code) = verify_file_info(config, &file_info, file_map.clone())?;
                if test_exit_code != 0 {
                    exit_code += test_exit_code
                }
            }
            ExecMode::Print => unreachable!(),
        }
    }

    // exit with non-zero status is test is not "OK"
    if matches!(config.exec_mode, ExecMode::Test) {
        std::process::exit(exit_code)
    }

    // sort new paths before writing to file, threads may complete in non-sorted order
    hash_matches.par_sort_unstable_by_key(|file_info| file_info.clone().path);
    hash_non_matches.par_sort_unstable_by_key(|file_info| file_info.clone().path);

    Ok(NewFilesBundle {
        hash_matches,
        hash_non_matches,
    })
}

pub fn write_new_file_info(config: &Config, new_files_bundle: &NewFilesBundle) -> DanoResult<()> {
    // write new files - no hash match in record
    if !new_files_bundle.hash_non_matches.is_empty()
        && matches!(config.exec_mode, ExecMode::Write(_))
        || (config.exec_mode == ExecMode::Compare && config.opt_write_new)
    {
        write_all_new_paths(
            config,
            &new_files_bundle.hash_non_matches,
            WriteType::Append,
        )?
    } else if !config.opt_silent && matches!(config.exec_mode, ExecMode::Write(_)) {
        eprintln!("No new paths to write.");
    }

    // write old files with new names - hash matches
    if !new_files_bundle.hash_matches.is_empty()
        && ((matches!(config.exec_mode, ExecMode::Write(_)) && config.opt_overwrite_old)
            || (config.exec_mode == ExecMode::Compare
                && config.opt_overwrite_old
                && config.opt_write_new))
    {
        // append new paths
        write_all_new_paths(config, &new_files_bundle.hash_matches, WriteType::Append)?;

        // overwrite all paths if in non-xattr/file write mode
        match &config.exec_mode {
            ExecMode::Write(write_config)
                if matches!(write_config.opt_xattr, XattrMode::Disabled) =>
            {
                // read back
                let recorded_file_info_with_duplicates: Vec<FileInfo> =
                    if config.output_file.exists() {
                        let mut input_file = read_input_file(config)?;
                        let mut buffer = String::new();
                        input_file.read_to_string(&mut buffer)?;
                        // important this blows up because if you change the struct it can't deserialize
                        buffer
                            .lines()
                            .filter(|line| !line.starts_with("//"))
                            .map(deserialize)
                            .collect::<DanoResult<Vec<FileInfo>>>()?
                    } else {
                        Vec::new()
                    };

                // then dedup
                let unique_paths: Vec<FileInfo> = recorded_file_info_with_duplicates
                    .iter()
                    .into_group_map_by(|file_info| match &file_info.metadata {
                        Some(metadata) => metadata.hash_value,
                        None => u128::MIN,
                    })
                    .into_iter()
                    .flat_map(|(_hash, group_file_info)| {
                        group_file_info.into_iter().max_by_key(|file_info| {
                            match &file_info.metadata {
                                Some(metadata) => metadata.last_written,
                                None => SystemTime::UNIX_EPOCH,
                            }
                        })
                    })
                    .cloned()
                    .collect();

                // and overwrite
                write_all_new_paths(config, &unique_paths, WriteType::OverwriteAll)
            }
            _ => Ok(()),
        }
    } else {
        Ok(())
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
            ExecMode::Compare | ExecMode::Test => {
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
    } else if is_same_filename && is_same_hash {
        if !config.opt_silent {
            match config.exec_mode {
                ExecMode::Compare | ExecMode::Test => {
                    print_out_buf(&format!("{:?}: OK\n", &file_info.path))?;
                }
                ExecMode::Write(_) => {
                    print_file_info(config, file_info)?;
                }
                _ => unreachable!(),
            }
        }
        Some(Either::Left(file_info.clone()))
    } else if is_same_hash {
        match config.exec_mode {
            ExecMode::Compare | ExecMode::Test => {
                let err_buf = if config.opt_write_new && config.opt_overwrite_old {
                    // Compare mode only condition
                    format!(
                            "{:?}: OK, but path has same hash for new filename.  Old file info has been overwritten.\n",
                            file_info.path
                        )
                } else {
                    format!(
                        "{:?}: OK, but path has same hash for new filename.\n",
                        file_info.path
                    )
                };
                print_out_buf(&err_buf)?;
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
            ExecMode::Compare | ExecMode::Test => {
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
    } else if !is_same_filename && !is_same_hash {
        if !config.opt_silent {
            match config.exec_mode {
                ExecMode::Compare | ExecMode::Test => {
                    print_out_buf(&format!("{:?}: Path is a new file.\n", file_info.path))?;
                }
                ExecMode::Write(_) => {
                    print_file_info(config, file_info)?;
                }
                _ => unreachable!(),
            }
        }
        Some(Either::Right(file_info.clone()))
    } else {
        unreachable!()
    };

    Ok((opt_file_info, test_exit_code))
}
