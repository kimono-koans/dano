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

use itertools::Itertools;

use crate::ingest::RecordedFileInfo;
use crate::{Config, ExecMode};

use crate::lookup::FileInfo;
use crate::process::{ProcessedFiles, RemainderBundle};
use crate::utility::{
    get_output_file, make_tmp_file, print_err_buf, read_file_info_from_file, write_file,
    write_non_file, DanoError, DanoResult,
};

const WRITE_NEW_PREFIX: &str = "Writing dano hash for: ";
const EMPTY_STR: &str = "";
const OVERWRITE_OLD_PREFIX: &str = "Overwriting dano hash for: ";

const NOT_WRITE_NEW_PREFIX: &str =
    "WARN: Not writing dano hash for (as writing is not specified): ";
const NOT_WRITE_NEW_SUFFIX: &str = ", --write-new was not specified.";

const NOT_OVERWRITE_OLD_PREFIX: &str =
    "WARN: Not overwriting dano hash for (as overwriting is not specified): ";
const NOT_OVERWRITE_OLD_SUFFIX: &str = ", --overwrite was not specified.";

const NEW_FILES_EMPTY: &str = "No new file paths to write.";
const MODIFIED_FILE_NAMES_EMPTY: &str = "No old file data to overwrite.";

// in this mod "write" refers to writing to file or xattr
// and "print" refers to printing out to stdout or stderr
//
// for any write non-dry run action we will write to disk
// and print to notify the user

pub enum WriteType {
    Append,
    OverwriteAll,
}

impl ProcessedFiles {
    pub fn write_out(self, config: &Config) -> DanoResult<i32> {
        [self.new_files, self.modified_file_names]
            .into_iter()
            .try_for_each(|remainder_bundle| {
                // if files.empty() guard applies to both sides of the pattern
                match &remainder_bundle {
                    RemainderBundle::NewFile(files) | RemainderBundle::ModifiedFilename(files)
                        if files.is_empty() =>
                    {
                        Self::print_bundle_empty(config, &remainder_bundle);
                        Ok(())
                    }
                    _ => remainder_bundle.write_out(config),
                }
            })?;
        Ok(self.exit_code)
    }

    fn print_bundle_empty(config: &Config, remainder_bundle: &RemainderBundle) {
        if !config.is_single_path {
            match &config.exec_mode {
                ExecMode::Test(test_config)
                    if !test_config.opt_write_new || !test_config.opt_overwrite_old =>
                {
                    match remainder_bundle {
                        RemainderBundle::NewFile(_) if !test_config.opt_write_new => {
                            eprintln!("{}{}", NEW_FILES_EMPTY, NOT_WRITE_NEW_SUFFIX);
                        }
                        RemainderBundle::ModifiedFilename(_) if !test_config.opt_overwrite_old => {
                            eprintln!("{}{}", MODIFIED_FILE_NAMES_EMPTY, NOT_OVERWRITE_OLD_SUFFIX);
                        }
                        _ => unreachable!(),
                    }
                }
                _ => match remainder_bundle {
                    RemainderBundle::NewFile(_) => {
                        eprintln!("{}", NEW_FILES_EMPTY);
                    }
                    RemainderBundle::ModifiedFilename(_) => {
                        eprintln!("{}", MODIFIED_FILE_NAMES_EMPTY);
                    }
                },
            }
        }
    }
}

impl RemainderBundle {
    fn write_out(self, config: &Config) -> DanoResult<()> {
        match &config.exec_mode {
            ExecMode::Write(_) => match self {
                RemainderBundle::NewFile(files) => {
                    WriteableFileInfo::from(files).write_action(config, NOT_WRITE_NEW_PREFIX, WRITE_NEW_PREFIX)?
                }
                RemainderBundle::ModifiedFilename(files) => {
                    WriteableFileInfo::from(files).write_action(config, NOT_OVERWRITE_OLD_PREFIX, OVERWRITE_OLD_PREFIX)?
                }
            },
            ExecMode::Test(test_config) => match self {
                RemainderBundle::NewFile(files) if test_config.opt_write_new => {
                    WriteableFileInfo::from(files).write_action(config, NOT_WRITE_NEW_PREFIX, WRITE_NEW_PREFIX)?
                }
                RemainderBundle::ModifiedFilename(files) if test_config.opt_overwrite_old => {
                    WriteableFileInfo::from(files).write_action(config, NOT_OVERWRITE_OLD_PREFIX, OVERWRITE_OLD_PREFIX)?
                }
                RemainderBundle::NewFile(files) => {
                    WriteableFileInfo::from(files).print_action(NOT_WRITE_NEW_PREFIX, NOT_WRITE_NEW_SUFFIX)?
                }
                RemainderBundle::ModifiedFilename(files) => {
                    WriteableFileInfo::from(files).print_action(NOT_OVERWRITE_OLD_PREFIX, NOT_OVERWRITE_OLD_SUFFIX)?
                }
            },
            _ => unreachable!(),
        }
        Ok(())
    }
}

pub struct WriteableFileInfo {
    inner: Vec<FileInfo>,
}

impl From<Vec<FileInfo>> for WriteableFileInfo {
    fn from(value: Vec<FileInfo>) -> Self {
        Self { inner: value }
    }
}

impl From<RecordedFileInfo> for WriteableFileInfo {
    fn from(value: RecordedFileInfo) -> Self {
        Self {
            inner: value.into_inner(),
        }
    }
}

impl WriteableFileInfo {
    fn write_action(
        self,
        config: &Config,
        dry_prefix: &str,
        wet_prefix: &str,
    ) -> DanoResult<()> {
        if config.opt_dry_run {
            self.print_action(dry_prefix, EMPTY_STR)?;
            self.overwrite_all(config)
        } else {
            self.print_action(wet_prefix, EMPTY_STR)
        }
    }

    fn print_action(&self, prefix: &str, suffix: &str) -> DanoResult<()> {    
        self.inner.iter().try_for_each(|file_info| {
            print_err_buf(&format!("{}{:?}{}\n", prefix, file_info.path, suffix))
        })
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
                    .into_iter()
                    .filter(|file_info| file_info.metadata.is_some())
                    .into_group_map_by(|file_info| {
                        file_info.metadata.as_ref().unwrap().hash_value.clone()
                    })
                    .into_iter()
                    .flat_map(|(_hash, group_file_info)| {
                        group_file_info.into_iter().max_by_key(|file_info| {
                            file_info.metadata.as_ref().unwrap().last_written
                        })
                    })
                    .collect();

                let writeable_file_info: WriteableFileInfo = Self {
                    inner: unique_paths,
                };

                // and overwrite
                writeable_file_info.write_new(config, WriteType::OverwriteAll)
            }
            _ => Ok(()),
        }
    }

    pub fn write_new(&self, config: &Config, write_type: WriteType) -> DanoResult<()> {
        // ExecMode::Dump is about writing to a file always want to skip xattrs
        // can always be enabled by env var so ad hoc debugging can be tricky
        if !config.opt_dry_run {
            if config.opt_xattr && !matches!(config.exec_mode, ExecMode::Dump) {
                self.inner.iter().try_for_each(write_non_file)
            } else {
                match write_type {
                    WriteType::Append => {
                        let mut output_file = get_output_file(config, WriteType::Append)?;
                        self.inner
                            .iter()
                            .try_for_each(|file_info| write_file(file_info, &mut output_file))
                    }
                    WriteType::OverwriteAll => {
                        let mut output_file = get_output_file(config, WriteType::OverwriteAll)?;

                        self.inner
                            .iter()
                            .try_for_each(|file_info| write_file(file_info, &mut output_file))?;

                        std::fs::rename(
                            make_tmp_file(config.output_file.as_path()),
                            &config.output_file,
                        )
                        .map_err(|err| err.into())
                    }
                }
            }
        } else {
            Ok(())
        }
    }
}
