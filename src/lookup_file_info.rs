// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    path::{Path, PathBuf},
    process::Command as ExecProcess,
    thread,
    time::SystemTime,
};

use crossbeam::channel::{Receiver, Sender};
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use which::which;

use crate::util::DanoError;
use crate::{DanoResult, DANO_FILE_INFO_VERSION};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileInfo {
    pub version: usize,
    pub path: PathBuf,
    pub metadata: Option<FileMetadata>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileMetadata {
    pub hash_algo: Box<str>,
    pub hash_value: u128,
    pub last_written: SystemTime,
    pub modify_time: SystemTime,
}

impl FileInfo {
    pub fn send_file_info(path: &Path, tx_item: Sender<FileInfo>) -> DanoResult<()> {
        if let Ok(ffmpeg_command) = which("ffmpeg") {
            exec_ffmpeg(path, &ffmpeg_command, tx_item)
        } else {
            Err(DanoError::new(
                "'ffmpeg' command not found. Make sure the command 'ffmpeg' is in your path.",
            )
            .into())
        }
    }
}

fn exec_ffmpeg(path: &Path, ffmpeg_command: &Path, tx_item: Sender<FileInfo>) -> DanoResult<()> {
    // all snapshots should have the same timestamp
    let timestamp = &SystemTime::now();
    let path_clone = path.to_string_lossy();

    let process_args = vec![
        "-i",
        path_clone.as_ref(),
        "-codec",
        "copy",
        "-f",
        "hash",
        "-hash",
        "murmur3",
        "-",
    ];
    let process_output = ExecProcess::new(ffmpeg_command)
        .args(&process_args)
        .output()?;
    let stdout_string = std::str::from_utf8(&process_output.stdout)?.trim();

    let phantom_file_info = FileInfo {
        path: path.to_owned(),
        version: DANO_FILE_INFO_VERSION,
        metadata: None,
    };

    // stderr_string is a string not an error, so here we build an err or output
    if stdout_string.is_empty() {
        // ffmpeg won't work with a non-media file so it solely
        // prints to stderr here, we want to still print our request
        // and keep moving so we don't return an error
        tx_item.send(phantom_file_info)?;

        Ok(())
    } else {
        let res = match stdout_string.split_once('=') {
            Some((first, last)) => FileInfo {
                path: path.to_owned(),
                version: DANO_FILE_INFO_VERSION,
                metadata: Some(FileMetadata {
                    last_written: timestamp.to_owned(),
                    hash_algo: first.into(),
                    hash_value: { u128::from_str_radix(last, 16)? },
                    modify_time: path.metadata()?.modified()?,
                }),
            },
            None => phantom_file_info,
        };

        tx_item.send(res)?;
        Ok(())
    }
}

pub fn exec_lookup_file_info(
    requested_paths: &[PathBuf],
    thread_pool: ThreadPool,
) -> DanoResult<Receiver<FileInfo>> {
    let (tx_item, rx_item): (Sender<FileInfo>, Receiver<FileInfo>) =
        crossbeam::channel::unbounded();

    let requested_paths_clone = requested_paths.to_owned();

    // exec threads to hash files
    thread::spawn(move || {
        thread_pool.in_place_scope(|file_info_scope| {
            requested_paths_clone.iter().for_each(|path_buf| {
                let tx_item_clone = tx_item.clone();
                file_info_scope.spawn(move |_| {
                    let _ = FileInfo::send_file_info(path_buf, tx_item_clone);
                })
            });
        });
    });

    // implicitly drop tx_item at end of scope, otherwise we will hold onto the ref and loop forever
    // explicit drop is: drop(tx_item);
    Ok(rx_item)
}
