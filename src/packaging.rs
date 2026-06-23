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
