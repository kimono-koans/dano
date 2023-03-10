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

use std::{path::Path, process::Command as ExecProcess, time::SystemTime};

use rayon::prelude::*;
use which::which;

use crate::config::SelectedStreams;
use crate::lookup::HashValue;
use crate::lookup::{FileInfo, FileMetadata};
use crate::{Config, DanoError, DanoResult, RecordedFileInfo, DANO_FILE_INFO_VERSION};

const FLAC_HASH_ALGO: &str = "MD5";
const FLAC_DECODED: bool = true;
const FLAC_SELECTED_STREAMS: SelectedStreams = SelectedStreams::AudioOnly;

impl RecordedFileInfo {
    pub fn from_flac(config: &Config) -> DanoResult<Vec<FileInfo>> {
        let metaflac_cmd = if let Ok(metaflac_cmd) = which("metaflac") {
            metaflac_cmd
        } else {
            return Err(DanoError::new(
                "'metaflac' command not found. Make sure the command 'metaflac' is in your path.",
            )
            .into());
        };

        config
            .paths
            .par_iter()
            .flat_map(|path| match path.extension() {
                Some(extension) if extension.to_ascii_lowercase() == "flac" => Some(path),
                _ => {
                    eprintln!("Error: {:?} does not have a valid FLAC extension", path);
                    None
                }
            })
            .map(
                |path| match Self::import_flac_hash_value(path, &metaflac_cmd) {
                    Ok(hash_value) => Self::generate_flac_file_info(path, hash_value),
                    Err(err) => Err(err),
                },
            )
            .collect()
    }

    fn import_flac_hash_value(path: &Path, metaflac_command: &Path) -> DanoResult<HashValue> {
        // all snapshots should have the same timestamp
        let path_string = path.to_string_lossy();

        let process_args = vec!["--show-md5sum", path_string.as_ref()];

        let process_output = ExecProcess::new(metaflac_command)
            .args(&process_args)
            .output()?;
        let stdout_string = std::str::from_utf8(&process_output.stdout)?.trim();
        let stderr_string = std::str::from_utf8(&process_output.stderr)?.trim();

        if stderr_string.contains("FLAC__METADATA_CHAIN_STATUS_NOT_A_FLAC_FILE") {
            let msg = format!("Error: Path is not a valid FLAC file: {}", path_string);
            return Err(DanoError::new(&msg).into());
        }

        let hash_value =
            if let Ok(_parsed) = primitive_types::U512::from_str_radix(stdout_string, 16) {
                HashValue {
                    radix: 16,
                    value: stdout_string.trim_start_matches('0').into(),
                }
            } else {
                return Err(DanoError::new("Could not parse integer from ffmpeg output.").into());
            };

        if stdout_string.is_empty() {
            // likely file DNE?, except we have already check when we parsed input files
            // so this is a catch all, here we just bail if we have no explanation to give the user
            let msg = format!(
                "Error: Could not generate hash from FLAC file: {}",
                path_string
            );
            return Err(DanoError::new(&msg).into());
        }

        Ok(hash_value)
    }

    fn generate_flac_file_info(path: &Path, hash_value: HashValue) -> DanoResult<FileInfo> {
        Ok(FileInfo {
            path: path.to_owned(),
            version: DANO_FILE_INFO_VERSION,
            metadata: Some(FileMetadata {
                last_written: SystemTime::now(),
                hash_algo: FLAC_HASH_ALGO.into(),
                hash_value,
                modify_time: path.metadata()?.modified()?,
                selected_streams: FLAC_SELECTED_STREAMS,
                decoded: FLAC_DECODED,
            }),
        })
    }
}
