#!/bin/sh

cd ffmpeg
mkdir ../ffmpeg-build || true
export CFLAGS="-fpermissive -static"
./configure --disable-all --disable-runtime-cpudetect --disable-autodetect --enable-gpl --enable-nonfree --prefix="../ffmpeg-build" \
    --enable-gray --enable-small --enable-avcodec --enable-avformat --enable-pixelutils --enable-swscale --enable-avdevice --enable-avfilter --enable-swresample \
    --disable-cuda \
    --arch=arm --cpu=cortex-a9 --target-os=none --extra-libs="-lgcc -lnosys -lc -lm -lrdimon" --enable-cross-compile --cross-prefix=arm-none-eabi- \
    --enable-neon --enable-thumb --enable-lto --enable-pic --enable-optimizations --disable-safe-bitstream-reader --enable-hardcoded-tables \
    --enable-demuxer=matroska --enable-demuxer=ogg --enable-demuxer=mpegts --enable-demuxer=mpegps --enable-demuxer=mpegvideo \
    --enable-parser=av1 --enable-parser=h264 --enable-parser=hevc --enable-parser=vp8 --enable-parser=vp9 \
    --enable-decoder=av1 --enable-decoder=hevc --enable-decoder=h264 --enable-decoder=vp8 --enable-decoder=vp9 \
    --enable-protocol=data --enable-protocol=fd --enable-protocol=file --enable-protocol=cache
make clean
make
make install
