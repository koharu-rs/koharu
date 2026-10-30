use koharu_torch_sys::io::{register_callbacks, ReadStream};
use std::{
    ffi::c_void,
    io::{Read, Seek, Write},
};

pub(super) fn read_stream<T: Read + Seek + 'static>(stream: T) -> *mut c_void {
    register_callbacks();
    let stream: Box<Box<dyn ReadStream>> = Box::new(Box::new(stream));
    Box::into_raw(stream).cast()
}

pub(super) fn write_stream<T: Write + 'static>(stream: T) -> *mut c_void {
    register_callbacks();
    let stream: Box<Box<dyn Write>> = Box::new(Box::new(stream));
    Box::into_raw(stream).cast()
}
