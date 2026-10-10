//! Classic PSD serialization.
//!
//! GIMP reference: top-level export section order:
//! https://github.com/GNOME/gimp/blob/758fb4ed995bbb339282d3f777089a33f0a391b8/plug-ins/file-psd/psd-export.c#L2527-L2578
//! GIMP reference: header serialization:
//! https://github.com/GNOME/gimp/blob/758fb4ed995bbb339282d3f777089a33f0a391b8/plug-ins/file-psd/psd-export.c#L641-L686
//! GIMP reference: layer records and channel payloads:
//! https://github.com/GNOME/gimp/blob/758fb4ed995bbb339282d3f777089a33f0a391b8/plug-ins/file-psd/psd-export.c#L1520-L1926
//! GIMP reference: merged image data and Photoshop white matte:
//! https://github.com/GNOME/gimp/blob/758fb4ed995bbb339282d3f777089a33f0a391b8/plug-ins/file-psd/psd-export.c#L2252-L2378

use image::RgbaImage;
use koharu_rasterizer::{RasterOptions, Rasterizer};
use koharu_renderer::Frame;
use koharu_scene::Snapshot;
use std::sync::Arc;

use crate::{
    descriptor::{
        DescriptorObject, DescriptorValue, bounds_descriptor, write_versioned_descriptor,
    },
    document::{Document, Layer, PsdExportOptions, TextMetadata},
    engine_data::{TextEngineSpec, TextOrientation, encode_engine_data},
    error::PsdExportError,
    packbits::{ChannelId, EncodedChannel, encode_image_rle},
    writer::PsdWriter,
};

#[tracing::instrument(skip_all)]
pub async fn export_page(
    rasterizer: Arc<Rasterizer>,
    snapshot: &Snapshot,
    frame: &Frame,
    options: &PsdExportOptions,
) -> Result<Vec<u8>, PsdExportError> {
    let document = crate::document::build(
        snapshot,
        frame,
        rasterizer,
        RasterOptions::default(),
        options,
    )
    .await?;
    tokio_rayon::spawn(move || serialize(&document)).await
}

fn serialize(document: &Document) -> Result<Vec<u8>, PsdExportError> {
    let merged_has_alpha = has_transparency(&document.merged);
    let mut psd = PsdWriter::new();
    write_header(&mut psd, document.width, document.height, merged_has_alpha);
    psd.write_u32(0); // Color mode data.
    psd.write_u32(0); // Image resources.

    let layer_mask_info = build_layer_and_mask_info(&document.layers, merged_has_alpha)?;
    psd.write_u32(layer_mask_info.len() as u32);
    psd.write_bytes(&layer_mask_info);

    let mut merged = document.merged.clone();
    if merged_has_alpha {
        apply_photoshop_white_matte(&mut merged);
    }
    write_image_data(&mut psd, &merged, merged_has_alpha, "Merged Composite")?;
    Ok(psd.into_inner())
}

fn write_header(writer: &mut PsdWriter, width: u32, height: u32, has_alpha: bool) {
    writer.write_signature("8BPS");
    writer.write_u16(1);
    writer.write_zeroes(6);
    writer.write_u16(if has_alpha { 4 } else { 3 });
    writer.write_u32(height);
    writer.write_u32(width);
    writer.write_u16(8);
    writer.write_u16(3); // RGB.
}

fn build_layer_and_mask_info(
    layers: &[Layer],
    merged_has_alpha: bool,
) -> Result<Vec<u8>, PsdExportError> {
    let mut layer_info = PsdWriter::new();
    let count =
        i16::try_from(layers.len()).map_err(|_| PsdExportError::TooManyLayers(layers.len()))?;
    layer_info.write_i16(if merged_has_alpha { -count } else { count });

    let mut encoded_layers = Vec::with_capacity(layers.len());
    let mut extra_data = Vec::with_capacity(layers.len());
    for layer in layers {
        // GIMP writes alpha first for layer channel data, followed by color channels.
        let channels = encode_image_rle(
            &layer.pixels,
            &[
                ChannelId::Alpha,
                ChannelId::Red,
                ChannelId::Green,
                ChannelId::Blue,
            ],
            &layer.name,
        )?;
        encoded_layers.push(channels);
        extra_data.push(build_extra_data(layer)?);
    }

    for ((layer, channels), extra) in layers.iter().zip(&encoded_layers).zip(&extra_data) {
        let width = i32::try_from(layer.pixels.width()).map_err(|_| {
            PsdExportError::InvalidLayerBounds {
                layer: layer.name.clone(),
                width: i32::MAX,
                height: layer.pixels.height() as i32,
            }
        })?;
        let height = i32::try_from(layer.pixels.height()).map_err(|_| {
            PsdExportError::InvalidLayerBounds {
                layer: layer.name.clone(),
                width,
                height: i32::MAX,
            }
        })?;
        let right =
            layer
                .left
                .checked_add(width)
                .ok_or_else(|| PsdExportError::InvalidLayerBounds {
                    layer: layer.name.clone(),
                    width,
                    height,
                })?;
        let bottom =
            layer
                .top
                .checked_add(height)
                .ok_or_else(|| PsdExportError::InvalidLayerBounds {
                    layer: layer.name.clone(),
                    width,
                    height,
                })?;

        layer_info.write_i32(layer.top);
        layer_info.write_i32(layer.left);
        layer_info.write_i32(bottom);
        layer_info.write_i32(right);
        layer_info.write_u16(channels.len() as u16);
        for channel in channels {
            layer_info.write_i16(channel.channel_id);
            layer_info.write_u32((2 + channel.data.len()) as u32);
        }
        layer_info.write_signature("8BIM");
        layer_info.write_signature("norm");
        layer_info.write_u8(255);
        layer_info.write_u8(0);
        layer_info.write_u8(if layer.hidden { 2 } else { 0 });
        layer_info.write_u8(0);
        layer_info.write_u32(extra.len() as u32);
        layer_info.write_bytes(extra);
    }

    for channels in &encoded_layers {
        for channel in channels {
            layer_info.write_u16(1); // RLE.
            layer_info.write_bytes(&channel.data);
        }
    }
    layer_info.pad_to_multiple(4);

    let mut full = PsdWriter::new();
    full.write_u32(layer_info.len() as u32);
    full.write_bytes(&layer_info.into_inner());
    full.write_u32(0); // Global layer mask info.
    Ok(full.into_inner())
}

fn build_extra_data(layer: &Layer) -> Result<Vec<u8>, PsdExportError> {
    let mut extra = PsdWriter::new();
    extra.write_u32(0); // Layer mask data.
    extra.write_u32(0); // Layer blending ranges.
    extra.write_pascal_string(&layer.name, 4);

    write_additional_info_block(&mut extra, "luni", &luni_body(&layer.name), 4);
    write_additional_info_block(&mut extra, "lyid", &layer.id.to_be_bytes(), 4);
    write_additional_info_block(&mut extra, "lclr", &[0; 8], 4);
    if let Some(text) = layer.text.as_ref() {
        write_additional_info_block(&mut extra, "TySh", &tysh_body(text)?, 2);
    }
    extra.pad_to_multiple(4);
    Ok(extra.into_inner())
}

fn luni_body(name: &str) -> Vec<u8> {
    let mut units = Vec::new();
    for character in name.chars() {
        let mut encoded = [0; 2];
        let encoded = character.encode_utf16(&mut encoded);
        if units.len() + encoded.len() > 255 {
            break;
        }
        units.extend_from_slice(encoded);
    }
    let mut body = PsdWriter::new();
    body.write_u32(units.len() as u32);
    for unit in units.iter().copied() {
        body.write_u16(unit);
    }
    if !units.len().is_multiple_of(2) {
        body.write_u16(0);
    }
    body.into_inner()
}

fn tysh_body(text: &TextMetadata) -> Result<Vec<u8>, PsdExportError> {
    let engine_data = encode_engine_data(&TextEngineSpec {
        text: text.text.clone(),
        font_index: text.font_index,
        font_set: text.font_set.clone(),
        font_size: text.font_size,
        color: text.color,
        faux_bold: false,
        faux_italic: false,
        orientation: text.orientation,
        justification: text.justification,
        box_width: text.box_width,
        box_height: text.box_height,
    });
    let bounds = bounds_descriptor(
        "bounds",
        text.bounds[0],
        text.bounds[1],
        text.bounds[2],
        text.bounds[3],
    );
    let bounding_box = bounds_descriptor(
        "boundingBox",
        text.bounds[0],
        text.bounds[1],
        text.bounds[2],
        text.bounds[3],
    );
    let text_descriptor = DescriptorObject::new("", "TxLr")
        .with_item("Txt ", DescriptorValue::Text(text.text.clone()))
        .with_item(
            "textGridding",
            DescriptorValue::Enum {
                type_id: "textGridding".to_owned(),
                value: "None".to_owned(),
            },
        )
        .with_item(
            "Ornt",
            DescriptorValue::Enum {
                type_id: "Ornt".to_owned(),
                value: orientation_id(text.orientation).to_owned(),
            },
        )
        .with_item(
            "AntA",
            DescriptorValue::Enum {
                type_id: "Annt".to_owned(),
                value: "antiAliasSharp".to_owned(),
            },
        )
        .with_item("bounds", DescriptorValue::Object(bounds))
        .with_item("boundingBox", DescriptorValue::Object(bounding_box))
        .with_item("TextIndex", DescriptorValue::Integer(text.index))
        .with_item("EngineData", DescriptorValue::Raw(engine_data));
    let warp_descriptor = DescriptorObject::new("", "warp")
        .with_item(
            "warpStyle",
            DescriptorValue::Enum {
                type_id: "warpStyle".to_owned(),
                value: "warpNone".to_owned(),
            },
        )
        .with_item("warpValue", DescriptorValue::Double(0.0))
        .with_item("warpPerspective", DescriptorValue::Double(0.0))
        .with_item("warpPerspectiveOther", DescriptorValue::Double(0.0))
        .with_item(
            "warpRotate",
            DescriptorValue::Enum {
                type_id: "Ornt".to_owned(),
                value: orientation_id(text.orientation).to_owned(),
            },
        )
        .with_item(
            "bounds",
            DescriptorValue::Object(bounds_descriptor(
                "bounds",
                text.bounds[0],
                text.bounds[1],
                text.bounds[2],
                text.bounds[3],
            )),
        );

    let mut body = PsdWriter::new();
    body.write_i16(1);
    for value in text.transform {
        body.write_f64(value);
    }
    body.write_i16(50);
    write_versioned_descriptor(&mut body, &text_descriptor)?;
    body.write_i16(1);
    write_versioned_descriptor(&mut body, &warp_descriptor)?;
    // GIMP reads these four values as signed 32-bit integers in top/left/bottom/right order.
    body.write_i32(psd_coordinate(text.bounds[1]));
    body.write_i32(psd_coordinate(text.bounds[0]));
    body.write_i32(psd_coordinate(text.bounds[3]));
    body.write_i32(psd_coordinate(text.bounds[2]));
    body.pad_to_multiple(4);
    Ok(body.into_inner())
}

fn orientation_id(orientation: TextOrientation) -> &'static str {
    match orientation {
        TextOrientation::Horizontal => "Hrzn",
        TextOrientation::Vertical => "Vrtc",
    }
}

fn psd_coordinate(value: f64) -> i32 {
    value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

fn write_additional_info_block(writer: &mut PsdWriter, key: &str, body: &[u8], alignment: usize) {
    let padding = (alignment - (body.len() % alignment)) % alignment;
    writer.write_signature("8BIM");
    writer.write_signature(key);
    writer.write_u32((body.len() + padding) as u32);
    writer.write_bytes(body);
    writer.write_zeroes(padding);
}

fn write_image_data(
    writer: &mut PsdWriter,
    image: &RgbaImage,
    has_alpha: bool,
    name: &str,
) -> Result<(), PsdExportError> {
    writer.write_u16(1); // RLE.
    let mut channel_ids = vec![ChannelId::Red, ChannelId::Green, ChannelId::Blue];
    if has_alpha {
        channel_ids.push(ChannelId::Alpha);
    }
    let channels = encode_image_rle(image, &channel_ids, name)?;
    write_merged_channels(writer, &channels, image.height());
    Ok(())
}

fn write_merged_channels(writer: &mut PsdWriter, channels: &[EncodedChannel], height: u32) {
    let row_lengths_len = height as usize * 2;
    for channel in channels {
        writer.write_bytes(&channel.data[..row_lengths_len]);
    }
    for channel in channels {
        writer.write_bytes(&channel.data[row_lengths_len..]);
    }
}

fn has_transparency(image: &RgbaImage) -> bool {
    image.pixels().any(|pixel| pixel[3] < 255)
}

fn apply_photoshop_white_matte(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        if alpha == 255 {
            continue;
        }
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * alpha + 128) / 255 + 255 - alpha) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;
    use crate::{
        document::{Document, Layer, TextMetadata},
        engine_data::{TextJustification, TextOrientation},
    };

    fn layer(name: &str, hidden: bool) -> Layer {
        Layer {
            id: 1,
            name: name.to_owned(),
            left: 0,
            top: 0,
            pixels: RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 4])),
            hidden,
            text: None,
        }
    }

    #[test]
    fn merged_rle_groups_all_row_tables_before_channel_payloads() {
        let image = RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 4]));
        let mut writer = PsdWriter::new();
        write_image_data(&mut writer, &image, true, "Merged Composite").expect("write image data");
        assert_eq!(
            writer.into_inner(),
            vec![
                0, 1, // compression
                0, 2, 0, 2, 0, 2, 0, 2, // row lengths
                0, 1, // red
                0, 2, // green
                0, 3, // blue
                0, 4, // alpha
            ]
        );
    }

    #[test]
    fn normal_and_hidden_layer_flags_match_gimp() {
        let visible = build_layer_and_mask_info(&[layer("visible", false)], true)
            .expect("visible layer info");
        let hidden =
            build_layer_and_mask_info(&[layer("hidden", true)], true).expect("hidden layer info");
        let visible_blend = visible
            .windows(8)
            .position(|bytes| bytes == b"8BIMnorm")
            .expect("blend signature");
        let hidden_blend = hidden
            .windows(8)
            .position(|bytes| bytes == b"8BIMnorm")
            .expect("blend signature");
        assert_eq!(visible[visible_blend + 10], 0);
        assert_eq!(hidden[hidden_blend + 10], 2);
    }

    #[test]
    fn layer_channel_records_put_alpha_before_rgb() {
        let bytes = build_layer_and_mask_info(&[layer("layer", false)], true).expect("layer info");
        // Layer-info length, count, bounds, channel count.
        let start = 4 + 2 + 16 + 2;
        let ids = (0..4)
            .map(|index| {
                let offset = start + index * 6;
                i16::from_be_bytes(bytes[offset..offset + 2].try_into().expect("channel id"))
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, [-1, 0, 1, 2]);
    }

    #[test]
    fn transparent_merged_preview_uses_photoshop_white_matte() {
        let mut image = RgbaImage::from_pixel(1, 1, Rgba([0, 64, 255, 0]));
        apply_photoshop_white_matte(&mut image);
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 0]);
    }

    #[test]
    fn editable_layer_writes_gimp_readable_text_resources() {
        let text = TextMetadata {
            index: 1,
            text: "Hello".to_owned(),
            bounds: [2.0, 3.0, 12.0, 11.0],
            transform: [1.0, 0.0, 0.0, 1.0, 2.0, 3.0],
            orientation: TextOrientation::Horizontal,
            justification: TextJustification::Center,
            font_index: 0,
            font_set: vec!["ArialMT".to_owned()],
            font_size: 12.0,
            color: [0, 0, 0, 255],
            box_width: 10.0,
            box_height: 8.0,
        };
        let mut text_layer = layer("Text", false);
        text_layer.text = Some(text);
        let document = Document {
            width: 1,
            height: 1,
            merged: RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255])),
            layers: vec![text_layer],
        };
        let bytes = serialize(&document).expect("serialize PSD");
        assert!(bytes.windows(4).any(|bytes| bytes == b"luni"));
        assert!(bytes.windows(4).any(|bytes| bytes == b"lyid"));
        assert!(bytes.windows(4).any(|bytes| bytes == b"TySh"));
        assert!(bytes.windows(10).any(|bytes| bytes == b"EngineData"));
    }

    #[test]
    fn unicode_layer_name_is_limited_and_even_padded_like_gimp() {
        let body = luni_body(&"x".repeat(300));
        assert_eq!(
            u32::from_be_bytes(body[..4].try_into().expect("length")),
            255
        );
        assert_eq!(body.len(), 4 + 256 * 2);
    }
}
