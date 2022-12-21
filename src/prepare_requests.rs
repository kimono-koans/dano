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

use rayon::prelude::*;

use crate::config::SelectedStreams;
use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::{Config, DanoResult};

#[derive(Debug, Clone)]
pub struct FileInfoRequest {
    pub path: PathBuf,
    pub hash_algo: Option<Box<str>>,
    pub decoded: Option<bool>,
    pub selected_streams: Option<SelectedStreams>,
}

pub struct RequestBundle {
    inner: Vec<FileInfoRequest>,
}

impl From<Vec<FileInfoRequest>> for RequestBundle {
    fn from(vec: Vec<FileInfoRequest>) -> Self {
        Self { inner: vec }
    }
}

impl Deref for RequestBundle {
    type Target = Vec<FileInfoRequest>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl RequestBundle {
    pub fn into_inner(self) -> Vec<FileInfoRequest> {
        self.inner
    }

    pub fn new(config: &Config, recorded_file_info: &[FileInfo]) -> DanoResult<Self> {
        // here we generate a file info request because we need more than
        // the path name when the user has specified a different hash algo

        // first, we generate a map of the recorded file info to test against
        // map will allow

        // on disk
        let from_recorded_request = |path: &PathBuf, metadata: &FileMetadata| -> FileInfoRequest {
            FileInfoRequest {
                path: path.clone(),
                hash_algo: Some(metadata.hash_algo.clone()),
                decoded: Some(metadata.decoded),
                selected_streams: Some(metadata.selected_streams.to_owned()),
            }
        };

        // new requests
        let new_request = |path: &PathBuf| -> FileInfoRequest {
            FileInfoRequest {
                path: path.clone(),
                hash_algo: None,
                decoded: None,
                selected_streams: None,
            }
        };

        let mut recorded_file_info_requests: BTreeMap<PathBuf, FileInfoRequest> =
            recorded_file_info
                .par_iter()
                .map(|file_info| match &file_info.metadata {
                    Some(metadata) => (
                        file_info.path.clone(),
                        from_recorded_request(&file_info.path, metadata),
                    ),
                    None => (file_info.path.clone(), new_request(&file_info.path)),
                })
                .collect();

        let paths_requests: Vec<FileInfoRequest> = config
            .paths
            .par_iter()
            .map(|path| match recorded_file_info_requests.get(path) {
                Some(value) => value.to_owned(),
                None => new_request(path),
            })
            .collect();

        paths_requests.into_iter().for_each(|request| {
            // don't care about the Option returned
            let _ = recorded_file_info_requests.insert(request.path.clone(), request);
        });

        let combined = recorded_file_info_requests.into_values().collect();

        Ok(Self { inner: combined })
    }
}
