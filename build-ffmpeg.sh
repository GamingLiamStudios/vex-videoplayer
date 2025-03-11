#!/bin/sh

cd ffmpeg
rm -r ../ffmpeg-build || true
mkdir ../ffmpeg-build || true
export CFLAGS="-fpermissive -static"
./configure --disable-runtime-cpudetect --disable-autodetect --enable-gpl --enable-nonfree --prefix="../ffmpeg-build" \
    --enable-gray --enable-avcodec --enable-avformat --enable-pixelutils --enable-swscale \
    --disable-swresample --disable-avdevice --disable-avfilter --disable-postproc --disable-programs --disable-protocols --disable-doc \
    --arch=arm --cpu=cortex-a9 --target-os=none --extra-libs="-lgcc -lnosys -lc -lm -lrdimon" --sysroot="/usr/arm-none-eabi" --enable-cross-compile --cross-prefix=arm-none-eabi- \
    --enable-neon --enable-thumb --enable-optimizations --disable-safe-bitstream-reader --enable-hardcoded-tables --malloc-prefix=vexide_ \
    --disable-everything --enable-filter=color --enable-filter=scale \
    --enable-demuxer=matroska --enable-demuxer=ogg --enable-demuxer=mpegts --enable-demuxer=mpegps --enable-demuxer=mpegvideo \
    --enable-parser=av1 --enable-parser=h264 --enable-parser=hevc --enable-parser=vp8 --enable-parser=vp9 \
    --enable-decoder=av1 --enable-decoder=hevc --enable-decoder=h264 --enable-decoder=vp8 --enable-decoder=vp9 \
    --enable-bsf=h264_metadata --enable-bsf=hevc_metadata --enable-bsf=av1_metadata #--enable-demuxers
make -j12
make install
