// (c) Robert Swinford <robert.swinford<...at...>gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.

use std::{
    path::{Path, PathBuf},
    process::Command as ExecProcess,
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use which::which;

use crate::util::{display_output_path, DanoError};
use crate::{Config, DanoResult, ExecMode};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub metadata: Option<FileMetadata>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileMetadata {
    pub hash_algo: Box<str>,
    pub hash_value: u128,
    pub last_checked: SystemTime,
    pub modify_time: SystemTime,
}

impl FileInfo {
    pub fn new(config: &Config, path: &Path) -> DanoResult<Self> {
        fn exec_ffmpeg(
            config: &Config,
            path: &Path,
            ffmpeg_command: &Path,
        ) -> DanoResult<FileInfo> {
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

            // stderr_string is a string not an error, so here we build an err or output
            if stdout_string.is_empty() {
                Err(DanoError::new("Unable to exec ffmpeg").into())
            } else {
                let res = match stdout_string.split_once('=') {
                    Some((first, last)) => FileInfo {
                        path: path.to_owned(),
                        metadata: Some(FileMetadata {
                            last_checked: timestamp.to_owned(),
                            hash_algo: first.into(),
                            hash_value: { u128::from_str_radix(last, 16)? },
                            modify_time: path.metadata()?.modified()?,
                        }),
                    },
                    None => FileInfo {
                        path: path.to_owned(),
                        metadata: None,
                    },
                };

                if config.exec_mode == ExecMode::Write && !config.opt_silent {
                    display_output_path(&res)?;
                }

                Ok(res)
            }
        }

        if let Ok(ffmpeg_command) = which("ffmpeg") {
            exec_ffmpeg(config, path, &ffmpeg_command)
        } else {
            Err(DanoError::new(
                "'ffmpeg' command not found. Make sure the command 'ffmpeg' is in your path.",
            )
            .into())
        }
    }
}
