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
use crate::{DanoResult, FileInfoRequest};

pub fn get_file_info_requests(
    recorded_file_info: &[FileInfo],
    opt_requested_paths: Option<&Vec<PathBuf>>,
) -> DanoResult<Vec<FileInfoRequest>> {
    // here we generate a file info request because we need more than
    // the path name when the user has specified a different hash algo

    // first, we generate a map of the recorded file info to test against
    // map will allow
    let recorded_file_info_requests: BTreeMap<PathBuf, FileInfoRequest> = recorded_file_info
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

    // next, we consider the new paths/not recorded paths.  map will allow us to
    // dedup the against the recorded file info an only include None values for
    // hash selection where needed
    let selected = if let Some(requested_paths) = opt_requested_paths {
        requested_paths
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
            .collect()
    } else {
        recorded_file_info_requests.into_values().collect()
    };

    Ok(selected)
}
