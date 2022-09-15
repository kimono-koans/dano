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
use crate::utility::{deserialize, read_file_info_from_file};
use crate::{Config, DanoError, DanoResult, ExecMode, DANO_FILE_INFO_VERSION, DANO_XATTR_KEY_NAME};

pub fn get_recorded_file_info(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let mut recorded_file_info: Vec<FileInfo> = match &config.exec_mode {
        ExecMode::Write(write_config) if write_config.opt_import_flac => {
            get_info_from_import(config)?
        }
        _ => get_info_from_recorded(config)?,
    };

    // if empty, no valid hashes to test in test mode, and we should quit
    if let ExecMode::Compare(compare_config) = &config.exec_mode {
        if compare_config.opt_test_mode && recorded_file_info.is_empty() {
            return Err(DanoError::new("No valid hashes to test.  Quitting.").into());
        }
    }

    // sort and dedup in case we have paths in both hash file and xattrs
    recorded_file_info.par_sort_unstable_by_key(|file_info| file_info.path.clone());
    recorded_file_info.dedup_by_key(|file_info| file_info.path.clone());

    Ok(recorded_file_info)
}

fn get_info_from_recorded(config: &Config) -> DanoResult<Vec<FileInfo>> {
    let mut file_info_from_xattrs: Vec<FileInfo> = {
        config
            .paths
            .par_iter()
            .flat_map(|path| xattr::get(path, DANO_XATTR_KEY_NAME).map(|opt| (path, opt)))
            .flat_map(|(path, opt)| opt.map(|s| (path, s)))
            .flat_map(|(path, bytes)| std::str::from_utf8(&bytes).map(|i| (path, i.to_owned())))
            .flat_map(|(path, line)| deserialize(&line).map(|i| (path, i)))
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
        read_file_info_from_file(config)?
    } else {
        Vec::new()
    };

    // combine
    file_info_from_xattrs.extend(file_info_from_file);
    Ok(file_info_from_xattrs)
}

fn get_info_from_import(config: &Config) -> DanoResult<Vec<FileInfo>> {
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
            Some(extension) if extension == "flac" || extension == "FLAC" => Some(path),
            _ => None,
        })
        .flat_map(|path| {
            import_hash_value(path, &metaflac_cmd).map(|hash_string| (path, hash_string))
        })
        .flat_map(|(path, hash)| generate_flac_file_info(path, &hash))
        .collect();
    Ok(res)
}

fn import_hash_value(path: &Path, metaflac_command: &Path) -> DanoResult<String> {
    // all snapshots should have the same timestamp
    let path_string = path.to_string_lossy();
    let hash_algo = "MD5";

    let process_args = vec!["--show-md5sum", path_string.as_ref()];

    let process_output = ExecProcess::new(metaflac_command)
        .args(&process_args)
        .output()?;
    let stdout_string = std::str::from_utf8(&process_output.stdout)?.trim();
    let stderr_string = std::str::from_utf8(&process_output.stderr)?.trim();

    if stderr_string.contains("incorrect codec parameters") {
        eprintln!(
            "Error: Invalid hash algorithm specified.  \
        This version of ffmpeg does not support: {} .  \
        Upgrade or specify another hash algorithm.",
            hash_algo
        );
        std::process::exit(1)
    }

    Ok(stdout_string.to_owned())
}

fn generate_flac_file_info(path: &Path, hash_string: &str) -> DanoResult<FileInfo> {
    let timestamp = &SystemTime::now();
    let hash_algo = "MD5".into();
    let decoded = true;

    Ok(FileInfo {
        path: path.to_owned(),
        version: DANO_FILE_INFO_VERSION,
        metadata: Some(FileMetadata {
            last_written: timestamp.to_owned(),
            hash_algo,
            hash_value: { Integer::from_str_radix(hash_string, 16)? },
            modify_time: path.metadata()?.modified()?,
            decoded,
        }),
    })
}
