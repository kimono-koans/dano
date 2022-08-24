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

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use crossbeam::channel::Receiver;
use itertools::Either;
use rayon::prelude::*;

use crate::{Config, DanoResult, ExecMode};

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::utility::{print_file_info, print_out_buf};

#[derive(Debug, Clone)]
pub struct NewFilesBundle {
    pub new_filenames: Vec<FileInfo>,
    pub new_files: Vec<FileInfo>,
}

pub fn process_file_info_exec(
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
                    verify_file_info(config, file_info, file_map.clone())?
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
    new_filenames.par_sort_unstable_by_key(|file_info| file_info.path.clone());
    new_files.par_sort_unstable_by_key(|file_info| file_info.path.clone());

    Ok(NewFilesBundle {
        new_filenames,
        new_files,
    })
}

fn is_same_hash(file_map: &BTreeMap<PathBuf, Option<FileMetadata>>, path: &FileInfo) -> bool {
    file_map
        .into_par_iter()
        .filter_map(|(_file_map_path, file_map_metadata)| file_map_metadata.as_ref())
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
    file_info: FileInfo,
    file_map: Arc<BTreeMap<PathBuf, Option<FileMetadata>>>,
) -> DanoResult<(Option<Either<FileInfo, FileInfo>>, i32)> {
    let is_same_hash = is_same_hash(&file_map, &file_info);
    let is_same_filename = is_same_filename(&file_map, &file_info);
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
                print_file_info(config, &file_info)?;
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
                    print_file_info(config, &file_info)?;
                }
                _ => unreachable!(),
            }
        }
        Some(Either::Right(file_info))
    } else if is_same_filename && is_same_hash {
        if !config.opt_silent {
            match config.exec_mode {
                ExecMode::Compare(_) => {
                    print_out_buf(&format!("{:?}: OK\n", &file_info.path))?;
                }
                ExecMode::Write(_) => {
                    print_file_info(config, &file_info)?;
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
                print_file_info(config, &file_info)?;
            }
            _ => unreachable!(),
        }
        Some(Either::Left(file_info))
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
                print_file_info(config, &file_info)?;
            }
            _ => unreachable!(),
        }
        None
    } else {
        unreachable!()
    };

    Ok((opt_file_info, test_exit_code))
}
