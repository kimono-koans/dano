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

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::lookup_file_info::FileInfo;
use crate::util::{deserialize, read_input_file};
use crate::{Config, DanoResult, ExecMode, DANO_FILE_INFO_VERSION, DANO_XATTR_KEY_NAME};

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

    // include test paths and combine
    file_info_from_xattrs.extend(file_info_from_file);
    let mut recorded_file_info: Vec<FileInfo> = match config.exec_mode {
        ExecMode::Test => {
            // why include "test paths"?  Because in Compare/Write modes we are comparing against
            // what we have to see what to do.  In Test mode we are comparing a file against
            // itself, and to compare a file against itself we must include it among the FileInfo
            // items to be be compared
            let map_recorded: BTreeMap<PathBuf, FileInfo> = file_info_from_xattrs
                .into_iter()
                .map(|file_info| (file_info.path.clone(), file_info))
                .collect();

            let test_paths: Vec<FileInfo> = config
                .paths
                .par_iter()
                .filter(|path| !map_recorded.contains_key(path.as_path()))
                .map(|path| FileInfo {
                    version: DANO_FILE_INFO_VERSION,
                    path: path.clone(),
                    metadata: None,
                })
                .collect();

            let mut recorded_file_info: Vec<FileInfo> = map_recorded.into_values().collect();
            recorded_file_info.extend(test_paths);
            recorded_file_info
        }
        _ => file_info_from_xattrs,
    };

    // sort and dedup in case we have paths in hash file and xattrs
    recorded_file_info.sort_by_key(|file_info| file_info.path.clone());
    recorded_file_info.dedup_by_key(|file_info| file_info.path.clone());

    Ok(recorded_file_info)
}
