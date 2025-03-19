#![feature(c_size_t, sync_unsafe_cell, stdarch_arm_neon_intrinsics)]
#![no_main]
#![no_std]

extern crate alloc;
use alloc::{
    borrow::ToOwned,
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::{self, Vec},
};
use core::{
    cell::{SyncUnsafeCell, UnsafeCell},
    ffi::{CStr, c_int, c_long, c_size_t, c_void},
    pin::Pin,
    time::Duration,
    u8,
};

use rgb::{ComponentMap, FromSlice};
use vexide::{
    devices::display::Rect,
    fs::File,
    prelude::*,
    startup::banner::themes::BannerTheme,
    sync::{LazyLock, Mutex},
    time::Instant,
};

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

        //println!("Freed ptr {ptr:?}");
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
            //println!("Allocated {size} at {ptr:?}");

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
extern "C" fn _write(_file: c_int, buf: *const u8, len: c_size_t) -> c_int {
    // Stdout/err
    unsafe {
        let os_str = core::slice::from_raw_parts(buf, len);
        let str = String::from_utf8_lossy(os_str);
        println!("{str:?}");
    }

    len as c_int
}

#[unsafe(no_mangle)]
extern "C" fn _read() {
    println!("Read!");
    unimplemented!();
}

#[unsafe(no_mangle)]
extern "C" fn _getpid() -> c_int {
    println!("GetPID!");
    1
}

struct SbrkInfo {
    allocated: Pin<Box<[u8]>>,
    end: isize,
}

static mut SBRK_BLOCK: LazyLock<SbrkInfo> = LazyLock::new(|| SbrkInfo {
    allocated: Box::into_pin(alloc::vec![0u8; 1024 * 8].into_boxed_slice()),
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

#[repr(C)]
struct Stat {
    st_dev: ffmpeg::dev_t,         /* ID of device containing file */
    st_ino: ffmpeg::ino_t,         /* inode number */
    st_mode: ffmpeg::mode_t,       /* protection */
    st_nlink: ffmpeg::nlink_t,     /* number of hard links */
    st_uid: ffmpeg::uid_t,         /* user ID of owner */
    st_gid: ffmpeg::gid_t,         /* group ID of owner */
    st_rdev: ffmpeg::dev_t,        /* device ID (if special file) */
    st_size: ffmpeg::off_t,        /* total size, in bytes */
    st_blksize: ffmpeg::blksize_t, /* blocksize for file system I/O */
    st_blocks: ffmpeg::blkcnt_t,   /* number of 512B blocks allocated */
    st_atime: ffmpeg::time_t,      /* time of last access */
    st_mtime: ffmpeg::time_t,      /* time of last modification */
    st_ctime: ffmpeg::time_t,      /* time of last status change */
}

#[unsafe(no_mangle)]
extern "C" fn _fstat(_fd: c_int, stat: *mut Stat) -> c_int {
    println!("fstat!");
    unsafe {
        (*stat).st_mode = 8192;
    }
    0
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

struct ThreadReactor {
    active: BTreeMap<ffmpeg::pthread_t, Pin<Box<dyn Future<Output = *mut c_void>>>>,
    next_id: ffmpeg::pthread_t,
}

unsafe impl Sync for ThreadReactor {}

static ACTIVE_THREADS: SyncUnsafeCell<ThreadReactor> = SyncUnsafeCell::new(ThreadReactor {
    active: BTreeMap::new(),
    next_id: 1,
});

#[unsafe(no_mangle)]
extern "C" fn pthread_create(
    pthread: *mut ffmpeg::pthread_t,
    _attr: *const ffmpeg::pthread_attr_t,
    routine: extern "C" fn(*mut c_void) -> *mut c_void,
    arg: *mut c_void,
) -> c_int {
    let future = spawn(async move { routine(arg) });

    unsafe {
        let id = (*ACTIVE_THREADS.get()).next_id;
        (*ACTIVE_THREADS.get()).next_id += 1;
        (*ACTIVE_THREADS.get()).active.insert(id, Box::pin(future));

        pthread.write(id);
    }

    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_join(thread: ffmpeg::pthread_t, value: *mut *mut c_void) -> c_int {
    print!("Join");
    unsafe {
        let future = (*ACTIVE_THREADS.get())
            .active
            .remove(&thread)
            .expect("Thread doesn't exist");

        value.write(block_on(future));
    }

    println!("good");
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_once(
    _once_ctrl: *mut ffmpeg::pthread_once_t,
    routine: extern "C" fn(),
) -> c_int {
    routine();
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_attr_init(attr: *mut ffmpeg::pthread_attr_t) -> c_int {
    unsafe {
        (*attr).is_initialized = 1;
    }
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_attr_destroy(attr: *mut ffmpeg::pthread_attr_t) -> c_int {
    unsafe {
        (*attr).is_initialized = 0;
    }
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_attr_setstacksize(
    attr: *mut ffmpeg::pthread_attr_t,
    stack_size: c_size_t,
) -> c_int {
    unsafe {
        (*attr).stacksize = stack_size as i32;
    }
    0
}

// Mutexes can basically be noops (for now)
#[unsafe(no_mangle)]
extern "C" fn pthread_mutex_init(
    mutex: *mut ffmpeg::pthread_mutex_t,
    attr: *const ffmpeg::pthread_mutexattr_t,
) -> c_int {
    //println!("Mutex Init");
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_mutex_destroy(mutex: *mut ffmpeg::pthread_mutex_t) -> c_int {
    //println!("Mutex Destr");
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_mutex_lock(mutex: *mut ffmpeg::pthread_mutex_t) -> c_int {
    //println!("Mutex Lock");
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_mutex_unlock(mutex: *mut ffmpeg::pthread_mutex_t) -> c_int {
    //println!("Mutex unLock");
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_cond_init(
    cond: *mut ffmpeg::pthread_cond_t,
    attr: *const ffmpeg::pthread_condattr_t,
) -> c_int {
    unimplemented!("pthread_cond_init")
}

#[unsafe(no_mangle)]
extern "C" fn pthread_cond_destroy(cond: *mut ffmpeg::pthread_cond_t) -> c_int {
    unimplemented!("pthread_cond_destroy")
}

#[unsafe(no_mangle)]
extern "C" fn pthread_cond_broadcast(cond: *mut ffmpeg::pthread_cond_t) -> c_int {
    unimplemented!("pthread_cond_broadcast")
}

#[unsafe(no_mangle)]
extern "C" fn pthread_cond_signal(cond: *mut ffmpeg::pthread_cond_t) -> c_int {
    unimplemented!("pthread_cond_signal")
}

#[unsafe(no_mangle)]
extern "C" fn pthread_cond_wait(
    cond: *mut ffmpeg::pthread_cond_t,
    mutex: *mut ffmpeg::pthread_mutex_t,
) -> c_int {
    unimplemented!("pthread_cond_wait")
}

unsafe extern "C" {
    static __heap_start: u8;
    static __heap_end: u8;

    unsafe fn __libc_init_array();

    #[link_name = "__errno"]
    unsafe fn errno_location() -> *mut c_int;
}

#[vexide::main]
async fn main(mut peripherals: Peripherals) {
    println!("shitface");

    unsafe {
        __libc_init_array();
    }

    println!(
        "Usable memory range: {:?}-{:?}",
        core::ptr::addr_of!(__heap_start),
        core::ptr::addr_of!(__heap_end)
    );

    let video_file = vexide::fs::File::open("monogatari.mkv").expect("shitface");
    println!("Opened file");

    unsafe {
        let mut av_context = ffmpeg::avformat_alloc_context();
        (*av_context).debug = !0;
        println!("AVFormat Alloc");

        let avio_buffer = ffmpeg::av_malloc(1024 * 64).cast(); // 64Kb buffer
        let mut avio_ctx = ffmpeg::avio_alloc_context(
            avio_buffer,
            1024 * 64,
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

        /*
        let scaled = ffmpeg::av_frame_alloc();
        (*scaled).format = ffmpeg::AV_PIX_FMT_0RGB;
        (*scaled).width = Display::HORIZONTAL_RESOLUTION as i32;
        (*scaled).height = Display::VERTICAL_RESOLUTION as i32;
        ffmpeg::av_frame_get_buffer(scaled, 0);

        let scale_context = ffmpeg::sws_getContext(
            (*codec_ctx).coded_width,
            (*codec_ctx).coded_height,
            (*(*stream).codecpar).format,
            (*scaled).width,
            (*scaled).height,
            (*scaled).format,
            ffmpeg::SWS_BILINEAR as i32,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        );
        */

        let mut adjusted_frame = alloc::vec![rgb::Bgra::new_bgra(0u8, 0, 0, 0); (*codec_ctx).coded_width as usize * (*codec_ctx).coded_height as usize].into_boxed_slice();
        let mut scaled_frame = alloc::vec![rgb::Bgra::new_bgra(0u8, 0, 0, 0); Display::HORIZONTAL_RESOLUTION as usize * Display::VERTICAL_RESOLUTION as usize].into_boxed_slice();

        let mut last_frame = Instant::now();
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

                println!("Time to decode frame: {:?}", last_frame.elapsed());

                //println!("Decoded packet in {:?}", begin.elapsed());

                // TODO: Colorspace conversion
                // match (*frame).colorspace {
                //     ffmpeg::AVCOL_SPC_RGB => (), // Do nothing; target fmt
                //     space => unimplemented!("Unsupported Colorspace: {space}"),
                // }

                let begin = Instant::now();

                let line_size = (*frame).linesize;
                let width = (*frame).width as usize;
                let height = (*frame).height as usize;

                // TODO: Rescale before Resample (why put in extra work, does make rescaling harder tho)

                // Convert to 0RGB (8bit)
                match (*frame).format {
                    ffmpeg::AV_PIX_FMT_0RGB => (), // Do nothing; target fmt
                    ffmpeg::AV_PIX_FMT_YUV420P => {
                        let luma_stride = line_size[0] as usize;
                        let luma_plane: *const u8 = (*frame).data[0].cast();

                        let blue_stride = line_size[1] as usize;
                        let blue_plane: *const u8 = (*frame).data[1].cast();

                        let red_stride = line_size[2] as usize;
                        let red_plane: *const u8 = (*frame).data[2].cast();

                        // TODO: Fix color fringing

                        // Each CbCr represents a 2x2 field of luma

                        // TODO: Change these constants depending on colorspace
                        const CHROMA_BLUE_WEIGHTS: [i16; 2] = [
                            24,  //-0.1873,
                            238, // 1.8556,
                        ];
                        const CHROMA_RED_WEIGHTS: [i16; 2] = [
                            202, // 1.5748,
                            60,  // -0.4681,
                        ];

                        use core::arch::arm::*;

                        let half = vmovq_n_s16(128);

                        for hy in 0..height / 2 {
                            let scanline_blue = blue_plane.add(hy * blue_stride);
                            let scanline_red = red_plane.add(hy * red_stride);

                            for y in (hy * 2)..(hy + 1) * 2 {
                                let scanline_luma = luma_plane.add(y * luma_stride);

                                for hx in (0..width / 2).step_by(4) {
                                    let chroma_blue = vld1_u8(scanline_blue.add(hx)); // Read 8 chromas
                                    let chroma_blue = vreinterpretq_s16_u16(vmovl_u8(
                                        // Interleave with itself, broadcasting duplicates into adjacent lanes
                                        vzip_u8(chroma_blue, chroma_blue).0, // Downsample to first vector (4 chromas, duplicated adj)
                                    ));
                                    let chroma_blue = vsubq_s16(chroma_blue, half);

                                    // So we're really only using 4 chromas (2x2 block size)
                                    let chroma_red = vld1_u8(scanline_red.add(hx));
                                    let chroma_red = vreinterpretq_s16_u16(vmovl_u8(
                                        vzip_u8(chroma_red, chroma_red).0,
                                    ));
                                    let chroma_red = vsubq_s16(chroma_red, half);

                                    // Load 8x lumas, shift left 7
                                    let luma = vshlq_n_s16::<7>(vreinterpretq_s16_u16(vmovl_u8(
                                        vld1_u8(scanline_luma.add(hx * 2)),
                                    )));

                                    // Assuming BT.709
                                    let red = vaddq_s16(
                                        luma,
                                        vmulq_s16(chroma_red, vmovq_n_s16(CHROMA_RED_WEIGHTS[0])),
                                    );
                                    let green = vsubq_s16(
                                        luma,
                                        vaddq_s16(
                                            vmulq_s16(
                                                chroma_blue,
                                                vmovq_n_s16(CHROMA_BLUE_WEIGHTS[0]),
                                            ),
                                            vmulq_s16(
                                                chroma_red,
                                                vmovq_n_s16(CHROMA_RED_WEIGHTS[1]),
                                            ),
                                        ),
                                    );
                                    let blue = vaddq_s16(
                                        luma,
                                        vmulq_s16(chroma_blue, vmovq_n_s16(CHROMA_BLUE_WEIGHTS[1])),
                                    );

                                    // Store as interleaved rgba
                                    let padding = vmovq_n_s16(64);
                                    vst4_s8(
                                        adjusted_frame.as_mut_ptr().add(y * width + hx * 2).cast(),
                                        int8x8x4_t(
                                            vmovn_s16(vshrq_n_s16::<7>(vaddq_s16(blue, padding))),
                                            vmovn_s16(vshrq_n_s16::<7>(vaddq_s16(green, padding))),
                                            vmovn_s16(vshrq_n_s16::<7>(vaddq_s16(red, padding))),
                                            vcreate_s8(0),
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    format => unimplemented!("Unsupported pixel format: {format}"),
                }

                println!("Took {:?} to Resample to RGB", begin.elapsed());
                let begin = Instant::now();

                // Rescale image
                // TODO: Bilinear/Average(area)
                for y in 0..Display::VERTICAL_RESOLUTION as usize {
                    let sy = y as f32 / f32::from(Display::VERTICAL_RESOLUTION);
                    for x in 0..Display::HORIZONTAL_RESOLUTION as usize {
                        let sx = x as f32 / f32::from(Display::HORIZONTAL_RESOLUTION);

                        let nx = (sx * width as f32) as usize;
                        let ny = (sy * height as f32) as usize;
                        scaled_frame[y * Display::HORIZONTAL_RESOLUTION as usize + x] =
                            adjusted_frame[ny * width + nx];
                    }
                }

                println!("Took {:?} to Rescale", begin.elapsed());

                /*
                // Rescale to brain size + color format
                ffmpeg::sws_scale(
                    scale_context,
                    &(*frame).data as *const *mut _ as *const *const _,
                    &(*frame).linesize as *const _,
                    0,
                    (*frame).height,
                    &(*scaled).data as *const *mut _,
                    &(*scaled).linesize as *const _,
                );

                let raw_pixels = core::slice::from_raw_parts(
                    (*scaled).data[0],
                    (*scaled).linesize[0] as usize * (*scaled).height as usize,
                );
                */

                //let fract = (*stream).time_base.num as f64 / (*stream).time_base.den as f64;
                //let pres = (*packet).pts as f64 * fract;
                //println!(
                //    "current time: {}, presenting at: {pres}",
                //    start.elapsed().as_secs_f64()
                //);
                //sleep_until(start + Duration::from_secs_f64(pres)).await;

                vex_sdk::vexDisplayCopyRect(
                    0,
                    Display::HEADER_HEIGHT as i32,
                    Display::HORIZONTAL_RESOLUTION as i32,
                    Display::VERTICAL_RESOLUTION as i32 + Display::HEADER_HEIGHT as i32,
                    bytemuck::cast_slice::<_, u32>(&scaled_frame)
                        .as_ptr()
                        .cast_mut(),
                    Display::HORIZONTAL_RESOLUTION as i32,
                );

                //peripherals.display.draw_buffer(region, buf, src_stride);
                ffmpeg::av_frame_unref(frame);
                last_frame = Instant::now();
            }

            ffmpeg::av_packet_unref(packet);
        }

        //ffmpeg::sws_freeContext(scale_context);
        ffmpeg::avformat_close_input(&mut av_context as *mut _);

        let _: Box<File> = Box::from_raw((*avio_ctx).opaque.cast());
        ffmpeg::av_freep(avio_buffer.cast());
        ffmpeg::avio_context_free(&mut avio_ctx as *mut _);

        ffmpeg::avformat_free_context(av_context);
    }
}
