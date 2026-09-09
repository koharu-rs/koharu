use std::ffi::c_int;

#[repr(C)]
pub struct MallInfo {
    arena: c_int,
    ordblks: c_int,
    smblks: c_int,
    hblks: c_int,
    hblkhd: c_int,
    usmblks: c_int,
    fsmblks: c_int,
    uordblks: c_int,
    fordblks: c_int,
    keepcost: c_int,
}

#[repr(C)]
struct MallInfo2 {
    arena: usize,
    ordblks: usize,
    smblks: usize,
    hblks: usize,
    hblkhd: usize,
    usmblks: usize,
    fsmblks: usize,
    uordblks: usize,
    fordblks: usize,
    keepcost: usize,
}

unsafe extern "C" {
    fn mallinfo2() -> MallInfo2;
}

const FIELD_LIMIT: usize = (c_int::MAX / 4) as usize;

fn clamp(value: usize) -> c_int {
    value.min(FIELD_LIMIT) as c_int
}

// Official CEF Linux builds call glibc's legacy 32-bit mallinfo(). Exporting
// the symbol from the executable keeps CEF's diagnostic field arithmetic from
// overflowing once Koharu's in-process ML heap grows beyond 2 GiB.
#[unsafe(no_mangle)]
pub extern "C" fn mallinfo() -> MallInfo {
    // SAFETY: mallinfo2 takes no arguments and returns its statistics by value.
    let info = unsafe { mallinfo2() };
    MallInfo {
        arena: clamp(info.arena),
        ordblks: clamp(info.ordblks),
        smblks: clamp(info.smblks),
        hblks: clamp(info.hblks),
        hblkhd: clamp(info.hblkhd),
        usmblks: clamp(info.usmblks),
        fsmblks: clamp(info.fsmblks),
        uordblks: clamp(info.uordblks),
        fordblks: clamp(info.fordblks),
        keepcost: clamp(info.keepcost),
    }
}
