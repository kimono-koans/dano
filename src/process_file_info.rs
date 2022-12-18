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

use std::{collections::BTreeMap, ops::Deref, path::PathBuf, sync::Arc};

use crossbeam::channel::Receiver;
use itertools::Either;
use rayon::prelude::*;

use crate::{Config, DanoResult, ExecMode};

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::utility::{print_file_info, print_out_buf};

#[derive(Debug, Clone)]
pub enum RemainderType {
    NewFile,
    ModifiedFilename,
}

#[derive(Debug, Clone)]
pub struct RemainderFilesBundle {
    pub files: Vec<FileInfo>,
    pub remainder_type: RemainderType,
}

pub struct ProcessedFiles {
    pub file_bundle: Vec<RemainderFilesBundle>,
    pub exit_code: i32,
}

impl ProcessedFiles {
    pub fn new(
        config: &Config,
        recorded_file_info: &[FileInfo],
        rx_item: Receiver<FileInfo>,
    ) -> DanoResult<ProcessedFiles> {
        // prepare for loop
        let file_map = Arc::new(FileMap::new(recorded_file_info)?);
        let mut exit_code = 0;
        // L
        let mut new_filenames = Vec::new();
        // R
        let mut new_files = Vec::new();

        // loop while recv from channel
        while let Ok(file_info) = rx_item.recv() {
            if let (Some(new_files_partitioned), test_exit_code) =
                file_map.verify(config, file_info)?
            {
                match new_files_partitioned {
                    Either::Left(file_info) => new_filenames.push(file_info),
                    Either::Right(file_info) => new_files.push(file_info),
                }
                if test_exit_code != 0 {
                    exit_code = test_exit_code
                }
            }
        }

        // sort new paths before writing to file, threads may complete in non-sorted order
        new_filenames.par_sort_unstable_by_key(|file_info| file_info.path.clone());
        new_files.par_sort_unstable_by_key(|file_info| file_info.path.clone());

        let file_bundle = vec![
            RemainderFilesBundle {
                files: new_files,
                remainder_type: RemainderType::NewFile,
            },
            RemainderFilesBundle {
                files: new_filenames,
                remainder_type: RemainderType::ModifiedFilename,
            },
        ];

        Ok(ProcessedFiles {
            file_bundle,
            exit_code,
        })
    }
}

struct FileMap {
    inner: BTreeMap<PathBuf, Option<FileMetadata>>,
}

impl From<BTreeMap<PathBuf, Option<FileMetadata>>> for FileMap {
    fn from(map: BTreeMap<PathBuf, Option<FileMetadata>>) -> Self {
        Self { inner: map }
    }
}

impl Deref for FileMap {
    type Target = BTreeMap<PathBuf, Option<FileMetadata>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl FileMap {
    fn new(recorded_file_info: &[FileInfo]) -> DanoResult<Self> {
        let recorded_file_info_map = recorded_file_info
            .par_iter()
            .cloned()
            .map(|file_info| (file_info.path, file_info.metadata))
            .collect::<BTreeMap<PathBuf, Option<FileMetadata>>>();

        Ok(Self {
            inner: recorded_file_info_map,
        })
    }

    fn is_same_filename(&self, file_info: &FileInfo) -> bool {
        self.inner.contains_key(&file_info.path)
    }

    fn is_same_hash(&self, file_info: &FileInfo) -> bool {
        match &file_info.metadata {
            Some(path_metadata) => {
                // fast path
                if let Some(Some(fast_path_metadata)) = self.get(&file_info.path) {
                    if fast_path_metadata.hash_value == path_metadata.hash_value {
                        return true;
                    }
                }

                // slow path
                self.deref()
                    .into_par_iter()
                    .filter_map(|(_file_map_path, file_map_metadata)| file_map_metadata.as_ref())
                    .any(|file_map_metadata| {
                        path_metadata.hash_value == file_map_metadata.hash_value
                    })
            }
            None => false,
        }
    }

    fn verify(
        &self,
        config: &Config,
        file_info: FileInfo,
    ) -> DanoResult<(Option<Either<FileInfo, FileInfo>>, i32)> {
        let is_same_hash = self.is_same_hash(&file_info);
        let is_same_filename = self.is_same_filename(&file_info);
        let mut test_exit_code = 0;

        // must check whether metadata is none first
        let opt_file_info = if file_info.metadata.is_none() {
            // always print, even in silent
            match config.exec_mode {
                ExecMode::Test(_) => {
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
            // always print, even in silent
            match config.exec_mode {
                ExecMode::Test(_) => {
                    print_out_buf(&format!("{:?}: Path is a new file.\n", file_info.path))?;
                }
                ExecMode::Write(_) => {
                    print_file_info(config, &file_info)?;
                }
                _ => unreachable!(),
            }
            Some(Either::Right(file_info))
        } else if is_same_filename && is_same_hash {
            if !config.opt_silent {
                match config.exec_mode {
                    ExecMode::Test(_) => {
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
            // always print, even in silent
            match &config.exec_mode {
                ExecMode::Test(test_config) => {
                    if test_config.opt_write_new && test_config.opt_overwrite_old {
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
                ExecMode::Test(_) => {
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
            test_exit_code = 3;
            None
        } else {
            unreachable!()
        };

        Ok((opt_file_info, test_exit_code))
    }
}
