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
use std::path::Path;

use rayon::prelude::*;

use crate::lookup::FileInfo;
use crate::utility::{deserialize, read_file_info_from_file};
use crate::{Config, DanoError, DanoResult, ExecMode, DANO_XATTR_KEY_NAME};

pub struct RecordedFileInfo {
    inner: Vec<FileInfo>,
}

impl From<Vec<FileInfo>> for RecordedFileInfo {
    fn from(vec: Vec<FileInfo>) -> Self {
        Self { inner: vec }
    }
}

impl Deref for RecordedFileInfo {
    type Target = Vec<FileInfo>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl RecordedFileInfo {
    pub fn into_inner(self) -> Vec<FileInfo> {
        self.inner
    }

    pub fn new(config: &Config) -> DanoResult<Self> {
        let mut recorded_file_info: Vec<FileInfo> = match &config.exec_mode {
            ExecMode::Write(write_config) if write_config.opt_import_flac => {
                Self::from_flac(config)?
            }
            _ => Self::from_recorded(config)?,
        };

        // if empty, no valid hashes to test in test mode, and we should quit
        if let ExecMode::Test(test_mode_config) = &config.exec_mode {
            if recorded_file_info.is_empty()
                && !test_mode_config.opt_overwrite_old
                && !test_mode_config.opt_write_new
            {
                return Err(DanoError::new("No valid hashes to test.  Quitting.").into());
            }
        }

        // sort and dedup in case we have paths in both hash file and xattrs
        recorded_file_info.par_sort_unstable_by_key(|file_info| file_info.path.clone());
        recorded_file_info.dedup_by_key(|file_info| file_info.path.clone());

        Ok(Self {
            inner: recorded_file_info,
        })
    }

    fn from_recorded(config: &Config) -> DanoResult<Vec<FileInfo>> {
        let mut file_info_from_xattrs: Vec<FileInfo> = {
            config
                .paths
                .par_iter()
                .filter_map(|path| match Self::read_file_info_from_xattr(path) {
                    Some(file_info) => Some((path, file_info)),
                    None => {
                        eprintln!(
                            "WARN: No dano extended attribute exists for path: {:?}",
                            path
                        );
                        None
                    }
                })
                .map(|(path, file_info)| {
                    // use the actual path name always
                    if path != &file_info.path {
                        return FileInfo {
                            version: file_info.version,
                            path: path.to_owned(),
                            metadata: file_info.metadata,
                        };
                    }

                    file_info
                })
                .collect()
        };

        if config.hash_file.exists() {
            let file_info_from_file = read_file_info_from_file(config)?;
            file_info_from_xattrs.extend(file_info_from_file);
        }

        // combine
        Ok(file_info_from_xattrs)
    }

    fn read_file_info_from_xattr(path: &Path) -> Option<FileInfo> {
        fn inner(path: &Path) -> DanoResult<Option<FileInfo>> {
            if let Some(bytes) = xattr::get(path, DANO_XATTR_KEY_NAME)? {
                let line = std::str::from_utf8(&bytes)?;
                let res = deserialize(line)?;

                Ok(Some(res))
            } else {
                Ok(None)
            }
        }

        // key idea is to let errors be printed but also let the files, which have errors,
        // to have those errors be flattened
        match inner(path) {
            Ok(res) => res,
            Err(err) => {
                eprintln!("ERROR: {:?}", err);
                None
            }
        }
    }
}
