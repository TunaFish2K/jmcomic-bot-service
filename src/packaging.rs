use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use printpdf::{Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, RawImage, XObjectTransform};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{image_processing::ProcessedImage, models::ArtifactFormat};

#[derive(Debug, Clone)]
pub struct ArchivePhoto {
    pub name: String,
    pub images: Vec<ProcessedImage>,
}

pub fn write_artifact(
    path: &Path,
    format: ArtifactFormat,
    photos: &[ArchivePhoto],
) -> anyhow::Result<()> {
    match format {
        ArtifactFormat::Zip | ArtifactFormat::Cbz => write_zip(path, photos),
        ArtifactFormat::Pdf => write_pdf(path, photos),
    }
}

fn write_zip(path: &Path, photos: &[ArchivePhoto]) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    if photos.len() == 1 {
        let digits = digits_for(photos[0].images.len().max(1));
        for (index, image) in photos[0].images.iter().enumerate() {
            zip.start_file(format!("{:0digits$}.jpg", index + 1), options)?;
            zip.write_all(&image.data)?;
        }
    } else {
        let chapter_digits = digits_for(photos.len().max(1));
        for (photo_index, photo) in photos.iter().enumerate() {
            let prefix = format!(
                "{:0chapter_digits$}-{}",
                photo_index + 1,
                sanitize_archive_segment(&photo.name)
            );
            let image_digits = digits_for(photo.images.len().max(1));
            for (image_index, image) in photo.images.iter().enumerate() {
                zip.start_file(
                    format!("{prefix}/{:0image_digits$}.jpg", image_index + 1),
                    options,
                )?;
                zip.write_all(&image.data)?;
            }
        }
    }

    zip.finish()?;
    Ok(())
}

fn write_pdf(path: &Path, photos: &[ArchivePhoto]) -> anyhow::Result<()> {
    let mut document = PdfDocument::new("JMComic");
    let mut pages = Vec::new();
    let mut warnings = Vec::new();
    let dpi = 72.0_f32;

    for image in photos.iter().flat_map(|photo| photo.images.iter()) {
        let raw = RawImage::decode_from_bytes(&image.data, &mut warnings)
            .map_err(|message| anyhow::anyhow!("PDF image decode failed: {message}"))?;
        let image_id = document.add_image(&raw);
        let width_mm = image.width as f32 * 25.4 / dpi;
        let height_mm = image.height as f32 * 25.4 / dpi;
        pages.push(PdfPage::new(
            Mm(width_mm),
            Mm(height_mm),
            vec![Op::UseXobject {
                id: image_id,
                transform: XObjectTransform {
                    translate_x: None,
                    translate_y: None,
                    rotate: None,
                    scale_x: None,
                    scale_y: None,
                    dpi: Some(dpi),
                },
            }],
        ));
    }

    let bytes = document
        .with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut warnings);
    std::fs::write(path, bytes)?;
    Ok(())
}

pub fn sanitize_archive_segment(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            '\u{0000}'..='\u{001f}' => '_',
            _ => ch,
        })
        .collect::<String>()
        .trim()
        .to_owned();

    if sanitized.is_empty() {
        "untitled".to_owned()
    } else {
        sanitized
    }
}

fn digits_for(value: usize) -> usize {
    value.to_string().len()
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use image::{ImageEncoder, RgbImage, codecs::jpeg::JpegEncoder};
    use zip::ZipArchive;

    use super::*;

    #[test]
    fn sanitize_archive_segment_replaces_forbidden_characters() {
        assert_eq!(
            sanitize_archive_segment(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_archive_segment(" \u{0001} "), "_");
        assert_eq!(sanitize_archive_segment("   "), "untitled");
    }

    #[test]
    fn write_artifact_creates_single_chapter_zip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single.cbz");
        write_artifact(
            &path,
            ArtifactFormat::Cbz,
            &[ArchivePhoto {
                name: "chapter".to_owned(),
                images: vec![image(2, 2), image(3, 2)],
            }],
        )
        .unwrap();

        let mut archive = ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(archive.len(), 2);
        assert_eq!(archive.by_index(0).unwrap().name(), "1.jpg");
        assert_eq!(archive.by_index(1).unwrap().name(), "2.jpg");
    }

    #[test]
    fn write_artifact_creates_multi_chapter_zip_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.zip");
        write_artifact(
            &path,
            ArtifactFormat::Zip,
            &[
                ArchivePhoto {
                    name: "a/b".to_owned(),
                    images: vec![image(2, 2)],
                },
                ArchivePhoto {
                    name: "c".to_owned(),
                    images: vec![image(2, 3)],
                },
            ],
        )
        .unwrap();

        let mut archive = ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(archive.len(), 2);
        assert_eq!(archive.by_index(0).unwrap().name(), "1-a_b/1.jpg");
        assert_eq!(archive.by_index(1).unwrap().name(), "2-c/1.jpg");
    }

    #[test]
    fn write_artifact_creates_pdf_with_pages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.pdf");
        write_artifact(
            &path,
            ArtifactFormat::Pdf,
            &[ArchivePhoto {
                name: "chapter".to_owned(),
                images: vec![image(2, 2), image(2, 3)],
            }],
        )
        .unwrap();

        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.3"));
        assert!(String::from_utf8_lossy(&bytes).contains("%%EOF"));
        let mut warnings = Vec::new();
        let parsed = printpdf::PdfDocument::parse(
            &bytes,
            &printpdf::PdfParseOptions {
                fail_on_error: true,
            },
            &mut warnings,
        )
        .unwrap();
        assert_eq!(parsed.pages.len(), 2);
    }

    #[test]
    fn zip_entries_contain_jpeg_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single.zip");
        write_artifact(
            &path,
            ArtifactFormat::Zip,
            &[ArchivePhoto {
                name: "chapter".to_owned(),
                images: vec![image(2, 2)],
            }],
        )
        .unwrap();

        let mut archive = ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut entry = archive.by_index(0).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8]));
    }

    fn image(width: u32, height: u32) -> ProcessedImage {
        let mut rgb = RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                rgb.put_pixel(x, y, image::Rgb([x as u8 * 30, y as u8 * 30, 16]));
            }
        }

        let mut data = Vec::new();
        let encoder = JpegEncoder::new_with_quality(&mut data, 90);
        encoder
            .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
        ProcessedImage {
            data,
            width,
            height,
        }
    }
}
