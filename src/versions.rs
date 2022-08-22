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

use std::{path::PathBuf, time::SystemTime};

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::{DanoError, DanoResult};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn convert_version(line: &str) -> DanoResult<FileInfo> {
    let root: Value = serde_json::from_str(line)?;
    let value = root
        .get("version")
        .ok_or_else(|| DanoError::new("Could not get version value from JSON."))?
        .to_owned();

    let version_number: usize = serde_json::from_value(value)?;
    let file_info: FileInfo = rewrite_version(version_number, line)?.convert()?;

    Ok(file_info)
}

fn rewrite_version(version_number: usize, line: &str) -> DanoResult<Box<dyn ConvertVersion>> {
    let res = match version_number {
        1usize => FileInfoV1::rewrite(line)?,
        _ => return Err(DanoError::new("No matching legacy version found.").into()),
    };

    Ok(res)
}

pub trait ConvertVersion {
    fn rewrite(line: &str) -> DanoResult<Box<Self>>
    where
        Self: std::marker::Sized;
    fn convert(&self) -> DanoResult<FileInfo>;
}

impl ConvertVersion for FileInfoV1 {
    fn rewrite(line: &str) -> DanoResult<Box<Self>> {
        Self::rewrite(line)
    }
    fn convert(&self) -> DanoResult<FileInfo> {
        self.convert()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileInfoV1 {
    pub version: usize,
    pub path: PathBuf,
    pub metadata: Option<FileMetadataV1>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileMetadataV1 {
    pub hash_algo: Box<str>,
    pub hash_value: u128,
    pub last_written: SystemTime,
    pub modify_time: SystemTime,
}

impl FileInfoV1 {
    fn rewrite(line: &str) -> DanoResult<Box<Self>> {
        let rewrite = line.replace("FileInfo", "FileInfoV1");
        let legacy_file_info: FileInfoV1 = serde_json::from_str(&rewrite)?;
        let boxed = Box::new(legacy_file_info);

        Ok(boxed)
    }
    fn convert(&self) -> DanoResult<FileInfo> {
        let new_metadata = self.metadata.as_ref().map(|metadata| FileMetadata {
            hash_algo: metadata.hash_algo.to_owned(),
            hash_value: metadata.hash_value,
            last_written: metadata.last_written,
            modify_time: metadata.modify_time,
            decoded_stream: false,
        });

        Ok(FileInfo {
            version: self.version,
            path: self.path.to_owned(),
            metadata: new_metadata,
        })
    }
}
