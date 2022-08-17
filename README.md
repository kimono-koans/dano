# `dano`

dano is a wrapper for `ffmpeg` that checksums the internal file streams of certain media files, and stores them in a format which can be used to verify such checksums later.  This is handy, because, should you choose to change metadata tags, or change file names, the media checksums *should* remain the same.

## Why `dano`?

As a ZFS fan, I realize ZFS probably appeals to certain kind of person (like me!).  It really shouldn't be so extraordinary to expect the files we read back to be the same files we wrote to disk.  But we live in a mad, mad world.

And because checksums are so cheap, we should expect them everywhere.  

## Because `flac` is really clever?

To me, this is what makes FLAC so great, and most other media format 2nd rate.  FLAC has first class support for writing and checking checksums of the streams held within a container.

So when I ask whether the FLAC audio stream has the same checksum as when I originally wrote it to disk, the `flac` command tells me whether the checksum matches:

```bash
% flac -t 'Link Wray - Rumble! The Best of Link Wray - 01-01 - 02 - The Swag.flac'
Link Wray - Rumble! The Best of Link Wray - 01-01 - 02 - The Swag.flac: ok
```

## Why can't I do that everywhere?

The question is why don't we have this functionality for video and other media streams?  The answer is, of course, we do, (because `ffmpeg` is incredible!) we just never use it.  My new CLI app, `dano`, aims to make what ffmpeg provides easy to use.

```bash
% dano -w 'Tour de France 2019 - Crashes and crosswinds – EF Gone Racing-9gFhDuOqnRw.mkv'
% dano -t 'Tour de France 2019 - Crashes and crosswinds – EF Gone Racing-9gFhDuOqnRw.mkv'
"Tour de France 2019 - Crashes and crosswinds – EF Gone Racing-9gFhDuOqnRw.mkv": OK
# Now change the file's name and our checksum still verifies
% mv 'Tour de France 2019 - Crashes and crosswinds – EF Gone Racing-9gFhDuOqnRw.mkv' 'test.mkv'
% dano -t 'test.mkv'
"test.mkv": OK
```

## Shout outs!

Inspired by `hashdeep`, `md5tree`, `flac`, and, of course, `ffmpeg`

## Installation

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh 
cargo install --git https://github.com/kimono-koans/dano.git 
```