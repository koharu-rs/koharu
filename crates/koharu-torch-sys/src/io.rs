use std::{
    ffi::c_void,
    io::{Read, Seek, SeekFrom, Write},
    sync::Once,
};

pub trait ReadStream: Read + Seek {}

impl<T: Read + Seek> ReadStream for T {}

static REGISTER_CALLBACKS: Once = Once::new();

pub fn register_callbacks() {
    REGISTER_CALLBACKS.call_once(|| unsafe {
        crate::at_set_stream_callbacks(
            Some(write_stream_destructor),
            Some(write_stream_write),
            Some(read_stream_destructor),
            Some(read_stream_position),
            Some(read_stream_seek_start),
            Some(read_stream_seek_end),
            Some(read_stream_read),
        )
    });
}

extern "C" fn write_stream_destructor(stream_ptr: *mut c_void) {
    unsafe {
        drop(Box::<Box<dyn Write>>::from_raw(
            stream_ptr.cast::<Box<dyn Write>>(),
        ));
    }
}

extern "C" fn write_stream_write(
    stream_ptr: *mut c_void,
    buf: *const u8,
    size: usize,
    out_size: *mut usize,
) -> bool {
    unsafe {
        let stream = &mut *stream_ptr.cast::<Box<dyn Write>>();
        let buffer = std::slice::from_raw_parts(buf, size);
        match stream.write(buffer) {
            Ok(size) => {
                *out_size = size;
                true
            }
            Err(_) => false,
        }
    }
}

extern "C" fn read_stream_destructor(stream_ptr: *mut c_void) {
    unsafe {
        drop(Box::<Box<dyn ReadStream>>::from_raw(
            stream_ptr.cast::<Box<dyn ReadStream>>(),
        ));
    }
}

extern "C" fn read_stream_position(stream_ptr: *mut c_void, position: *mut u64) -> bool {
    unsafe {
        let stream = &mut *stream_ptr.cast::<Box<dyn ReadStream>>();
        match stream.stream_position() {
            Ok(current) => {
                *position = current;
                true
            }
            Err(_) => false,
        }
    }
}

extern "C" fn read_stream_seek_start(
    stream_ptr: *mut c_void,
    position: u64,
    new_position: *mut u64,
) -> bool {
    seek(stream_ptr, SeekFrom::Start(position), new_position)
}

extern "C" fn read_stream_seek_end(
    stream_ptr: *mut c_void,
    position: i64,
    new_position: *mut u64,
) -> bool {
    seek(stream_ptr, SeekFrom::End(position), new_position)
}

fn seek(stream_ptr: *mut c_void, position: SeekFrom, new_position: *mut u64) -> bool {
    unsafe {
        let stream = &mut *stream_ptr.cast::<Box<dyn ReadStream>>();
        match stream.seek(position) {
            Ok(position) => {
                *new_position = position;
                true
            }
            Err(_) => false,
        }
    }
}

extern "C" fn read_stream_read(
    stream_ptr: *mut c_void,
    buf: *mut u8,
    size: usize,
    out_size: *mut usize,
) -> bool {
    unsafe {
        let stream = &mut *stream_ptr.cast::<Box<dyn ReadStream>>();
        let buffer = std::slice::from_raw_parts_mut(buf, size);
        match stream.read(buffer) {
            Ok(size) => {
                *out_size = size;
                true
            }
            Err(_) => false,
        }
    }
}
