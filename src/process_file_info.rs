// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::Arc, time::SystemTime};

use crossbeam::channel::Receiver;
use itertools::{Either, Itertools};
use rayon::prelude::*;

use crate::{Config, DanoResult, ExecMode};

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::util::{
    deserialize, overwrite_all_paths, print_file_info, read_input_file, write_new_paths,
};

pub struct CompareHashesBundle {
    hash_matches: Vec<FileInfo>,
    hash_non_matches: Vec<FileInfo>,
}

pub fn exec_process_file_info(
    config: &Config,
    requested_paths: &[PathBuf],
    paths_from_file: &[FileInfo],
    rx_item: Receiver<FileInfo>,
) -> DanoResult<CompareHashesBundle> {
    // prepare for loop
    let file_map = Arc::new(get_file_map(config, paths_from_file, requested_paths)?);
    let mut exit_code = 0;
    // L
    let mut hash_matches = Vec::new();
    // R
    let mut hash_non_matches = Vec::new();

    while let Ok(file_info) = rx_item.recv() {
        match config.exec_mode {
            ExecMode::Write(_) | ExecMode::Compare => {
                if let (Some(either), _) = verify_file_info(config, &file_info, file_map.clone()) {
                    match either {
                        Either::Left(file_info) => hash_matches.push(file_info),
                        Either::Right(file_info) => hash_non_matches.push(file_info),
                    }
                }
            }
            ExecMode::Test => {
                let (_, test_exit_code) = verify_file_info(config, &file_info, file_map.clone());
                if test_exit_code != 0 {
                    exit_code = test_exit_code
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
    hash_matches.sort_unstable_by_key(|file_info| file_info.clone().path);
    hash_non_matches.sort_unstable_by_key(|file_info| file_info.clone().path);

    Ok(CompareHashesBundle {
        hash_matches,
        hash_non_matches,
    })
}

pub fn write_to_file(
    config: &Config,
    compare_hashes_bundle: &CompareHashesBundle,
) -> DanoResult<()> {
    if matches!(config.exec_mode, ExecMode::Write(_))
        || (config.exec_mode == ExecMode::Compare && config.opt_write_new)
            && !compare_hashes_bundle.hash_non_matches.is_empty()
    {
        write_new_paths(config, &compare_hashes_bundle.hash_non_matches)?
    } else if !config.opt_silent && matches!(config.exec_mode, ExecMode::Write(_)) {
        eprintln!("No new paths to write.");
    }

    if !compare_hashes_bundle.hash_matches.is_empty()
        && ((matches!(config.exec_mode, ExecMode::Write(_)) && config.opt_overwrite_old)
            || (config.exec_mode == ExecMode::Compare
                && config.opt_overwrite_old
                && config.opt_write_new))
    {
        // append new paths
        write_new_paths(config, &compare_hashes_bundle.hash_matches)?;

        // read back
        let paths_from_file_with_duplicates: Vec<FileInfo> = if config.output_file.exists() {
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
                        Some(metadata) => metadata.last_written,
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

fn is_same_hash(file_map: &BTreeMap<PathBuf, Option<FileMetadata>>, path: &FileInfo) -> bool {
    let file_map_by_hash = file_map
        .iter()
        .filter_map(|(path, metadata)| {
            metadata
                .as_ref()
                .map(|metadata| (metadata.hash_value, path))
        })
        .collect::<BTreeMap<u128, &PathBuf>>();

    match &path.metadata {
        Some(metadata) => file_map_by_hash.contains_key(&metadata.hash_value),
        None => false,
    }
}

fn is_same_filename(file_map: &BTreeMap<PathBuf, Option<FileMetadata>>, path: &FileInfo) -> bool {
    file_map.contains_key(&path.path)
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
        // for write and test, we take the paths /available/ from file and make
        // dummy versions of the rest
        ExecMode::Test => requested_paths
            .iter()
            .map(|path| match paths_from_file_map.get(path) {
                Some(metadata) => (path.to_owned(), metadata.to_owned()),
                None => (path.to_owned(), None),
            })
            .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>(),
        ExecMode::Write(_) | ExecMode::Compare => paths_from_file_map,
        ExecMode::Print => BTreeMap::new(),
    };
    Ok(res)
}

fn verify_file_info(
    config: &Config,
    file_info: &FileInfo,
    file_map: Arc<BTreeMap<PathBuf, Option<FileMetadata>>>,
) -> (Option<Either<FileInfo, FileInfo>>, i32) {
    let is_same_hash = is_same_hash(&file_map, file_info);
    let is_same_filename = is_same_filename(&file_map, file_info);
    let mut test_exit_code = 0;

    // must check whether metadata is none first
    let opt_file_info = if file_info.metadata.is_none() {
        match config.exec_mode {
            ExecMode::Compare | ExecMode::Test => {
                eprintln!("{:?}: WARNING, path does not exist", &file_info.path)
            }
            ExecMode::Write(_) => {
                if !config.opt_silent {
                    let _ = print_file_info(file_info);
                }
            }
            _ => unreachable!(),
        }
        test_exit_code = 2;
        None
    } else if is_same_filename && is_same_hash {
        match config.exec_mode {
            ExecMode::Compare | ExecMode::Test => eprintln!("{:?}: OK", &file_info.path),
            ExecMode::Write(_) => {
                if !config.opt_silent {
                    let _ = print_file_info(file_info);
                }
            }
            _ => unreachable!(),
        }
        Some(Either::Left(file_info.clone()))
    } else if is_same_hash {
        // know we are in Compare mode, so require write_new and overwrite_old
        // to specify things will be overwritten
        match config.exec_mode {
            ExecMode::Compare | ExecMode::Test => {
                if config.opt_write_new && config.opt_overwrite_old {
                    eprintln!(
                        "{:?}: OK, but path has same hash for new filename.  Old file info has been overwritten.",
                        file_info.path
                    );
                } else {
                    eprintln!(
                        "{:?}: OK, but path has same hash for new filename",
                        file_info.path
                    );
                }
            }
            ExecMode::Write(_) => {
                if !config.opt_silent {
                    let _ = print_file_info(file_info);
                }
            }
            _ => unreachable!(),
        }
        Some(Either::Left(file_info.clone()))
    } else if is_same_filename {
        match config.exec_mode {
            ExecMode::Compare | ExecMode::Test => {
                eprintln!(
                    "{:?}: WARNING, path has new hash for same filename",
                    file_info.path
                );
            }
            ExecMode::Write(_) => {
                if !config.opt_silent {
                    let _ = print_file_info(file_info);
                }
            }
            _ => unreachable!(),
        }
        None
    } else {
        match config.exec_mode {
            ExecMode::Compare | ExecMode::Test => {
                eprintln!("{:?}: Path is a new file", file_info.path);
            }
            ExecMode::Write(_) => {
                if !config.opt_silent {
                    let _ = print_file_info(file_info);
                }
            }
            _ => unreachable!(),
        }
        Some(Either::Right(file_info.clone()))
    };

    (opt_file_info, test_exit_code)
}
