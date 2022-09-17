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
use rug::Integer;
use which::which;

use crate::lookup_file_info::{FileInfo, FileMetadata};
use crate::{Config, DanoError, DanoResult, SelectedStreams, DANO_FILE_INFO_VERSION};

const FLAC_HASH_ALGO: &str = "MD5";
const FLAC_DECODED: bool = true;
const FLAC_SELECTED_STREAMS: SelectedStreams = SelectedStreams::AudioOnly;

pub fn get_info_from_flac_import(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let metaflac_cmd = if let Ok(metaflac_cmd) = which("metaflac") {
        metaflac_cmd
    } else {
        return Err(DanoError::new(
            "'metaflac' command not found. Make sure the command 'metaflac' is in your path.",
        )
        .into());
    };

    let res = config
        .paths
        .par_iter()
        .flat_map(|path| match path.extension() {
            Some(extension) if extension.to_ascii_lowercase() == "flac" => Some(path),
            _ => None,
        })
        .flat_map(|path| {
            import_flac_hash_value(path, &metaflac_cmd).map(|hash_string| (path, hash_string))
        })
        .flat_map(|(path, hash)| generate_flac_file_info(path, &hash))
        .collect();

    Ok(res)
}

fn import_flac_hash_value(path: &Path, metaflac_command: &Path) -> DanoResult<String> {
    // all snapshots should have the same timestamp
    let path_string = path.to_string_lossy();

    let process_args = vec!["--show-md5sum", path_string.as_ref()];

    let process_output = ExecProcess::new(metaflac_command)
        .args(&process_args)
        .output()?;
    let stdout_string = std::str::from_utf8(&process_output.stdout)?.trim();
    let stderr_string = std::str::from_utf8(&process_output.stderr)?.trim();

    if stderr_string.contains("FLAC__METADATA_CHAIN_STATUS_NOT_A_FLAC_FILE") {
        eprintln!("Error: Path given is an invalid FLAC file: {}", path_string);
        std::process::exit(1)
    }

    Ok(stdout_string.to_owned())
}

fn generate_flac_file_info(path: &Path, hash_string: &str) -> DanoResult<FileInfo> {
    Ok(FileInfo {
        path: path.to_owned(),
        version: DANO_FILE_INFO_VERSION,
        metadata: Some(FileMetadata {
            last_written: SystemTime::now(),
            hash_algo: FLAC_HASH_ALGO.into(),
            hash_value: { Integer::from_str_radix(hash_string, 16)? },
            modify_time: path.metadata()?.modified()?,
            selected_streams: FLAC_SELECTED_STREAMS,
            decoded: FLAC_DECODED,
        }),
    })
}
