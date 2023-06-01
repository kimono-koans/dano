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

use crate::config::SelectedStreams;
use crate::lookup::{FileInfo, FileMetadata, HashValue};
use crate::utility::DanoResult;
use crate::{DanoError, DANO_FILE_INFO_VERSION};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub enum LegacyVersion {
    Version1,
    Version2,
    Version3,
}

impl LegacyVersion {
    pub fn into_latest(line: &str) -> DanoResult<FileInfo> {
        let root: Value = serde_json::from_str(line)?;
        let value = root
            .get("version")
            .ok_or_else(|| DanoError::new("Could not get version value from JSON."))?
            .to_owned();

        let version_number: usize = serde_json::from_value(value)?;
        let legacy_version: LegacyVersion = LegacyVersion::number_to_version(version_number)?;
        let file_info = legacy_version.convert(line)?;

        Ok(file_info)
    }

    fn number_to_version(version_number: usize) -> DanoResult<LegacyVersion> {
        let res = match version_number {
            1 => LegacyVersion::Version1,
            2 => LegacyVersion::Version2,
            3 => LegacyVersion::Version3,
            _ => return Err(DanoError::new("Legacy version number is invalid").into()),
        };

        Ok(res)
    }

    fn convert(&self, line: &str) -> DanoResult<FileInfo> {
        match self {
            LegacyVersion::Version1 => FileInfoV1::from_recorded(line)?.convert(),
            LegacyVersion::Version2 => FileInfoV2::from_recorded(line)?.convert(),
            LegacyVersion::Version3 => FileInfoV3::from_recorded(line)?.convert(),
        }
    }
}

pub trait ConvertVersion {
    fn from_recorded(line: &str) -> DanoResult<Self>
    where
        Self: std::marker::Sized;
    fn convert(&self) -> DanoResult<FileInfo>;
}

impl ConvertVersion for FileInfoV1 {
    fn from_recorded(line: &str) -> DanoResult<Self> {
        Self::from_recorded(line)
    }
    fn convert(&self) -> DanoResult<FileInfo> {
        self.convert()
    }
}

impl ConvertVersion for FileInfoV2 {
    fn from_recorded(line: &str) -> DanoResult<Self> {
        Self::from_recorded(line)
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
    fn from_recorded(line: &str) -> DanoResult<Self> {
        let rewrite = line.replace("FileInfo", "FileInfoV1");
        let legacy_file_info: FileInfoV1 = serde_json::from_str(&rewrite)?;

        Ok(legacy_file_info)
    }
    fn convert(&self) -> DanoResult<FileInfo> {
        let new_metadata = self.metadata.as_ref().map(|metadata| FileMetadata {
            hash_algo: metadata.hash_algo.to_owned(),
            hash_value: HashValue {
                radix: 16,
                value: format!("{:x}", metadata.hash_value).into(),
            },
            last_written: metadata.last_written,
            modify_time: metadata.modify_time,
            decoded: false,
            selected_streams: SelectedStreams::All,
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
    pub hash_value: HashValue,
    pub last_written: SystemTime,
    pub modify_time: SystemTime,
    pub decoded: bool,
}

impl FileInfoV2 {
    fn from_recorded(line: &str) -> DanoResult<Self> {
        let rewrite = line.replace("FileInfo", "FileInfoV2");
        let legacy_file_info: FileInfoV2 = serde_json::from_str(&rewrite)?;

        Ok(legacy_file_info)
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileInfoV3 {
    pub version: usize,
    pub path: PathBuf,
    pub metadata: Option<FileMetadata>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileMetadataV3 {
    pub hash_algo: Box<str>,
    pub hash_value: HashValue,
    pub last_written: SystemTime,
    pub modify_time: SystemTime,
    pub decoded: bool,
    pub selected_streams: SelectedStreams,
}

impl FileInfoV3 {
    fn from_recorded(line: &str) -> DanoResult<Self> {
        let rewrite = line.replace("FileInfo", "FileInfoV3");
        let legacy_file_info: FileInfoV3 = serde_json::from_str(&rewrite)?;

        Ok(legacy_file_info)
    }
    fn convert(&self) -> DanoResult<FileInfo> {
        let new_metadata = self.metadata.as_ref().map(|metadata| FileMetadata {
            hash_algo: metadata.hash_algo.to_owned(),
            hash_value: metadata.hash_value.to_owned(),
            last_written: metadata.last_written,
            modify_time: metadata.modify_time,
            decoded: metadata.decoded,
            selected_streams: metadata.selected_streams.to_owned(),
        });

        Ok(FileInfo {
            version: DANO_FILE_INFO_VERSION,
            path: self.path.to_owned(),
            metadata: new_metadata,
        })
    }
}
