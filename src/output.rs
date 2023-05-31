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

use crate::ingest::RecordedFileInfo;
use crate::{Config, ExecMode, HEXADECIMAL_RADIX};

use crate::lookup::{FileInfo, HashValue};
use crate::process::{RemainderBundle, RemainderType};
use crate::utility::{
    get_output_file, make_tmp_file, print_err_buf, read_file_info_from_file, write_file,
    write_non_file, DanoError, DanoResult,
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

// in this mod "write" refers to writing to file or xattr
// and "print" refers to printing out to stdout or stderr
//
// for any write non-dry run action we will write to disk
// and print to notify the user

pub enum WriteType {
    Append,
    OverwriteAll,
}

pub struct WriteOutBundle {
    inner: Vec<RemainderBundle>,
}

impl WriteOutBundle {
    fn into_inner(self) -> Vec<RemainderBundle> {
        self.inner
    }
}

impl From<Vec<RemainderBundle>> for WriteOutBundle {
    fn from(vec: Vec<RemainderBundle>) -> Self {
        Self { inner: vec }
    }
}

impl Deref for WriteOutBundle {
    type Target = Vec<RemainderBundle>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl WriteOutBundle {
    pub fn write_out(self, config: &Config) -> DanoResult<()> {
        self.into_inner()
            .into_iter()
            .try_for_each(|remainder_bundle| {
                if !remainder_bundle.files.is_empty() {
                    remainder_bundle.write_out(config)
                } else {
                    Self::print_bundle_empty(config, &remainder_bundle.remainder_type);
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
                    match &self.remainder_type {
                        RemainderType::NewFile => {
                            self.print_write_action(NOT_WRITE_NEW_PREFIX, NOT_WRITE_NEW_SUFFIX)?
                        }
                        RemainderType::ModifiedFilename => self.print_write_action(
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
        if config.opt_dry_run {
            self.print_write_action(dry_prefix, EMPTY_STR)
        } else {
            self.print_write_action(wet_prefix, EMPTY_STR)?;

            let writeable_file_info: WriteableFileInfo = self.files.into();

            match self.remainder_type {
                RemainderType::ModifiedFilename => writeable_file_info.overwrite_all(config),
                RemainderType::NewFile => writeable_file_info.write_new(config, WriteType::Append),
            }
        }
    }

    fn print_write_action(&self, prefix: &str, suffix: &str) -> DanoResult<()> {
        self.files.iter().try_for_each(|file_info| {
            print_err_buf(&format!("{}{:?}{}\n", prefix, file_info.path, suffix))
        })
    }
}

pub type WriteableFileInfo = RecordedFileInfo;

impl WriteableFileInfo {
    pub fn write_new(&self, config: &Config, write_type: WriteType) -> DanoResult<()> {
        // ExecMode::Dump is about writing to a file always want to skip xattrs
        // can always be enabled by env var so ad hoc debugging can be tricky
        if !config.opt_dry_run {
            if config.opt_xattr && !matches!(config.exec_mode, ExecMode::Dump) {
                self.iter().try_for_each(write_non_file)
            } else {
                match write_type {
                    WriteType::Append => {
                        let mut output_file = get_output_file(config, WriteType::Append)?;
                        self.iter()
                            .try_for_each(|file_info| write_file(file_info, &mut output_file))
                    }
                    WriteType::OverwriteAll => {
                        let mut output_file = get_output_file(config, WriteType::OverwriteAll)?;

                        self.iter()
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

    pub fn overwrite_all(&self, config: &Config) -> DanoResult<()> {
        // append new paths
        self.write_new(config, WriteType::Append)?;

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
                        None => HashValue {
                            radix: HEXADECIMAL_RADIX,
                            value: "0".into(),
                        },
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

                let printed_info: WriteableFileInfo = unique_paths.into();

                // and overwrite
                printed_info.write_new(config, WriteType::OverwriteAll)
            }
            _ => Ok(()),
        }
    }
}
