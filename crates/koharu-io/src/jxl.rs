//! Safe JPEG XL encoding wrapper isolating all `unsafe` `jxl-sys` FFI.

use std::{ffi::c_int, mem::MaybeUninit, ptr};

use anyhow::{Context as _, Result, bail, ensure};
use jxl_sys::{
    JxlBasicInfo, JxlColorEncoding, JxlColorEncodingSetToSRGB, JxlDataType, JxlEncoder,
    JxlEncoderAddImageFrame, JxlEncoderCloseInput, JxlEncoderCreate, JxlEncoderDestroy,
    JxlEncoderDistanceFromQuality, JxlEncoderFrameSettingId, JxlEncoderFrameSettingsCreate,
    JxlEncoderFrameSettingsSetOption, JxlEncoderInitBasicInfo, JxlEncoderProcessOutput,
    JxlEncoderSetBasicInfo, JxlEncoderSetColorEncoding, JxlEncoderSetFrameDistance,
    JxlEncoderSetFrameLossless, JxlEncoderStatus, JxlEncoderUseContainer, JxlEndianness,
    JxlPixelFormat,
};

pub(crate) enum JxlPixels<'a> {
    L8(&'a [u8]),
    Rgb8(&'a [u8]),
    Rgba8(&'a [u8]),
}

struct Encoder(*mut JxlEncoder);

impl Encoder {
    fn new() -> Result<Self> {
        // SAFETY: `JxlEncoderCreate` accepts a null memory manager and returns either a valid
        // encoder pointer or null on allocation failure.
        let raw = unsafe { JxlEncoderCreate(ptr::null()) };
        if raw.is_null() {
            bail!("failed to allocate JXL encoder");
        }
        Ok(Self(raw))
    }

    fn as_ptr(&self) -> *mut JxlEncoder {
        self.0
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: `self.0` was returned by `JxlEncoderCreate` and has not been destroyed yet.
        unsafe { JxlEncoderDestroy(self.0) };
    }
}

fn check_status(status: JxlEncoderStatus, context: &str) -> Result<()> {
    match status {
        JxlEncoderStatus::JXL_ENC_SUCCESS => Ok(()),
        other => bail!("{context}: JXL encoder status {other:?}"),
    }
}

pub(crate) fn encode(
    pixels: JxlPixels<'_>,
    width: u32,
    height: u32,
    lossless: bool,
    quality: u8,
) -> Result<Vec<u8>> {
    ensure!(
        width > 0 && height > 0,
        "JXL dimensions must be non-zero ({width}x{height})"
    );
    ensure!(
        (1..=100).contains(&quality),
        "export quality {quality} must be between 1 and 100"
    );

    let (raw, channels, has_alpha, is_gray) = match pixels {
        JxlPixels::L8(data) => (data, 1usize, false, true),
        JxlPixels::Rgb8(data) => (data, 3usize, false, false),
        JxlPixels::Rgba8(data) => (data, 4usize, true, false),
    };
    let expected_len = usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(height).ok()?))
        .and_then(|px| px.checked_mul(channels))
        .context("JXL image dimensions overflow address space")?;
    ensure!(
        raw.len() == expected_len,
        "JXL pixel buffer length {} does not match {}x{}x{}",
        raw.len(),
        width,
        height,
        channels
    );

    let encoder = Encoder::new()?;
    let enc = encoder.as_ptr();

    // SAFETY: `enc` is a live `JxlEncoder` owned by `encoder`; `0` is `JXL_FALSE`.
    let status = unsafe { JxlEncoderUseContainer(enc, 0) };
    check_status(status, "JxlEncoderUseContainer")?;

    let mut basic_info = MaybeUninit::<JxlBasicInfo>::uninit();
    // SAFETY: `basic_info` points to valid uninitialized `JxlBasicInfo` storage.
    unsafe { JxlEncoderInitBasicInfo(basic_info.as_mut_ptr()) };
    // SAFETY: `JxlEncoderInitBasicInfo` initializes all fields of `JxlBasicInfo`.
    let mut basic_info = unsafe { basic_info.assume_init() };
    basic_info.xsize = width;
    basic_info.ysize = height;
    basic_info.bits_per_sample = 8;
    basic_info.exponent_bits_per_sample = 0;
    basic_info.num_color_channels = if is_gray { 1 } else { 3 };
    basic_info.num_extra_channels = u32::from(has_alpha);
    basic_info.alpha_bits = if has_alpha { 8 } else { 0 };
    basic_info.uses_original_profile = c_int::from(lossless);

    // SAFETY: `enc` and `basic_info` are valid and initialized.
    let status = unsafe { JxlEncoderSetBasicInfo(enc, &basic_info) };
    check_status(status, "JxlEncoderSetBasicInfo")?;

    let mut color_encoding = MaybeUninit::<JxlColorEncoding>::uninit();
    // SAFETY: `color_encoding` points to valid storage and `c_int::from(is_gray)` is 0 or 1.
    unsafe { JxlColorEncodingSetToSRGB(color_encoding.as_mut_ptr(), c_int::from(is_gray)) };
    // SAFETY: `JxlColorEncodingSetToSRGB` initializes `color_encoding`.
    let color_encoding = unsafe { color_encoding.assume_init() };
    // SAFETY: `enc` and `color_encoding` are valid and initialized.
    let status = unsafe { JxlEncoderSetColorEncoding(enc, &color_encoding) };
    check_status(status, "JxlEncoderSetColorEncoding")?;

    // SAFETY: `enc` is valid and `options_source` is null for fresh defaults.
    let settings = unsafe { JxlEncoderFrameSettingsCreate(enc, ptr::null()) };
    if settings.is_null() {
        bail!("JxlEncoderFrameSettingsCreate returned null");
    }

    // SAFETY: `settings` belongs to `enc` and is non-null.
    let status = unsafe { JxlEncoderSetFrameLossless(settings, c_int::from(lossless)) };
    check_status(status, "JxlEncoderSetFrameLossless")?;

    if !lossless {
        // SAFETY: `JxlEncoderDistanceFromQuality` is a pure function mapping quality 1..=100.
        let distance = unsafe { JxlEncoderDistanceFromQuality(f32::from(quality)) };
        // SAFETY: `settings` is valid and `distance` is finite for quality 1..=100.
        let status = unsafe { JxlEncoderSetFrameDistance(settings, distance) };
        check_status(status, "JxlEncoderSetFrameDistance")?;
    }

    // SAFETY: `settings` is valid; effort 7 is libjxl's default balanced speed/size setting.
    let status = unsafe {
        JxlEncoderFrameSettingsSetOption(
            settings,
            JxlEncoderFrameSettingId::JXL_ENC_FRAME_SETTING_EFFORT,
            7,
        )
    };
    check_status(status, "JxlEncoderFrameSettingsSetOption(effort)")?;

    let pixel_format = JxlPixelFormat {
        num_channels: u32::try_from(channels)?,
        data_type: JxlDataType::JXL_TYPE_UINT8,
        endianness: JxlEndianness::JXL_NATIVE_ENDIAN,
        align: 0,
    };

    // SAFETY: `raw` is a contiguous slice of `raw.len()` bytes matching `pixel_format` and `basic_info`.
    let status =
        unsafe { JxlEncoderAddImageFrame(settings, &pixel_format, raw.as_ptr().cast(), raw.len()) };
    check_status(status, "JxlEncoderAddImageFrame")?;

    // SAFETY: `enc` is valid; no further frames will be added.
    unsafe { JxlEncoderCloseInput(enc) };

    let mut output = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut next_out = chunk.as_mut_ptr();
        let mut avail_out = chunk.len();
        // SAFETY: `next_out` points to `avail_out` writable bytes in `chunk`.
        let status = unsafe { JxlEncoderProcessOutput(enc, &mut next_out, &mut avail_out) };
        let written = chunk.len() - avail_out;
        output.extend_from_slice(&chunk[..written]);
        match status {
            JxlEncoderStatus::JXL_ENC_SUCCESS => break,
            JxlEncoderStatus::JXL_ENC_NEED_MORE_OUTPUT => continue,
            other => bail!("JxlEncoderProcessOutput failed: {other:?}"),
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use jxl_sys::{
        JxlBasicInfo, JxlDecoder, JxlDecoderCreate, JxlDecoderDestroy, JxlDecoderGetBasicInfo,
        JxlDecoderImageOutBufferSize, JxlDecoderProcessInput, JxlDecoderSetImageOutBuffer,
        JxlDecoderSetInput, JxlDecoderStatus, JxlDecoderSubscribeEvents, JxlEncoderStatus,
    };

    use super::*;

    struct Decoder(*mut JxlDecoder);

    impl Decoder {
        fn new() -> Self {
            // SAFETY: null memory manager uses libjxl's default allocator.
            let raw = unsafe { JxlDecoderCreate(ptr::null()) };
            assert!(!raw.is_null());
            Self(raw)
        }
    }

    impl Drop for Decoder {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `JxlDecoderCreate`.
            unsafe { JxlDecoderDestroy(self.0) };
        }
    }

    fn decode_u8(encoded: &[u8], channels: u32) -> (u32, u32, u32, Vec<u8>) {
        let decoder = Decoder::new();
        let dec = decoder.0;
        let events = (jxl_sys::JxlDecoderStatus::JXL_DEC_BASIC_INFO as c_int)
            | (jxl_sys::JxlDecoderStatus::JXL_DEC_FULL_IMAGE as c_int);
        // SAFETY: `dec` is valid and `events` is a bitmask of `JxlDecoderStatus`.
        assert_eq!(
            unsafe { JxlDecoderSubscribeEvents(dec, events) },
            JxlDecoderStatus::JXL_DEC_SUCCESS
        );
        // SAFETY: `encoded` stays alive until `JxlDecoderProcessInput` completes.
        assert_eq!(
            unsafe { JxlDecoderSetInput(dec, encoded.as_ptr(), encoded.len()) },
            JxlDecoderStatus::JXL_DEC_SUCCESS
        );

        let format = JxlPixelFormat {
            num_channels: channels,
            data_type: JxlDataType::JXL_TYPE_UINT8,
            endianness: JxlEndianness::JXL_NATIVE_ENDIAN,
            align: 0,
        };
        let mut info = MaybeUninit::<JxlBasicInfo>::uninit();
        let mut has_info = false;
        let mut pixels = Vec::new();

        loop {
            // SAFETY: `dec` is initialized with valid input.
            match unsafe { JxlDecoderProcessInput(dec) } {
                JxlDecoderStatus::JXL_DEC_BASIC_INFO => {
                    // SAFETY: `info` points to writable `JxlBasicInfo`.
                    assert_eq!(
                        unsafe { JxlDecoderGetBasicInfo(dec, info.as_mut_ptr()) },
                        JxlDecoderStatus::JXL_DEC_SUCCESS
                    );
                    has_info = true;
                }
                JxlDecoderStatus::JXL_DEC_NEED_IMAGE_OUT_BUFFER => {
                    let mut size = 0usize;
                    // SAFETY: `dec` and `format` are valid after basic info.
                    assert_eq!(
                        unsafe { JxlDecoderImageOutBufferSize(dec, &format, &mut size) },
                        JxlDecoderStatus::JXL_DEC_SUCCESS
                    );
                    pixels.resize(size, 0);
                    // SAFETY: `pixels` has `size` writable bytes and remains alive across decoding.
                    assert_eq!(
                        unsafe {
                            JxlDecoderSetImageOutBuffer(
                                dec,
                                &format,
                                pixels.as_mut_ptr().cast(),
                                pixels.len(),
                            )
                        },
                        JxlDecoderStatus::JXL_DEC_SUCCESS
                    );
                }
                JxlDecoderStatus::JXL_DEC_FULL_IMAGE => {}
                JxlDecoderStatus::JXL_DEC_SUCCESS => break,
                other => panic!("unexpected JXL decoder status: {other:?}"),
            }
        }

        assert!(has_info);
        // SAFETY: `has_info` guarantees `JxlDecoderGetBasicInfo` succeeded.
        let info = unsafe { info.assume_init() };
        (info.xsize, info.ysize, info.num_color_channels, pixels)
    }

    #[test]
    fn lossless_round_trip_for_l8_rgb8_and_rgba8() {
        let l8 = vec![0u8, 64, 128, 255, 10, 20, 30, 40, 99, 150, 200, 250];
        let encoded_l8 = encode(JxlPixels::L8(&l8), 4, 3, true, 90).unwrap();
        let (w, h, color_channels, decoded_l8) = decode_u8(&encoded_l8, 1);
        assert_eq!((w, h, color_channels), (4, 3, 1));
        assert_eq!(decoded_l8, l8);

        let rgb8 = vec![
            10, 20, 30, 40, 50, 60, //
            70, 80, 90, 100, 110, 120,
        ];
        let encoded_rgb = encode(JxlPixels::Rgb8(&rgb8), 2, 2, true, 90).unwrap();
        let (w, h, color_channels, decoded_rgb) = decode_u8(&encoded_rgb, 3);
        assert_eq!((w, h, color_channels), (2, 2, 3));
        assert_eq!(decoded_rgb, rgb8);

        let rgba8 = vec![
            10, 20, 30, 255, 40, 50, 60, 128, //
            70, 80, 90, 0, 100, 110, 120, 200,
        ];
        let encoded_rgba = encode(JxlPixels::Rgba8(&rgba8), 2, 2, true, 90).unwrap();
        let (w, h, color_channels, decoded_rgba) = decode_u8(&encoded_rgba, 4);
        assert_eq!((w, h, color_channels), (2, 2, 3));
        assert_eq!(decoded_rgba, rgba8);
    }

    #[test]
    fn lossy_encode_decodes_expected_dimensions() {
        let l8 = vec![120u8; 16 * 16];
        let encoded = encode(JxlPixels::L8(&l8), 16, 16, false, 85).unwrap();
        let (w, h, color_channels, decoded) = decode_u8(&encoded, 1);
        assert_eq!((w, h, color_channels), (16, 16, 1));
        assert_eq!(decoded.len(), 16 * 16);
        let _ = JxlEncoderStatus::JXL_ENC_SUCCESS;
    }

    #[test]
    fn rejects_invalid_dimensions_quality_and_buffer_length() {
        assert!(encode(JxlPixels::L8(&[]), 0, 10, true, 90).is_err());
        assert!(encode(JxlPixels::L8(&[1, 2, 3, 4]), 2, 2, false, 0).is_err());
        assert!(encode(JxlPixels::L8(&[1, 2, 3, 4]), 2, 2, false, 101).is_err());
        assert!(encode(JxlPixels::Rgb8(&[1, 2, 3]), 2, 2, true, 90).is_err());
    }
}
