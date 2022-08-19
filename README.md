# `dano`

[dano](https://github.com/kimono-koans/dano) is a wrapper for `ffmpeg` that checksums the internal file streams of `ffmpeg` compatible media files, and stores them in a format which can be used to verify such checksums later.  This is handy, because, should you choose to change metadata tags, or change file names, the media checksums should remain the same.

## Features

* Non-media path filtering (which can be disabled)
* Highly concurrent hashing (select # of threads)
* Several useful modes: WRITE, TEST, COMPARE, PRINT
* Select from multiple checksum algorithms (default: murmur3, MD5, adler32, CRC32)
* Write to xattrs or to hash file (and always read back and operate on both)

## Why dano? Because FLAC is really clever

To me, first class checksums are one thing that sets the FLAC music format apart.  FLAC supports the writing and checking of the streams held within its container.  When I ask whether the FLAC audio stream has the same checksum as the stream I originally wrote to disk, the `flac` command tells me whether the checksum matches:

```bash
% flac -t 'Link Wray - Rumble! The Best of Link Wray - 01-01 - 02 - The Swag.flac'
Link Wray - Rumble! The Best of Link Wray - 01-01 - 02 - The Swag.flac: ok
```

## Why can't I do that everywhere?

The question is -- why don't we have this functionality for video and other media streams?  The answer is, of course, we do, (because `ffmpeg` is incredible!) we just never use it.  My new CLI app, `dano`, aims to make what `ffmpeg` provides easier to use.

So -- when I ask whether a media stream has the same checksum as when I originally wrote it to disk, `dano` tells me whether the checksum matches:

```bash
% dano -w 'Sample.mkv'
murmur3=2f23cebfe8969a8e11cd3919ce9c9067 : "Sample.mkv"
% dano -t 'Sample.mkv'
"Sample": OK
# Now change our file's name and our checksum still verifies,
# because the checksum is stored in a xattr
% mv 'Sample.mkv' 'test1.mkv'
% dano -t 'test2.mkv'
"test1.mkv": OK
# Now change our file's metadata and *write a new file in a 
# new container* and our checksum is the *same*
% ffmpeg -i 'test1.mkv' -metadata author="Kimono" 'test2.mp4'
% dano -w 'test2.mp4'
murmur3=2f23cebfe8969a8e11cd3919ce9c9067 : "test2.mkv"
```

## Shout outs! Yo, yo, yo!

Inspired by `hashdeep`, `md5tree`, `flac`, and, of course, `ffmpeg`.

## Installation

For now, `dano` depends on `ffmpeg`.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh 
cargo install --git https://github.com/kimono-koans/dano.git 
```