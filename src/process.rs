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

use std::{collections::BTreeMap, ops::Deref, path::PathBuf};

use crossbeam_channel::Receiver;
use itertools::Either;
use rayon::prelude::*;

use crate::config::TestModeWriteOpt;
use crate::ingest::RecordedFileInfo;
use crate::{Config, ExecMode};

use crate::lookup::{FileInfo, FileMetadata};
use crate::utility::{print_file_info, print_out_buf, DanoResult};

#[derive(Debug, Clone)]
pub enum RemainderBundle {
    NewFile(Vec<FileInfo>),
    ModifiedFilename(Vec<FileInfo>),
}

pub struct ProcessedFiles {
    pub new_files: RemainderBundle,
    pub modified_file_names: RemainderBundle,
    pub exit_code: i32,
}

impl ProcessedFiles {
    pub fn new(
        config: &Config,
        recorded_file_info: RecordedFileInfo,
        rx_item: Receiver<FileInfo>,
    ) -> DanoResult<ProcessedFiles> {
        // prepare for loop
        let file_map = FileMap::new(recorded_file_info.into_inner());
        let mut exit_code = 0;
        // L
        let mut modified_file_names = Vec::new();
        // R
        let mut new_files = Vec::new();

        // loop while recv from channel
        while let Ok(file_info) = rx_item.recv() {
            if let (Some(new_files_partitioned), test_exit_code) =
                &file_map.verify(config, &file_info)?
            {
                match new_files_partitioned {
                    Either::Left(_) => modified_file_names.push(file_info),
                    Either::Right(_) => new_files.push(file_info),
                }

                if test_exit_code != &0 {
                    exit_code = *test_exit_code
                }
            }
        }

        // sort new paths before writing to file, threads may complete in non-sorted order
        modified_file_names.par_sort_unstable_by_key(|file_info| file_info.path.clone());
        new_files.par_sort_unstable_by_key(|file_info| file_info.path.clone());

        Ok(ProcessedFiles {
            new_files: RemainderBundle::NewFile(new_files),
            modified_file_names: RemainderBundle::ModifiedFilename(modified_file_names),
            exit_code,
        })
    }
}

struct FileMap {
    inner: BTreeMap<PathBuf, Option<FileMetadata>>,
}

impl Deref for FileMap {
    type Target = BTreeMap<PathBuf, Option<FileMetadata>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl From<Vec<FileInfo>> for FileMap {
    fn from(value: Vec<FileInfo>) -> Self {
        let recorded_file_info_map: BTreeMap<PathBuf, Option<FileMetadata>> = value
            .into_iter()
            .map(|file_info| (file_info.path, file_info.metadata))
            .collect();

        Self {
            inner: recorded_file_info_map,
        }
    }
}

impl FileMap {
    fn new(recorded_file_info: Vec<FileInfo>) -> Self {
        recorded_file_info.into()
    }

    fn verify<'a>(
        &self,
        config: &Config,
        file_info: &'a FileInfo,
    ) -> DanoResult<(Option<Either<&'a FileInfo, &'a FileInfo>>, i32)> {
        let is_same_hash = self.is_same_hash(&file_info);
        let is_same_filename = self.is_same_filename(&file_info);
        let mut test_exit_code = 0;

        // must check whether metadata is none first
        let opt_file_info = if file_info.metadata.is_none() {
            // always print, even in silent
            match config.exec_mode {
                ExecMode::Test(_) => {
                    print_out_buf(&format!(
                        "WARN: {:?}: Path does not exist.\n",
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
                ExecMode::Test(opt_test_write_opt) => {
                    if matches!(opt_test_write_opt, Some(TestModeWriteOpt::OverwriteAll)) {
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
                        "WARN: {:?}: Path has new hash for same filename.\n",
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

    fn is_same_filename(&self, file_info: &FileInfo) -> bool {
        self.deref().contains_key(&file_info.path)
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

                // slow path -- why? if we have hash match with a new path name
                self.par_iter()
                    .filter_map(|(_file_map_path, file_map_metadata)| file_map_metadata.as_ref())
                    .any(|file_map_metadata| {
                        path_metadata.hash_value == file_map_metadata.hash_value
                    })
            }
            None => false,
        }
    }
}
