# `dano`

[dano](https://github.com/kimono-koans/dano) is a wrapper for `ffmpeg` that checksums the internal file streams of `ffmpeg` compatible media files, and stores them in a format which can be used to verify such checksums later.  This is handy, because, should you choose to change metadata tags, or change file names, the media checksums should remain the same.

## Features

* Non-media path filtering (which can be disabled)
* Highly concurrent hashing (select # of threads)
* Several useful modes: WRITE, TEST, COMPARE, PRINT or DUMP
* Select from multiple checksum algorithms (default: murmur3, MD5, adler32, CRC32, SHA160, SHA256, SHA512)
* Option to decode the stream before executing hash function
* Write to xattrs or to hash file (and always read back and operate on both)

## Why `dano`? Because FLAC is really clever

To me, first class checksums are one thing that sets the FLAC music format apart.  FLAC supports the writing and checking of the streams held within its container.  When I ask whether the FLAC audio stream has the same checksum as the stream I originally wrote to disk, the `flac` command tells me whether the checksum matches:

```bash
% flac -t 'Link Wray - Rumble! The Best of Link Wray - 01-01 - 02 - The Swag.flac'
Link Wray - Rumble! The Best of Link Wray - 01-01 - 02 - The Swag.flac: ok
```

*For lossless files*, this means we can confirm that a lossless file decodes to the same to the exact bit stream we encoded, but, *for all files*, this means our checksums are stable against metadata changes, file name changes, and/or moving a bitstream, or many bitstreams, from one media container to another 

## Why can't I do that everywhere?

The question is -- why don't we have this functionality for video and other media streams?  The answer is, of course, we do, (because `ffmpeg` is incredible!) we just never use it.  `dano` aims to make what `ffmpeg` provides easier to use.

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
## Can I use `dano` with my FLAC files?

Of course you can.  `dano` will even import your FLAC file's checksums directly:

```bash
# Import dano checksum from FLAC and write to an xattr
% dano --import-flac 'Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac'
MD5=fed8052012fb6d0523ef3980a0f6f7bd : "Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac"
Writing dano hash for: "Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac"
No old file data to overwrite.
# Verify checksum is the same as the checksum embedded in the FLAC container
% metaflac --show-md5sum 'Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac'
fed8052012fb6d0523ef3980a0f6f7bd
# Verify the decoded FLAC audio stream is the same as the xattr checksum
% dano -t 'Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac'
"Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac": OK
```

## Ugh, why can't ALAC be more like FLAC?

I get it!  For serious collectors, if you can't verify your checksums later when you change the album art, what use is a checksum?

`dano` allows you have to store a stable checksum, and verify it later, just like FLAC:

```bash
# To test, this we will create an ALAC copy of a FLAC file
ffmpeg -i 'Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac' -acodec alac 'Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.m4a'
# Write dano checksum to an xattr
% dano -w --only=audio --decode --hash-algo=md5 'Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.m4a'
MD5=fed8052012fb6d0523ef3980a0f6f7bd : "Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.m4a"
Writing dano hash for: "Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.m4a"
No old file data to overwrite.
# Verify checksum is the same as the decoded FLAC audio stream
% metaflac --show-md5sum "Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.flac"
fed8052012fb6d0523ef3980a0f6f7bd
# Verify the decoded ALAC audio stream is the same as the xattr checksum
% dano -t "Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.m4a"
"Pavement - Wowee Zowee_ Sordid Sentinels Edition - 02-02 - 50 - We Dance.m4a": OK
```

## Shout outs! Yo, yo, yo!

Inspired by `hashdeep`, `md5tree`, `flac`, and, of course, `ffmpeg`.

## Install via Native Packages

For Debian-based and Redhat-based Linux distributions (like, Ubuntu or Fedora, etc.), check the [tagged releases](https://github.com/kimono-koans/dano/tags) for native packages for your distribution.  

You may also create and install your own native package from the latest sources, like so:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo install cargo-deb 
git clone https://github.com/kimono-koans/dano.git
cd ./dano/; cargo deb
# to install on a Debian/Ubuntu-based system
dpkg -i ./target/debian/dano_*.deb
# or convert to RPM 
alien -r ./target/debian/dano_*.deb
# and install on a Redhat-based system
rpm -i --replacefiles ./dano*.rpm
```

## Installation from Source

For now, `dano` depends on `ffmpeg` and `metaflac` if you want to import FLAC files.

You may install `rustup` and build `dano` like so:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh 
cargo install dano
```

Caveat: `dano` is only tested on MacOS and Linux, and will probably only compile and run on Unix-y Rust supported platforms, but a Windows is version is *likely* to compile with only minor changes.  My further thoughts on a Windows version can be found in this [linked issue](https://github.com/kimono-koans/dano/issues/3).

Note: In addition to what your package manager or OS may provide (for instance, `apt install rustc cargo`, security-minded users may be interested to know that there are [alternative methods](https://rust-lang.github.io/rustup/installation/other.html) for installing the `rustc` compiler and `cargo` besides the method described above, which allow you to verify the `rustup` before install.