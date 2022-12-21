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
use std::time::SystemTime;

use itertools::Itertools;
use rug::Integer;

use crate::prepare_recorded::RecordedFileInfo;
use crate::{Config, DanoResult, ExecMode};

use crate::lookup_file_info::FileInfo;
use crate::process_file_info::{RemainderBundle, RemainderType};
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

const NEW_FILES_EMPTY: &str = "No new file paths to write";
const MODIFIED_FILE_NAMES_EMPTY: &str = "No old file data to overwrite";

pub enum WriteType {
    Append,
    OverwriteAll,
}

pub struct PrintBundle {
    inner: Vec<RemainderBundle>,
}

impl From<Vec<RemainderBundle>> for PrintBundle {
    fn from(vec: Vec<RemainderBundle>) -> Self {
        Self { inner: vec }
    }
}

impl Deref for PrintBundle {
    type Target = Vec<RemainderBundle>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl PrintBundle {
    pub fn write_out(self, config: &Config) -> DanoResult<()> {
        self.inner.into_iter().try_for_each(|file_bundle| {
            if !file_bundle.files.is_empty() {
                file_bundle.write_out(config)
            } else {
                Self::print_bundle_empty(config, &file_bundle.remainder_type);
                Ok(())
            }
        })
    }

    fn print_bundle_empty(config: &Config, remainder_type: &RemainderType) {
        if !config.is_single_path {
            match &config.exec_mode {
                ExecMode::Test(test_config)
                    if !test_config.opt_write_new || !test_config.opt_overwrite_old =>
                {
                    match remainder_type {
                        RemainderType::NewFile if !test_config.opt_write_new => {
                            eprintln!("{}{}", NEW_FILES_EMPTY, NOT_WRITE_NEW_SUFFIX);
                        }
                        RemainderType::ModifiedFilename if !test_config.opt_overwrite_old => {
                            eprintln!("{}{}", MODIFIED_FILE_NAMES_EMPTY, NOT_OVERWRITE_OLD_SUFFIX);
                        }
                        _ => unreachable!(),
                    }
                }
                _ => match remainder_type {
                    RemainderType::NewFile => {
                        eprintln!("{}.", NEW_FILES_EMPTY);
                    }
                    RemainderType::ModifiedFilename => {
                        eprintln!("{}.", MODIFIED_FILE_NAMES_EMPTY);
                    }
                },
            }
        }
    }
}

impl RemainderBundle {
    fn write_out(self, config: &Config) -> DanoResult<()> {
        match &config.exec_mode {
            ExecMode::Write(_) => match &self.remainder_type {
                RemainderType::NewFile => {
                    self.exec_write_action(config, NOT_WRITE_NEW_PREFIX, WRITE_NEW_PREFIX)?
                }
                &RemainderType::ModifiedFilename => {
                    self.exec_write_action(config, NOT_OVERWRITE_OLD_PREFIX, OVERWRITE_OLD_PREFIX)?
                }
            },
            ExecMode::Test(test_config) => {
                if test_config.opt_write_new
                    && matches!(self.remainder_type, RemainderType::NewFile)
                {
                    self.exec_write_action(config, NOT_WRITE_NEW_PREFIX, WRITE_NEW_PREFIX)?
                } else if test_config.opt_overwrite_old
                    && matches!(&self.remainder_type, RemainderType::ModifiedFilename)
                {
                    self.exec_write_action(config, NOT_OVERWRITE_OLD_PREFIX, OVERWRITE_OLD_PREFIX)?
                } else {
                    let recorded_file_info: RecordedFileInfo = self.clone().files.into();

                    match &self.remainder_type {
                        RemainderType::NewFile => recorded_file_info
                            .print_write_action(NOT_WRITE_NEW_PREFIX, NOT_WRITE_NEW_SUFFIX)?,
                        RemainderType::ModifiedFilename => recorded_file_info.print_write_action(
                            NOT_OVERWRITE_OLD_PREFIX,
                            NOT_OVERWRITE_OLD_SUFFIX,
                        )?,
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn exec_write_action(
        self,
        config: &Config,
        dry_prefix: &str,
        wet_prefix: &str,
    ) -> DanoResult<()> {
        //  notice the fn print_write_action() parameters are different for write/dry run
        let file_info_set: PrintedFileInfo = self.files.into();

        if config.opt_dry_run {
            file_info_set.print_write_action(dry_prefix, EMPTY_STR)
        } else {
            file_info_set.print_write_action(wet_prefix, EMPTY_STR)?;

            match self.remainder_type {
                RemainderType::ModifiedFilename => file_info_set.overwrite_all(config),
                RemainderType::NewFile => {
                    write_new(config, file_info_set.deref(), WriteType::Append)
                }
            }
        }
    }
}

pub type PrintedFileInfo = RecordedFileInfo;

impl PrintedFileInfo {
    fn print_write_action(&self, prefix: &str, suffix: &str) -> DanoResult<()> {
        self.iter().try_for_each(|file_info| {
            print_err_buf(&format!("{}{:?}{}\n", prefix, file_info.path, suffix))
        })
    }

    pub fn overwrite_all(&self, config: &Config) -> DanoResult<()> {
        // append new paths
        write_new(config, self.deref(), WriteType::Append)?;

        // overwrite all paths if in non-xattr/file write mode
        match &config.exec_mode {
            ExecMode::Write(_) if !config.opt_xattr => {
                // read back
                let recorded_file_info_with_duplicates: Vec<FileInfo> =
                    if config.output_file.exists() {
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
                write_new(config, &unique_paths, WriteType::OverwriteAll)
            }
            _ => Ok(()),
        }
    }
}

pub fn write_new(config: &Config, new_files: &[FileInfo], write_type: WriteType) -> DanoResult<()> {
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
