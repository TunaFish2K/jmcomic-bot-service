use image::{ImageEncoder, RgbImage, codecs::jpeg::JpegEncoder, imageops};

#[derive(Debug, Clone)]
pub struct ProcessedImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn get_slice_count(scramble_id: u32, photo_id: &str, filename: &str) -> anyhow::Result<u32> {
    let parsed_photo_id = photo_id.parse::<u32>()?;

    if parsed_photo_id < scramble_id {
        return Ok(0);
    }
    if filename.to_ascii_lowercase().ends_with(".gif") {
        return Ok(0);
    }
    if parsed_photo_id < 268_850 {
        return Ok(10);
    }

    let name_without_extension = filename.split('.').next().unwrap_or(filename);
    let digest = format!(
        "{:x}",
        md5::compute(format!("{parsed_photo_id}{name_without_extension}"))
    );
    let last = digest.as_bytes().last().copied().unwrap_or(b'0') as u32;
    let modulo = if parsed_photo_id < 421_926 { 10 } else { 8 };
    Ok((last % modulo) * 2 + 2)
}

pub fn process_image(
    input: &[u8],
    slice_count: u32,
    jpeg_quality: u8,
) -> anyhow::Result<ProcessedImage> {
    let decoded = image::load_from_memory(input)?;
    let rgb = decoded.to_rgb8();
    let (width, height) = rgb.dimensions();
    let restored = if slice_count > 1 {
        restore_slices(&rgb, slice_count)
    } else {
        rgb
    };

    let mut output = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut output, jpeg_quality.clamp(1, 100));
    encoder.write_image(
        restored.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;

    Ok(ProcessedImage {
        data: output,
        width,
        height,
    })
}

pub fn restore_slices(source: &RgbImage, slice_count: u32) -> RgbImage {
    let (width, height) = source.dimensions();
    let over = height % slice_count;
    let move_height = height / slice_count;
    let mut result = RgbImage::new(width, height);

    for i in 0..slice_count {
        let source_y = height
            .saturating_sub(move_height * (i + 1))
            .saturating_sub(over);
        let mut destination_y = move_height * i;
        let mut slice_height = move_height;

        if i == 0 {
            slice_height += over;
        } else {
            destination_y += over;
        }

        if slice_height == 0 {
            continue;
        }

        let slice = imageops::crop_imm(source, 0, source_y, width, slice_height).to_image();
        imageops::replace(&mut result, &slice, 0, i64::from(destination_y));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn slice_count_matches_known_algorithm_branches() {
        assert_eq!(get_slice_count(300_000, "299999", "001.jpg").unwrap(), 0);
        assert_eq!(get_slice_count(1, "123456", "001.gif").unwrap(), 0);
        assert_eq!(get_slice_count(1, "268849", "001.jpg").unwrap(), 10);
        assert_eq!(get_slice_count(1, "421926", "00001.jpg").unwrap(), 14);
    }

    #[test]
    fn restore_slices_reverses_scrambled_layout() {
        let mut original: RgbImage = ImageBuffer::new(2, 7);
        for y in 0..7 {
            for x in 0..2 {
                original.put_pixel(x, y, Rgb([y as u8, x as u8, 0]));
            }
        }

        let scrambled = scramble_like_upstream(&original, 3);
        let restored = restore_slices(&scrambled, 3);
        assert_eq!(restored, original);
    }

    fn scramble_like_upstream(source: &RgbImage, slice_count: u32) -> RgbImage {
        let (width, height) = source.dimensions();
        let over = height % slice_count;
        let move_height = height / slice_count;
        let mut result = RgbImage::new(width, height);

        for i in 0..slice_count {
            let source_y = height
                .saturating_sub(move_height * (i + 1))
                .saturating_sub(over);
            let mut destination_y = move_height * i;
            let mut slice_height = move_height;

            if i == 0 {
                slice_height += over;
            } else {
                destination_y += over;
            }

            let slice =
                imageops::crop_imm(source, 0, destination_y, width, slice_height).to_image();
            imageops::replace(&mut result, &slice, 0, i64::from(source_y));
        }

        result
    }
}
