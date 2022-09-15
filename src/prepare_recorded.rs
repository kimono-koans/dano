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

use rayon::prelude::*;

use crate::lookup_file_info::FileInfo;
use crate::utility::{deserialize, read_file_info_from_file};
use crate::{Config, DanoError, DanoResult, ExecMode, DANO_XATTR_KEY_NAME};

pub fn get_recorded_file_info(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let mut file_info_from_xattrs: Vec<FileInfo> = {
        config
            .paths
            .par_iter()
            .flat_map(|path| xattr::get(path, DANO_XATTR_KEY_NAME).map(|opt| (path, opt)))
            .flat_map(|(path, opt)| opt.map(|s| (path, s)))
            .flat_map(|(path, bytes)| std::str::from_utf8(&bytes).map(|i| (path, i.to_owned())))
            .flat_map(|(path, line)| deserialize(&line).map(|i| (path, i)))
            .map(|(path, file_info)| {
                // use the actual path name always
                if path != &file_info.path {
                    FileInfo {
                        version: file_info.version,
                        path: path.to_owned(),
                        metadata: file_info.metadata,
                    }
                } else {
                    file_info
                }
            })
            .collect()
    };

    let file_info_from_file = if config.hash_file.exists() {
        read_file_info_from_file(config)?
    } else {
        Vec::new()
    };

    // combine
    file_info_from_xattrs.extend(file_info_from_file);
    let mut recorded_file_info: Vec<FileInfo> = file_info_from_xattrs;

    // if empty no valid hashes to test in test mode, so we should quit
    if let ExecMode::Compare(compare_config) = &config.exec_mode {
        if compare_config.opt_test_mode && recorded_file_info.is_empty() {
            return Err(DanoError::new("No valid hashes to test.  Quitting.").into());
        }
    }

    // sort and dedup in case we have paths in both hash file and xattrs
    recorded_file_info.par_sort_unstable_by_key(|file_info| file_info.path.clone());
    recorded_file_info.dedup_by_key(|file_info| file_info.path.clone());

    Ok(recorded_file_info)
}
