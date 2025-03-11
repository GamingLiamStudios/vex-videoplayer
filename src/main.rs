#![feature(c_size_t)]
#![no_main]
#![no_std]

extern crate alloc;
use alloc::{borrow::ToOwned, boxed::Box, string::String, vec};
use core::{
    ffi::{CStr, c_int, c_long, c_size_t, c_void},
    pin::Pin,
    time::Duration,
};

use rgb::FromSlice;
use vexide::{devices::display::Rect, fs::File, prelude::*, sync::LazyLock, time::Instant};

#[allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    improper_ctypes,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]
pub mod ffmpeg {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

mod ffmpeg_alloc {
    use alloc::collections::BTreeMap;
    use core::{
        alloc::Layout,
        cell::UnsafeCell,
        ffi::{c_int, c_size_t, c_void},
    };

    use vexide::io::println;

    struct AllocTracker(UnsafeCell<BTreeMap<*mut c_void, Layout>>);
    static ALLOCATED: AllocTracker = AllocTracker(UnsafeCell::new(BTreeMap::new()));
    unsafe impl Send for AllocTracker {}
    unsafe impl Sync for AllocTracker {}

    /// # Safety
    /// Panics on Out of Memory
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn vexide_malloc(size: c_size_t) -> *mut c_void {
        unsafe { vexide_memalign(1, size) }
    }

    /// # Safety
    /// Panics on Out of Memory
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn vexide_realloc(ptr: *mut c_void, size: c_size_t) -> *mut c_void {
        unsafe {
            match (*ALLOCATED.0.get()).get(&ptr) {
                Some(layout) => {
                    // Existing alloc; Let's realloc and move data
                    if size == 0 {
                        vexide_free(ptr);
                        return core::ptr::null_mut();
                    }

                    let new_ptr = alloc::alloc::realloc(ptr.cast(), *layout, size).cast();
                    (*ALLOCATED.0.get()).remove(&ptr);
                    (*ALLOCATED.0.get()).insert(
                        new_ptr,
                        Layout::from_size_align_unchecked(size, layout.align()),
                    );

                    new_ptr
                }
                None => {
                    // New allocation
                    vexide_memalign(1, size)
                }
            }
        }
    }

    /// # Safety
    /// Only deallocs pointer we know
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn vexide_free(ptr: *mut c_void) {
        if ptr.is_null() {
            return; // Early exit for nullptr
        }

        unsafe {
            let Some(layout) = (*ALLOCATED.0.get()).remove(&ptr) else {
                vexide::io::println!("Double free at {ptr:?}");
                return;
            };
            alloc::alloc::dealloc(ptr.cast(), layout);
        }
    }

    /// # Safety
    /// Panics on Out of Memory
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn vexide_memalign(align: c_size_t, size: c_size_t) -> *mut c_void {
        let layout = Layout::from_size_align(size, align).expect("Invalid mem layout");
        unsafe {
            let ptr: *mut c_void = alloc::alloc::alloc(layout).cast();
            if ptr.is_null() {
                alloc::alloc::handle_alloc_error(layout);
            }

            (*ALLOCATED.0.get()).insert(ptr, layout);
            ptr
        }
    }
    /// # Safety
    /// Just is
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn vexide_posix_memalign(
        ptr: *mut *mut c_void,
        align: c_size_t,
        size: c_size_t,
    ) -> c_int {
        let Ok(layout) = Layout::from_size_align(size, align) else {
            return 22; // EINVAL
        };

        unsafe {
            let alloc_ptr: *mut c_void = alloc::alloc::alloc(layout).cast();
            if alloc_ptr.is_null() {
                12 // ENOMEM
            } else {
                (*ALLOCATED.0.get()).insert(alloc_ptr, layout);
                ptr.write(alloc_ptr);
                0
            }
        }
    }
}

struct Robot {}

impl Compete for Robot {
    async fn autonomous(&mut self) {
        println!("Autonomous!");
    }

    async fn driver(&mut self) {
        println!("Driver!");
    }
}

const AVERROR_EOF: i32 =
    -(((b'E' as u32) | (b'O' as u32) << 8 | (b'F' as u32) << 16 | (b' ' as u32) << 24) as i32);

#[unsafe(no_mangle)]
unsafe extern "C" fn vexide_file_read(context: *mut c_void, ptr: *mut u8, size: i32) -> i32 {
    let mut file: Box<File> = unsafe { Box::from_raw(context.cast()) };

    // Read from current position into
    let buf = core::ptr::slice_from_raw_parts_mut(ptr, size as usize);
    let read = unsafe { file.read(&mut *buf).expect("Failed to read file") };

    core::mem::forget(file); // Don't drop the file
    if read == 0 && size != 0 {
        AVERROR_EOF
    } else {
        read as i32
    }
}

unsafe extern "C" fn vexide_file_seek(context: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let mut file: Box<File> = unsafe { Box::from_raw(context.cast()) };

    const SEEK_SET: c_int = 0;
    const SEEK_CUR: c_int = 1;
    const SEEK_END: c_int = 2;
    const SEEK_SIZE: c_int = ffmpeg::AVSEEK_SIZE as c_int;

    let offset = match whence {
        SEEK_SET => vexide::io::SeekFrom::Start(offset as u64),
        SEEK_CUR => vexide::io::SeekFrom::Current(offset),
        SEEK_END => vexide::io::SeekFrom::End(offset),
        SEEK_SIZE => {
            let size = file
                .metadata()
                .expect("shitface")
                .len()
                .expect("missing length");
            core::mem::forget(file);
            return size as i64;
        }
        whence => panic!("Invalid seek position {whence}"),
    };

    let new_offset = file.seek(offset).expect("shitface");

    core::mem::forget(file);
    new_offset as i64
}

#[unsafe(no_mangle)]
extern "C" fn __paritysi2(mut x: c_int) -> c_int {
    x ^= x >> 16;
    x ^= x >> 8;
    x ^= x >> 4;
    (0x6996 >> (x & 0xF)) & 1
}

// These stubs are fine... probably
#[unsafe(no_mangle)]
extern "C" fn _kill() {
    println!("Kill!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _write(file: c_int, buf: *const u8, len: c_size_t) -> c_int {
    match file {
        1 | 2 => {
            // Stdout/err
            unsafe {
                let os_str = core::slice::from_raw_parts(buf, len);
                let str = String::from_utf8_lossy(os_str);
                print!("{str:?}");
            }

            len as c_int
        }
        _ => unimplemented!(),
    }
}

#[unsafe(no_mangle)]
extern "C" fn _read() {
    println!("Read!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _getpid() {
    println!("GetPID!");
    unimplemented!();
}

struct SbrkInfo {
    allocated: Pin<Box<[u8]>>,
    end: isize,
}

static mut SBRK_BLOCK: LazyLock<SbrkInfo> = LazyLock::new(|| SbrkInfo {
    allocated: Box::into_pin(vec![0u8; 1024 * 8].into_boxed_slice()),
    end: 0,
});

#[allow(static_mut_refs)] // :D
#[unsafe(no_mangle)]
extern "C" fn _sbrk(incr: c_int) -> ffmpeg::caddr_t {
    println!("Sbrk {incr}");

    unsafe {
        let ptr = SBRK_BLOCK.allocated.as_ptr();

        let start = ptr.offset(SBRK_BLOCK.end);
        SBRK_BLOCK.end += incr as isize;

        start.cast_mut()
    }
}

#[unsafe(no_mangle)]
extern "C" fn _exit() {
    println!("Exit!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn sysconf(name: c_int) -> c_long {
    println!("sysconf {name}");
    if name == 8 {
        return 4096;
    }
    -1
}

#[unsafe(no_mangle)]
extern "C" fn _fstat() {
    println!("fstat!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _isatty() -> c_int {
    println!("isatty!");
    1
}

#[unsafe(no_mangle)]
extern "C" fn _open() {
    println!("Open!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _stat() {
    println!("Stat!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _times() {
    println!("Times!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn usleep() {
    println!("usleep!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _gettimeofday() {
    println!("GetTimeOfDay!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn mkdir() {
    println!("mkdir!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _close() {
    println!("Close!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _lseek() {
    println!("Seek!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _init() {
    println!("Init!");
}

unsafe extern "C" {
    unsafe fn __libc_init_array();

    #[link_name = "__errno"]
    unsafe fn errno_location() -> *mut c_int;
}

#[vexide::main]
async fn main(mut peripherals: Peripherals) {
    println!("shitface");
    let robot = Robot {};

    unsafe {
        __libc_init_array();
    }

    let video_file = vexide::fs::File::open("video.mkv").expect("shitface");
    println!("Opened file");

    unsafe {
        let mut av_context = ffmpeg::avformat_alloc_context();
        (*av_context).debug = !0;
        println!("AVFormat Alloc");

        let avio_buffer = ffmpeg::av_malloc(1024 * 4).cast(); // 4Kb buffer
        let mut avio_ctx = ffmpeg::avio_alloc_context(
            avio_buffer,
            1024 * 4,
            0,
            Box::into_raw(Box::new(video_file)).cast(),
            Some(vexide_file_read),
            None, // Dont think this needs to be set
            Some(vexide_file_seek),
        );
        (*av_context).pb = avio_ctx;
        println!("AVIO Alloc");

        let result = ffmpeg::avformat_open_input(
            &mut av_context as *mut _,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        );
        if result != 0 {
            let mut str = [0u8; 1024];
            ffmpeg::av_strerror(result, &mut str as *mut _, 1024);
            println!(
                "Failed to open input: {}",
                CStr::from_bytes_until_nul(&str)
                    .expect("shitface")
                    .to_str()
                    .expect("shitface")
            );
            return;
        }
        println!("AVFormat Open Input");

        let result = ffmpeg::avformat_find_stream_info(av_context, core::ptr::null_mut());
        if result < 0 {
            let mut str = [0u8; 1024];
            ffmpeg::av_strerror(result, &mut str as *mut _, 1024);
            println!(
                "Failed to find stream info: {}",
                CStr::from_bytes_until_nul(&str)
                    .expect("shitface")
                    .to_str()
                    .expect("shitface")
            );
            return;
        }
        println!("AVFormat Find Stream Info");

        let mut codec: *const ffmpeg::AVCodec = core::ptr::null();
        let stream_index = ffmpeg::av_find_best_stream(
            av_context,
            ffmpeg::AVMEDIA_TYPE_VIDEO,
            -1,
            -1,
            &mut codec as *mut *const _,
            0,
        );
        if stream_index < 0 {
            let mut str = [0u8; 1024];
            ffmpeg::av_strerror(stream_index, &mut str as *mut _, 1024);
            println!(
                "Failed to find best stream/decoder: {}",
                CStr::from_bytes_until_nul(&str)
                    .expect("shitface")
                    .to_str()
                    .expect("shitface")
            );
            return;
        }
        let stream = *(*av_context).streams.add(stream_index as usize);
        println!("Found best stream+decoder");

        let parser = ffmpeg::av_parser_init((*codec).id as c_int);
        if parser.is_null() {
            println!("Failed to create parser");
            return;
        }

        let codec_ctx = ffmpeg::avcodec_alloc_context3(codec);
        if codec_ctx.is_null() {
            println!("Failed to create codec context");
            return;
        }

        let result = ffmpeg::avcodec_parameters_to_context(codec_ctx, (*stream).codecpar);
        if result < 0 {
            println!("Failed to copy parameters to context");
            return;
        }

        let result = ffmpeg::avcodec_open2(codec_ctx, codec, core::ptr::null_mut());
        if result < 0 {
            println!("Failed to open codec stream");
            return;
        }

        let frame = ffmpeg::av_frame_alloc();
        let packet = ffmpeg::av_packet_alloc();

        peripherals
            .display
            .set_render_mode(vexide::devices::display::RenderMode::Immediate);
        println!("Ready to render");

        let scaled = ffmpeg::av_frame_alloc();
        (*scaled).format = ffmpeg::AV_PIX_FMT_RGB24;
        (*scaled).width = Display::HORIZONTAL_RESOLUTION as i32;
        (*scaled).height = Display::VERTICAL_RESOLUTION as i32;
        ffmpeg::av_frame_get_buffer(scaled, 0);

        println!("Here?");

        let start = Instant::now();

        while ffmpeg::av_read_frame(av_context, packet) >= 0 {
            let result = ffmpeg::avcodec_send_packet(codec_ctx, packet);
            if result < 0 {
                let mut str = [0u8; 1024];
                ffmpeg::av_strerror(result, &mut str as *mut _, 1024);
                println!(
                    "Failed to decode packet: {}",
                    CStr::from_bytes_until_nul(&str)
                        .expect("shitface")
                        .to_str()
                        .expect("shitface")
                );
                return;
            }

            loop {
                let begin = Instant::now();
                const AVERROR_EAGAIN: i32 = -(ffmpeg::EAGAIN as i32);
                match ffmpeg::avcodec_receive_frame(codec_ctx, frame) {
                    0.. => (),
                    AVERROR_EOF | AVERROR_EAGAIN => {
                        break;
                    }
                    result => {
                        let mut str = [0u8; 1024];
                        ffmpeg::av_strerror(result, &mut str as *mut _, 1024);
                        println!(
                            "Failed to decode packet: {}",
                            CStr::from_bytes_until_nul(&str)
                                .expect("shitface")
                                .to_str()
                                .expect("shitface")
                        );
                        return;
                    }
                }

                // Rescale to brain size + color format

                let scale_context = ffmpeg::sws_getContext(
                    (*frame).width,
                    (*frame).height,
                    (*frame).format,
                    (*scaled).width,
                    (*scaled).height,
                    (*scaled).format,
                    ffmpeg::SWS_BILINEAR as i32,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
                ffmpeg::sws_scale(
                    scale_context,
                    &(*frame).data as *const *mut _ as *const *const _,
                    &(*frame).linesize as *const _,
                    0,
                    (*frame).height,
                    &(*scaled).data as *const *mut _,
                    &(*scaled).linesize as *const _,
                );
                ffmpeg::sws_freeContext(scale_context);

                let raw_pixels = core::slice::from_raw_parts(
                    (*scaled).data[0],
                    (*scaled).linesize[0] as usize * (*scaled).height as usize,
                )
                .as_rgb();

                let fract = (*stream).time_base.num as f64 / (*stream).time_base.den as f64;
                let pres = (*packet).pts as f64 * fract;
                println!("took {:?} to render", begin.elapsed());
                println!(
                    "current time: {}, presenting at: {pres}",
                    start.elapsed().as_secs_f64()
                );
                sleep_until(start + Duration::from_secs_f64(pres)).await;

                peripherals.display.draw_buffer(
                    Rect::from_dimensions(
                        [0, 0],
                        Display::HORIZONTAL_RESOLUTION as u16,
                        Display::VERTICAL_RESOLUTION as u16,
                    ),
                    raw_pixels.iter().map(|pixel| pixel.to_owned()),
                    (*scaled).linesize[0] / 3,
                );

                //peripherals.display.draw_buffer(region, buf, src_stride);

                ffmpeg::av_frame_unref(frame);
            }

            ffmpeg::av_packet_unref(packet);
        }

        ffmpeg::avformat_close_input(&mut av_context as *mut _);

        let _: Box<File> = Box::from_raw((*avio_ctx).opaque.cast());
        ffmpeg::av_freep(avio_buffer.cast());
        ffmpeg::avio_context_free(&mut avio_ctx as *mut _);

        ffmpeg::avformat_free_context(av_context);
    }
}
