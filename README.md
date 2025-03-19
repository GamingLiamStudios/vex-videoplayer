# vex-videoplayer

Using the power of FFMPEG, Real-time* video playback directly from the SD.

Currently only supported build platform is Linux or MSYS, as you need to be able to run .sh scripts (ffmpeg moment).
Oh and mind the swears in the code, and any instability too.

## How do I get

To build, you only need a few things

- meson
- make
- ninja
- pkgconfig
- rust
- [cargo-make](https://github.com/sagiegurari/cargo-make)

Then it's as simple as running `cargo make build`, then the program can be uploaded using `cargo v5` (installed by Cargo Make)

## Configuration

To specify the video types you would like to decode (since otherwise the binary would be too big to upload), set the env variable for the formats you would like:

- `ENABLE_AV1=true`
- `ENABLE_H264=true`
- `ENABLE_HEVC=true`
- `ENABLE_VP9=true`

This can be set either by Enviornment Variables, or via `cargo make -e ENABLE_XXX=true -e ENABLE_YYY=true build`.
To configure the file being read for playback, consult the main fn in `src/main.rs`. It should be pretty obvious where it's set from there.

## TODOs

- Allow large binary sizes via clever use of SD & memory copies
- Speed up playback to 60fps (faster scaling)
- Better performance profiling
- ~~Fullscreen?~~ apparently not possible :(
