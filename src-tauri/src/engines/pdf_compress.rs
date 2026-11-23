//! Makes a PDF smaller by shrinking and re-encoding the pictures inside it. Text and
//! vector graphics are left exactly as they are, so the PDF stays searchable.

use super::{require_output, Run};
use crate::tools::Op;
use image::codecs::jpeg::JpegEncoder;
use image::{imageops::FilterType, DynamicImage, ExtendedColorType, GrayImage, RgbImage};
use lopdf::{Dictionary, Document, Object, Stream};
use std::sync::atomic::Ordering;

/// With no maximum set by the preset, pictures are capped by quality alone.
fn default_edge(quality: u8) -> u32 {
    match quality {
        80.. => 3000,
        50.. => 2000,
        _ => 1400,
    }
}

fn name_of(object: &Object) -> Option<&[u8]> {
    object.as_name().ok()
}

/// A dictionary value, following a reference if it is one.
fn resolve<'a>(document: &'a Document, object: &'a Object) -> &'a Object {
    match object {
        Object::Reference(id) => document.get_object(*id).unwrap_or(object),
        other => other,
    }
}

/// The single filter of a stream, which may be spelled as a one-item array.
fn only_filter<'a>(document: &'a Document, dict: &'a Dictionary) -> Result<Option<&'a [u8]>, ()> {
    let Ok(filter) = dict.get(b"Filter") else { return Ok(None) };
    match resolve(document, filter) {
        Object::Name(name) => Ok(Some(name)),
        Object::Array(items) if items.is_empty() => Ok(None),
        Object::Array(items) if items.len() == 1 => Ok(name_of(resolve(document, &items[0]))),
        _ => Err(()),
    }
}

/// How many colour channels an image's colour space has, for the ones safe to re-encode.
fn channels(document: &Document, dict: &Dictionary) -> Option<u8> {
    match resolve(document, dict.get(b"ColorSpace").ok()?) {
        Object::Name(name) => match name.as_slice() {
            b"DeviceRGB" => Some(3),
            b"DeviceGray" => Some(1),
            _ => None,
        },
        Object::Array(items) => {
            if name_of(resolve(document, items.first()?))? != b"ICCBased" {
                return None;
            }
            let Object::Stream(profile) = resolve(document, items.get(1)?) else { return None };
            match profile.dict.get(b"N").ok()?.as_i64().ok()? {
                3 => Some(3),
                1 => Some(1),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The picture in a stream, or None for anything that's safer left alone: masks,
/// palettes, CMYK, custom decode ranges, and less common codecs.
fn decode(document: &Document, stream: &Stream) -> Option<DynamicImage> {
    let dict = &stream.dict;
    if dict.get(b"ImageMask").ok().and_then(|o| o.as_bool().ok()) == Some(true) || dict.has(b"Decode") {
        return None;
    }
    let count = channels(document, dict)?;
    match only_filter(document, dict).ok()? {
        Some(b"DCTDecode") => {
            let picture = image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg).ok()?;
            // A channel count that disagrees means something the PDF would draw differently.
            let matches = match picture.color().channel_count() {
                1 => count == 1,
                3 => count == 3,
                _ => false,
            };
            matches.then_some(picture)
        }
        Some(b"FlateDecode") | None => {
            if dict.get(b"BitsPerComponent").ok()?.as_i64().ok()? != 8 {
                return None;
            }
            let width = u32::try_from(dict.get(b"Width").ok()?.as_i64().ok()?).ok()?;
            let height = u32::try_from(dict.get(b"Height").ok()?.as_i64().ok()?).ok()?;
            let data = stream.decompressed_content().ok().unwrap_or_else(|| stream.content.clone());
            if count == 3 {
                RgbImage::from_raw(width, height, data).map(DynamicImage::ImageRgb8)
            } else {
                GrayImage::from_raw(width, height, data).map(DynamicImage::ImageLuma8)
            }
        }
        _ => None,
    }
}

fn encode(picture: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut bytes, quality);
    let (width, height) = (picture.width(), picture.height());
    match picture {
        DynamicImage::ImageLuma8(grey) => encoder.encode(grey, width, height, ExtendedColorType::L8).ok()?,
        other => encoder.encode(other.to_rgb8().as_raw(), width, height, ExtendedColorType::Rgb8).ok()?,
    }
    Some(bytes)
}

/// Replaces a picture only when that really saves space.
fn shrink(document: &Document, stream: &Stream, quality: u8, edge: u32) -> Option<(Vec<u8>, u32, u32, bool)> {
    let picture = decode(document, stream)?;
    let picture = if picture.width().max(picture.height()) > edge {
        picture.resize(edge, edge, FilterType::Lanczos3)
    } else {
        picture
    };
    let bytes = encode(&picture, quality)?;
    let grey = matches!(picture, DynamicImage::ImageLuma8(_));
    (bytes.len() < stream.content.len()).then_some((bytes, picture.width(), picture.height(), grey))
}

pub fn run(run: &Run, op: &Op) -> Result<(), String> {
    let Op::Compress { quality, max_edge } = *op else {
        return Err("A PDF can only be compressed".into());
    };
    let quality = quality.clamp(1, 100);
    let edge = max_edge.unwrap_or_else(|| default_edge(quality));
    let mut document = Document::load(run.source).map_err(|e| format!("Can't open PDF: {e}"))?;
    if document.is_encrypted() {
        return Err("This PDF is password protected".into());
    }

    let ids: Vec<_> = document.objects.keys().copied().collect();
    let total = ids.len().max(1) as f64;
    for (i, id) in ids.iter().enumerate() {
        if run.cancel.load(Ordering::SeqCst) {
            return Err("Cancelled".into());
        }
        let is_image = matches!(
            document.objects.get(id),
            Some(Object::Stream(s)) if s.dict.get(b"Subtype").ok().and_then(name_of) == Some(b"Image")
        );
        if is_image {
            let replacement = match document.objects.get(id) {
                Some(Object::Stream(stream)) => shrink(&document, stream, quality, edge),
                _ => None,
            };
            if let (Some((bytes, width, height, grey)), Some(Object::Stream(stream))) =
                (replacement, document.objects.get_mut(id))
            {
                let dict = &mut stream.dict;
                dict.set("Width", i64::from(width));
                dict.set("Height", i64::from(height));
                dict.set("BitsPerComponent", 8);
                dict.set("ColorSpace", Object::Name(if grey { b"DeviceGray".to_vec() } else { b"DeviceRGB".to_vec() }));
                dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
                dict.remove(b"DecodeParms");
                stream.set_content(bytes);
                // Already JPEG: compressing the file must not wrap this stream again.
                stream.allows_compression = false;
            }
        }
        run.reporter.set((i + 1) as f64 / total);
    }
    document.compress();
    document.save(run.output).map_err(|e| format!("Can't write the PDF: {e}"))?;
    require_output(run.output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn image_stream(colour: Object, data: Vec<u8>, side: i64) -> Stream {
        Stream::new(
            dictionary! {
                "Subtype" => "Image",
                "Width" => side,
                "Height" => side,
                "BitsPerComponent" => 8,
                "ColorSpace" => colour,
            },
            data,
        )
    }

    #[test]
    fn raw_rgb_is_decoded_and_shrunk() {
        let document = Document::with_version("1.4");
        let stream = image_stream(Object::Name(b"DeviceRGB".to_vec()), vec![200; 300 * 300 * 3], 300);
        let (bytes, width, height, grey) = shrink(&document, &stream, 70, 100).unwrap();
        assert_eq!((width, height, grey), (100, 100, false));
        assert!(bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[test]
    fn grey_stays_grey() {
        let document = Document::with_version("1.4");
        let stream = image_stream(Object::Name(b"DeviceGray".to_vec()), vec![90; 200 * 200], 200);
        assert!(shrink(&document, &stream, 70, 3000).unwrap().3);
    }

    #[test]
    fn risky_pictures_are_left_alone() {
        let document = Document::with_version("1.4");
        let cmyk = image_stream(Object::Name(b"DeviceCMYK".to_vec()), vec![0; 40_000], 100);
        assert!(shrink(&document, &cmyk, 70, 3000).is_none());
        let mut mask = image_stream(Object::Name(b"DeviceGray".to_vec()), vec![0; 10_000], 100);
        mask.dict.set("ImageMask", true);
        assert!(shrink(&document, &mask, 70, 3000).is_none());
        let mut inverted = image_stream(Object::Name(b"DeviceGray".to_vec()), vec![0; 10_000], 100);
        inverted.dict.set("Decode", vec![Object::Integer(1), Object::Integer(0)]);
        assert!(shrink(&document, &inverted, 70, 3000).is_none());
    }

    #[test]
    fn a_replacement_that_saves_nothing_is_dropped() {
        let document = Document::with_version("1.4");
        let tiny = image_stream(Object::Name(b"DeviceGray".to_vec()), vec![0; 4], 2);
        assert!(shrink(&document, &tiny, 70, 3000).is_none());
    }
}
