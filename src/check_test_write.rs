// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::Arc, time::SystemTime};

use crossbeam::channel::{Receiver, Sender};
use itertools::{Either, Itertools};
use rayon::prelude::*;

use crate::{Config, DanoResult, ExecMode};

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::util::{
    deserialize, display_output_path, overwrite_all_paths, read_input_file, write_new_paths,
};

pub struct NewFilesBundle {
    new_filenames: Vec<FileInfo>,
    new_files: Vec<FileInfo>,
}

pub fn file_info_from_paths(
    config: &Config,
    requested_paths: &[PathBuf],
    paths_from_file: &[FileInfo],
) -> DanoResult<NewFilesBundle> {
    let rx_item = {
        let (tx_item, rx_item): (Sender<FileInfo>, Receiver<FileInfo>) =
            crossbeam::channel::unbounded();

        requested_paths.iter().for_each(|path_buf| {
            let tx_item_clone = tx_item.clone();
            rayon::scope(|file_info_scope| {
                file_info_scope.spawn(move |_| {
                    let _ = FileInfo::send_file_info(path_buf, tx_item_clone);
                })
            });
        });

        // explicitly drop here of we will hold onto the ref and loop forever
        drop(tx_item);
        rx_item
    };

    // prepare for loop
    let file_map = Arc::new(get_file_map(config, paths_from_file, requested_paths)?);
    let mut exit_code = 0;
    // L
    let mut new_filenames = Vec::new();
    // R
    let mut new_files = Vec::new();

    while let Ok(file_info) = rx_item.recv() {
        match config.exec_mode {
            ExecMode::Write => {
                display_output_path(&file_info)?;
                new_files.push(file_info);
            }
            ExecMode::Compare => {
                if let Some(either) = compare_check(config, &file_info, file_map.clone()) {
                    match either {
                        Either::Left(file_info) => new_filenames.push(file_info),
                        Either::Right(file_info) => new_files.push(file_info),
                    }
                }
            }
            ExecMode::Test => {
                let ret_val = test_check(&file_info, file_map.clone());
                if ret_val != 0 {
                    exit_code = ret_val
                }
            }
            ExecMode::Print => unreachable!(),
        }
    }

    if matches!(config.exec_mode, ExecMode::Test) {
        std::process::exit(exit_code)
    }

    Ok(NewFilesBundle {
        new_filenames,
        new_files,
    })
}

fn is_same_hash(
    paths_from_file_map: &BTreeMap<PathBuf, Option<FileMetadata>>,
    path: &FileInfo,
) -> bool {
    let paths_from_file_map_by_hash = paths_from_file_map
        .iter()
        .filter_map(|(path, metadata)| {
            metadata
                .as_ref()
                .map(|metadata| (metadata.hash_value, path))
        })
        .collect::<BTreeMap<u128, &PathBuf>>();

    match &path.metadata {
        Some(metadata) => paths_from_file_map_by_hash.contains_key(&metadata.hash_value),
        None => false,
    }
}

pub fn overwrite_and_write_new(
    config: &Config,
    new_file_bundle: &NewFilesBundle,
) -> DanoResult<()> {
    if config.exec_mode == ExecMode::Write
        || (config.exec_mode == ExecMode::Compare && config.opt_write_new)
            && !new_file_bundle.new_files.is_empty()
    {
        write_new_paths(config, &new_file_bundle.new_files)?
    } else if !config.opt_silent {
        eprintln!("No new paths to write.");
    }

    if !new_file_bundle.new_filenames.is_empty()
        && (config.exec_mode == ExecMode::Write && config.opt_overwrite_old)
        || (config.exec_mode == ExecMode::Compare
            && config.opt_overwrite_old
            && config.opt_write_new)
    {
        // append new paths
        write_new_paths(config, &new_file_bundle.new_filenames)?;

        // read back
        let paths_from_file_with_duplicates: Vec<FileInfo> =
            if config.pwd.join("dano_hashes.txt").exists() {
                let mut input_file = read_input_file(&config.pwd)?;
                let mut buffer = String::new();
                input_file.read_to_string(&mut buffer)?;
                buffer.lines().flat_map(deserialize).collect()
            } else {
                Vec::new()
            };

        // then dedup
        let unique_paths: Vec<FileInfo> = paths_from_file_with_duplicates
            .iter()
            .into_group_map_by(|file_info| match &file_info.metadata {
                Some(metadata) => metadata.hash_value,
                None => u128::MIN,
            })
            .into_iter()
            .flat_map(|(_hash, group_file_info)| {
                group_file_info
                    .into_iter()
                    .max_by_key(|file_info| match &file_info.metadata {
                        Some(metadata) => metadata.last_checked,
                        None => SystemTime::UNIX_EPOCH,
                    })
            })
            .cloned()
            .collect();

        // and overwrite
        overwrite_all_paths(config, &unique_paths)
    } else {
        Ok(())
    }
}

fn is_same_filename(
    paths_from_file_map: &BTreeMap<PathBuf, Option<FileMetadata>>,
    path: &FileInfo,
) -> bool {
    paths_from_file_map.contains_key(&path.path)
}

fn get_file_map(
    config: &Config,
    paths_from_file: &[FileInfo],
    requested_paths: &[PathBuf],
) -> DanoResult<BTreeMap<PathBuf, Option<FileMetadata>>> {
    let paths_from_file_map = paths_from_file
        .par_iter()
        .cloned()
        .map(|file_info| (file_info.path, file_info.metadata))
        .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>();

    let res = match config.exec_mode {
        ExecMode::Test => requested_paths
            .iter()
            .map(|path| match paths_from_file_map.get(path) {
                Some(metadata) => (path.to_owned(), metadata.to_owned()),
                None => (path.to_owned(), None),
            })
            .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>(),
        ExecMode::Compare => paths_from_file_map,
        ExecMode::Write | ExecMode::Print => BTreeMap::new(),
    };
    Ok(res)
}

fn compare_check(
    config: &Config,
    file_info: &FileInfo,
    paths_from_file_map: Arc<BTreeMap<PathBuf, Option<FileMetadata>>>,
) -> Option<Either<FileInfo, FileInfo>> {
    let is_same_hash = is_same_hash(&paths_from_file_map, file_info);
    let is_same_filename = is_same_filename(&paths_from_file_map, file_info);

    // must check whether metadata is none first
    let res = if file_info.metadata.is_none() {
        if config.exec_mode != ExecMode::Write {
            eprintln!("{:?}: Path is a new file", file_info.path);
        }
        Some((file_info.clone(), is_same_hash))
    } else if is_same_filename && is_same_hash {
        if config.exec_mode != ExecMode::Write {
            eprintln!("{:?}: OK", file_info.path);
        }
        None
    } else if is_same_hash {
        if config.exec_mode != ExecMode::Write {
            // know we are in Compare mode, so require write_new and overwrite_old
            // to specify things will be overwritten
            if config.opt_write_new && config.opt_overwrite_old {
                eprintln!(
                    "{:?}: OK, but path has same hash for new filename.  Hash data will be overwritten.",
                    file_info.path
                );
            } else {
                eprintln!(
                    "{:?}: OK, but path has same hash for new filename",
                    file_info.path
                );
            }
        }
        Some((file_info.clone(), is_same_hash))
    } else if is_same_filename {
        if config.exec_mode != ExecMode::Write {
            eprintln!(
                "{:?}: WARNING, path has new hash for same filename",
                file_info.path
            );
        }
        None
    } else {
        None
    };

    match res {
        Some((file_info, is_same_hash)) => {
            if is_same_hash {
                Some(Either::Left(file_info))
            } else {
                Some(Either::Right(file_info))
            }
        }
        None => None,
    }
}

fn test_check(
    requested_path: &FileInfo,
    requested_paths_map: Arc<BTreeMap<PathBuf, Option<FileMetadata>>>,
) -> i32 {
    let is_same_hash = is_same_hash(&requested_paths_map, requested_path);
    let is_same_filename = is_same_filename(&requested_paths_map, requested_path);
    let mut exit_code = 0;

    // must check whether metadata is none first
    // a number of checks compare paths against themselves
    // like HERE, so always check if we were able to run ffmpeg on the path
    if requested_path.metadata.is_none() {
        eprintln!("{:?}: WARNING, path does not exist", requested_path.path);
        exit_code = 2;
    } else if is_same_filename && is_same_hash {
        eprintln!("{:?}: OK", requested_path.path);
    } else if is_same_hash {
        eprintln!(
            "{:?}: OK, but path has same hash for new filename",
            requested_path.path
        );
    } else if is_same_filename {
        eprintln!(
            "{:?}: WARNING, path has new hash for same filename",
            requested_path.path
        );
    }

    exit_code
}
