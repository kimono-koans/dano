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
use crate::{DanoError, DanoResult, SelectedStreams, DANO_FILE_INFO_VERSION};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn convert_version(line: &str) -> DanoResult<FileInfo> {
    let root: Value = serde_json::from_str(line)?;
    let value = root
        .get("version")
        .ok_or_else(|| DanoError::new("Could not get version value from JSON."))?
        .to_owned();

    let version_number: usize = serde_json::from_value(value)?;
    let legacy_version: LegacyVersion = LegacyVersion::number_to_version(version_number)?;
    let file_info = legacy_version.prepare_rewrite(line)?;

    Ok(file_info)
}

enum LegacyVersion {
    Version1,
    Version2,
}

impl LegacyVersion {
    fn number_to_version(version_number: usize) -> DanoResult<LegacyVersion> {
        let res = match version_number {
            1 => LegacyVersion::Version1,
            2 => LegacyVersion::Version2,
            _ => return Err(DanoError::new("Legacy version number is invalid").into()),
        };

        Ok(res)
    }

    fn prepare_rewrite(&self, line: &str) -> DanoResult<FileInfo> {
        let res = match self {
            LegacyVersion::Version1 => {
                FileInfoV1::rewrite(line)?.convert()
            },
            LegacyVersion::Version2 => {
                FileInfoV2::rewrite(line)?.convert()
            },
        };

        res
    }
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

impl ConvertVersion for FileInfoV2 {
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
            hash_value: rug::Integer::from(metadata.hash_value),
            last_written: metadata.last_written,
            modify_time: metadata.modify_time,
            decoded: false,
            selected_streams: crate::SelectedStreams::All,
        });

        Ok(FileInfo {
            version: DANO_FILE_INFO_VERSION,
            path: self.path.to_owned(),
            metadata: new_metadata,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileInfoV2 {
    pub version: usize,
    pub path: PathBuf,
    pub metadata: Option<FileMetadataV2>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileMetadataV2 {
    pub hash_algo: Box<str>,
    pub hash_value: rug::Integer,
    pub last_written: SystemTime,
    pub modify_time: SystemTime,
    pub decoded: bool,
}

impl FileInfoV2 {
    fn rewrite(line: &str) -> DanoResult<Box<Self>> {
        let rewrite = line.replace("FileInfo", "FileInfoV2");
        let legacy_file_info: FileInfoV2 = serde_json::from_str(&rewrite)?;
        let boxed = Box::new(legacy_file_info);

        Ok(boxed)
    }
    fn convert(&self) -> DanoResult<FileInfo> {
        let new_metadata = self.metadata.as_ref().map(|metadata| FileMetadata {
            hash_algo: metadata.hash_algo.to_owned(),
            hash_value: metadata.hash_value.to_owned(),
            last_written: metadata.last_written,
            modify_time: metadata.modify_time,
            decoded: metadata.decoded,
            selected_streams: SelectedStreams::All,
        });

        Ok(FileInfo {
            version: DANO_FILE_INFO_VERSION,
            path: self.path.to_owned(),
            metadata: new_metadata,
        })
    }
}
