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

use std::{collections::BTreeMap, path::PathBuf};

use rayon::prelude::*;

use crate::lookup_file_info::FileInfo;
use crate::{Config, DanoResult, FileInfoRequest};

pub fn get_file_info_requests(
    config: &Config,
    recorded_file_info: &[FileInfo],
) -> DanoResult<Vec<FileInfoRequest>> {
    // here we generate a file info request because we need more than
    // the path name when the user has specified a different hash algo

    // first, we generate a map of the recorded file info to test against
    // map will allow
    let mut recorded_file_info_requests: BTreeMap<PathBuf, FileInfoRequest> = recorded_file_info
        .par_iter()
        .map(|file_info| match &file_info.metadata {
            Some(metadata) => (
                file_info.path.clone(),
                FileInfoRequest {
                    path: file_info.path.clone(),
                    hash_algo: Some(metadata.hash_algo.clone()),
                },
            ),
            None => (
                file_info.path.clone(),
                FileInfoRequest {
                    path: file_info.path.clone(),
                    hash_algo: None,
                },
            ),
        })
        .collect();

    let paths_requests: Vec<FileInfoRequest> = config
        .paths
        .par_iter()
        .map(|path| FileInfoRequest {
            path: path.clone(),
            hash_algo: None,
        })
        .map(
            |request| match recorded_file_info_requests.get(&request.path) {
                Some(value) => value.to_owned(),
                None => request,
            },
        )
        .collect();

    // include all config.paths in requests, but only if record_file_info does not contain the hash algo
    paths_requests.into_iter().for_each(|request| {
            // don't care about the Option returned
            let _ = recorded_file_info_requests.insert(request.path.clone(), request);
        });

    let combined = recorded_file_info_requests.into_values().collect();

    Ok(combined)
}
