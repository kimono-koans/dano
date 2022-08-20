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

use std::io::Read;

use rayon::prelude::*;

use crate::lookup_file_info::FileInfo;
use crate::util::{deserialize, read_input_file};
use crate::{Config, DanoError, DanoResult, ExecMode, DANO_XATTR_KEY_NAME};

pub fn get_recorded_file_info(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let mut file_info_from_xattrs: Vec<FileInfo> = {
        config
            .paths
            .par_iter()
            .flat_map(|path| xattr::get(path, DANO_XATTR_KEY_NAME).map(|opt| (path, opt)))
            .flat_map(|(path, opt)| opt.map(|s| (path, s)))
            .flat_map(|(path, s)| std::str::from_utf8(&s).map(|i| (path, i.to_owned())))
            .flat_map(|(path, s)| deserialize(&s).map(|i| (path, i)))
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
        let mut input_file = read_input_file(config)?;
        let mut buffer = String::new();
        input_file.read_to_string(&mut buffer)?;
        buffer.par_lines().flat_map(deserialize).collect()
    } else {
        Vec::new()
    };

    if let ExecMode::Compare(compare_config) = &config.exec_mode {
        if compare_config.opt_test_mode && file_info_from_file.is_empty() {
            return Err(DanoError::new("No valid hashes could be read from the specified hash file (required in Test mode).").into());
        }
    }

    // include test paths and combine
    file_info_from_xattrs.extend(file_info_from_file);
    let mut recorded_file_info: Vec<FileInfo> = file_info_from_xattrs;

    // sort and dedup in case we have paths in hash file and xattrs
    recorded_file_info.sort_by_key(|file_info| file_info.path.clone());
    recorded_file_info.dedup_by_key(|file_info| file_info.path.clone());

    Ok(recorded_file_info)
}
