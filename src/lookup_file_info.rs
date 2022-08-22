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

use std::{
    cmp::{Ord, Ordering, PartialOrd},
    path::{Path, PathBuf},
    process::Command as ExecProcess,
    sync::Arc,
    thread,
    time::SystemTime,
};

use crossbeam::channel::{Receiver, Sender};
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use which::which;

use crate::{util::DanoError, Config, FileInfoRequest};
use crate::{DanoResult, DANO_FILE_INFO_VERSION};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub version: usize,
    pub path: PathBuf,
    pub metadata: Option<FileMetadata>,
}

impl PartialOrd for FileInfo {
    #[inline]
    fn partial_cmp(&self, other: &FileInfo) -> Option<Ordering> {
        Some(self.path.cmp(&other.path))
    }
}

impl Ord for FileInfo {
    #[inline]
    fn cmp(&self, other: &FileInfo) -> Ordering {
        self.path.cmp(&other.path)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub hash_algo: Box<str>,
    pub hash_value: u128,
    pub last_written: SystemTime,
    pub modify_time: SystemTime,
    pub decoded_stream: bool,
}

impl FileInfo {
    pub fn send_file_info(
        config: Arc<Config>,
        request: &FileInfoRequest,
        tx_item: Sender<FileInfo>,
    ) -> DanoResult<()> {
        if let Ok(ffmpeg_command) = which("ffmpeg") {
            exec_ffmpeg(&config, request, &ffmpeg_command, tx_item)
        } else {
            Err(DanoError::new(
                "'ffmpeg' command not found. Make sure the command 'ffmpeg' is in your path.",
            )
            .into())
        }
    }
}

fn exec_ffmpeg(
    config: &Config,
    request: &FileInfoRequest,
    ffmpeg_command: &Path,
    tx_item: Sender<FileInfo>,
) -> DanoResult<()> {
    // all snapshots should have the same timestamp
    let timestamp = &SystemTime::now();
    let path_string = request.path.to_string_lossy();
    let hash_algo = match &request.hash_algo {
        Some(hash_algo) => hash_algo,
        None => &config.selected_hash_algo,
    };

    let process_args = vec![
        "-i",
        path_string.as_ref(),
        "-codec",
        "copy",
        "-f",
        "hash",
        "-hash",
        hash_algo,
        "-",
    ];

    let process_output = ExecProcess::new(ffmpeg_command)
        .args(&process_args)
        .output()?;
    let stdout_string = std::str::from_utf8(&process_output.stdout)?.trim();
    let stderr_string = std::str::from_utf8(&process_output.stderr)?.trim();

    if stderr_string.contains("incorrect codec parameters") {
        eprintln!(
            "Error: Invalid hash algorithm specified.  \
        This version of ffmpeg does not support: {} .  \
        Upgrade or specify another hash algorithm.",
            config.selected_hash_algo
        );
        std::process::exit(1)
    }

    let phantom_file_info = FileInfo {
        path: request.path.to_owned(),
        version: DANO_FILE_INFO_VERSION,
        metadata: None,
    };

    if stdout_string.is_empty() {
        // if stdout string is empty, then file DNE
        // we want to print the request instead of an error
        // or just continuing so we send the path + dummy value
        tx_item.send(phantom_file_info)?;

        Ok(())
    } else {
        let res = match stdout_string.split_once('=') {
            Some((first, last)) => FileInfo {
                path: request.path.to_owned(),
                version: DANO_FILE_INFO_VERSION,
                metadata: Some(FileMetadata {
                    last_written: timestamp.to_owned(),
                    hash_algo: first.into(),
                    hash_value: { u128::from_str_radix(last, 16)? },
                    modify_time: request.path.metadata()?.modified()?,
                    decoded_stream: false,
                }),
            },
            None => phantom_file_info,
        };

        tx_item.send(res)?;
        Ok(())
    }
}

pub fn exec_lookup_file_info(
    config: &Config,
    requested_paths: &[FileInfoRequest],
    thread_pool: ThreadPool,
) -> DanoResult<Receiver<FileInfo>> {
    let (tx_item, rx_item): (Sender<FileInfo>, Receiver<FileInfo>) =
        crossbeam::channel::unbounded();

    let requested_paths_clone = requested_paths.to_owned();

    let config_arc = Arc::new(config.clone());

    // exec threads to hash files
    thread::spawn(move || {
        thread_pool.in_place_scope(|file_info_scope| {
            requested_paths_clone.iter().for_each(|request| {
                let tx_item_clone = tx_item.clone();
                let config_clone = config_arc.clone();
                file_info_scope.spawn(move |_| {
                    let _ = FileInfo::send_file_info(config_clone, request, tx_item_clone);
                })
            });
        });
    });

    // implicitly drop tx_item at end of scope, otherwise we will hold onto the ref and loop forever
    // explicit drop is: drop(tx_item);
    Ok(rx_item)
}
