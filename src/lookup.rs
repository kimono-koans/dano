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
    ops::Deref,
    path::{Path, PathBuf},
    process::Command as ExecProcess,
    sync::Arc,
    time::SystemTime,
};

use crossbeam::channel::{Receiver, Sender};
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use which::which;

use crate::config::SelectedStreams;
use crate::requests::{FileInfoRequest, RequestBundle};
use crate::utility::DanoError;
use crate::{Config, DanoResult, DANO_FILE_INFO_VERSION};

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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HashValue {
    pub radix: i32,
    pub value: Box<str>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub hash_algo: Box<str>,
    pub hash_value: HashValue,
    pub last_written: SystemTime,
    pub modify_time: SystemTime,
    pub decoded: bool,
    pub selected_streams: SelectedStreams,
}

impl FileInfo {
    pub fn generate(
        config: Arc<Config>,
        request: &FileInfoRequest,
        tx_item: Sender<FileInfo>,
    ) -> DanoResult<()> {
        if let Ok(ffmpeg_command) = which("ffmpeg") {
            let decoded = match request.decoded {
                Some(decoded) => decoded,
                None => config.opt_decode,
            };
            let stdout_string =
                FileInfo::get_hash_value(&config, request, &ffmpeg_command, decoded)?;
            FileInfo::transmit_file_info(
                request,
                &stdout_string,
                tx_item,
                decoded,
                &config.selected_streams,
            )
        } else {
            Err(DanoError::new(
                "'ffmpeg' command not found. Make sure the command 'ffmpeg' is in your path.",
            )
            .into())
        }
    }

    fn get_hash_value(
        config: &Config,
        request: &FileInfoRequest,
        ffmpeg_command: &Path,
        decoded: bool,
    ) -> DanoResult<Box<str>> {
        // all snapshots should have the same timestamp
        let path_string = request.path.to_string_lossy();
        let hash_algo = match &request.hash_algo {
            Some(hash_algo) => hash_algo,
            None => &config.selected_hash_algo,
        };
        let selected_streams = match &request.selected_streams {
            Some(selected_streams) => selected_streams,
            None => &config.selected_streams,
        };

        let opt_selected_streams_str = match selected_streams {
            SelectedStreams::All => None,
            SelectedStreams::AudioOnly => Some("0:a?"),
            SelectedStreams::VideoOnly => Some("0:v?"),
        };

        let process_args = FileInfo::build_process_args(
            &path_string,
            hash_algo,
            decoded,
            opt_selected_streams_str,
        );

        let process_output = ExecProcess::new(ffmpeg_command)
            .args(&process_args)
            .output()?;

        let stdout_string = std::str::from_utf8(&process_output.stdout)?.trim();

        match std::str::from_utf8(&process_output.stderr) {
            Ok(stderr_string) if stderr_string.trim().contains("incorrect codec parameters") => {
                let msg = format!(
                    "Error: Invalid hash algorithm specified.  \
                This version of ffmpeg does not support: {} .  \
                Upgrade or specify another hash algorithm.",
                    config.selected_hash_algo
                );
                return Err(DanoError::new(&msg).into());
            }
            // ffmpeg stderr can produce invalid UTF8 sequences which cause dano to
            // error out in a very confusing way.  Better just to flatten that error here.
            _ => (),
        };

        Ok(stdout_string.into())
    }

    fn transmit_file_info(
        request: &FileInfoRequest,
        stdout_string: &str,
        tx_item: Sender<FileInfo>,
        decoded: bool,
        selected_streams: &SelectedStreams,
    ) -> DanoResult<()> {
        let timestamp = SystemTime::now();

        if request.path.to_str().is_none() {
            let msg = format!("Requested path failed UTF validation: {:?}", request.path);
            return Err(DanoError::new(&msg).into());
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
                Some((first, last)) => {
                    let hash_value = if let Ok(_parsed) =
                        primitive_types::U256::from_str_radix(last, 16)
                    {
                        HashValue {
                            radix: 16,
                            value: last.into(),
                        }
                    } else {
                        return Err(
                            DanoError::new("Could not parse integer from ffmpeg output.").into(),
                        );
                    };

                    FileInfo {
                        path: request.path.to_owned(),
                        version: DANO_FILE_INFO_VERSION,
                        metadata: Some(FileMetadata {
                            last_written: timestamp,
                            hash_algo: first.into(),
                            hash_value,
                            modify_time: request.path.metadata()?.modified()?,
                            selected_streams: selected_streams.to_owned(),
                            decoded,
                        }),
                    }
                }
                None => phantom_file_info,
            };

            tx_item.send(res)?;
            Ok(())
        }
    }

    fn build_process_args<'a>(
        path_string: &'a str,
        hash_algo: &'a str,
        decoded: bool,
        opt_selected_streams_str: Option<&'a str>,
    ) -> Vec<&'a str> {
        let mut process_args = vec!["-i", path_string];

        let end_opts = vec!["-f", "hash", "-hash", hash_algo, "-"];

        let codec_copy = vec!["-codec", "copy"];

        if let Some(selected_streams_str) = opt_selected_streams_str {
            process_args.push("-map");
            process_args.push(selected_streams_str);
        }

        if !decoded {
            process_args.extend(codec_copy);
        };

        process_args.extend(end_opts);

        process_args
    }
}

pub struct FileInfoLookup;

impl FileInfoLookup {
    pub fn exec(
        config: &Config,
        requested_paths: &RequestBundle,
        thread_pool: ThreadPool,
    ) -> DanoResult<Receiver<FileInfo>> {
        let (tx_item, rx_item): (Sender<FileInfo>, Receiver<FileInfo>) =
            crossbeam::channel::unbounded();

        let requested_paths_clone = requested_paths.deref().to_owned();

        let config_arc = Arc::new(config.clone());

        std::thread::spawn(move || {
            // exec threads to hash files
            thread_pool.in_place_scope(|file_info_scope| {
                requested_paths_clone.iter().for_each(|request| {
                    let tx_item_clone = tx_item.clone();
                    let config_clone = config_arc.clone();
                    file_info_scope.spawn(move |_| {
                        if let Err(err) = FileInfo::generate(config_clone, request, tx_item_clone) {
                            // probably want to see the error, but not exit the process
                            // when there is an error in a single thread
                            eprintln!("Error: {}", err);
                        }
                    })
                });
            });
        });

        // implicitly drop tx_item at end of scope, otherwise we will hold onto the ref and loop forever
        // explicit drop is: drop(tx_item);
        Ok(rx_item)
    }
}