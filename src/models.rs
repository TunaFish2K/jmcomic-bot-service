use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "totalViews", default)]
    pub total_views: String,
    #[serde(default)]
    pub likes: String,
    #[serde(default)]
    pub series: Vec<SeriesItem>,
    #[serde(rename = "seriesID", default)]
    pub series_id: String,
    #[serde(default)]
    pub author: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub works: Vec<String>,
    #[serde(default)]
    pub actors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterInfo {
    pub id: String,
    pub name: String,
    pub order: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumInfo {
    #[serde(flatten)]
    pub album: Album,
    pub chapters: Vec<ChapterInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub images: Vec<ImageInfo>,
    #[serde(rename = "scrambleId")]
    pub scramble_id: u32,
    #[serde(rename = "imageBaseURL", default)]
    pub image_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchError {
    pub message: String,
    pub stage: String,
    pub domain: Option<String>,
    pub reference: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BatchPhotoItem {
    Ok {
        #[serde(rename = "photoId")]
        photo_id: String,
        photo: Photo,
    },
    Err {
        #[serde(rename = "photoId")]
        photo_id: String,
        photo: Option<Photo>,
        error: BatchError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactFormat {
    Zip,
    Cbz,
    Pdf,
}

impl ArtifactFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Cbz => "cbz",
            Self::Pdf => "pdf",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Zip => "application/zip",
            Self::Cbz => "application/vnd.comicbook+zip",
            Self::Pdf => "application/pdf",
        }
    }
}

impl std::fmt::Display for ArtifactFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Zip => "zip",
            Self::Cbz => "cbz",
            Self::Pdf => "pdf",
        })
    }
}

impl std::str::FromStr for ArtifactFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "zip" => Ok(Self::Zip),
            "cbz" => Ok(Self::Cbz),
            "pdf" => Ok(Self::Pdf),
            other => Err(format!("unsupported artifact format: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResponse {
    pub job_id: String,
    pub status: String,
    pub format: String,
    pub album_id: String,
    pub photo_ids: Vec<String>,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
    pub cached: bool,
    pub artifact_id: Option<String>,
    pub download_url: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactResponse {
    pub artifact_id: String,
    pub format: String,
    pub title: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub page_count: i64,
    pub download_url: String,
    pub created_at: i64,
    pub last_accessed_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateDownloadRequest {
    pub album_id: String,
    #[serde(default)]
    pub photo_ids: Option<Vec<String>>,
    pub format: ArtifactFormat,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub query: Option<String>,
    pub page: Option<u32>,
    #[serde(rename = "orderBy")]
    pub order_by: Option<String>,
    pub time: Option<String>,
    #[serde(rename = "mainTag")]
    pub main_tag: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct SignedFileQuery {
    pub exp: i64,
    pub sig: String,
}

pub fn sorted_chapters(album: &Album) -> Vec<ChapterInfo> {
    let mut chapters = if album.series.is_empty() {
        vec![ChapterInfo {
            id: album.id.clone(),
            name: album.name.clone(),
            order: 0,
        }]
    } else {
        album
            .series
            .iter()
            .map(|item| ChapterInfo {
                id: item.id.clone(),
                name: item.name.clone(),
                order: parse_series_order(&item.sort),
            })
            .collect()
    };

    chapters.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    chapters
}

fn parse_series_order(value: &str) -> i64 {
    value.parse::<i64>().unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn artifact_format_round_trips_extension_and_content_type() {
        assert_eq!(
            ArtifactFormat::from_str("zip").unwrap(),
            ArtifactFormat::Zip
        );
        assert_eq!(
            ArtifactFormat::from_str("cbz").unwrap(),
            ArtifactFormat::Cbz
        );
        assert_eq!(
            ArtifactFormat::from_str("pdf").unwrap(),
            ArtifactFormat::Pdf
        );
        assert_eq!(ArtifactFormat::Zip.extension(), "zip");
        assert_eq!(
            ArtifactFormat::Cbz.content_type(),
            "application/vnd.comicbook+zip"
        );
        assert_eq!(ArtifactFormat::Pdf.to_string(), "pdf");
        assert!(ArtifactFormat::from_str("rar").is_err());
    }

    #[test]
    fn job_status_display_uses_api_values() {
        assert_eq!(JobStatus::Queued.to_string(), "queued");
        assert_eq!(JobStatus::Running.to_string(), "running");
        assert_eq!(JobStatus::Completed.to_string(), "completed");
        assert_eq!(JobStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn sorted_chapters_uses_album_for_single_chapter() {
        let album = album_with_series(vec![]);
        assert_eq!(sorted_chapters(&album)[0].id, "album");
        assert_eq!(sorted_chapters(&album)[0].order, 0);
    }

    #[test]
    fn sorted_chapters_orders_series_and_pushes_invalid_sort_last() {
        let album = album_with_series(vec![
            SeriesItem {
                id: "b".to_owned(),
                name: "B".to_owned(),
                sort: "2".to_owned(),
            },
            SeriesItem {
                id: "bad".to_owned(),
                name: "Bad".to_owned(),
                sort: "abc".to_owned(),
            },
            SeriesItem {
                id: "a".to_owned(),
                name: "A".to_owned(),
                sort: "1".to_owned(),
            },
        ]);
        let ids = sorted_chapters(&album)
            .into_iter()
            .map(|chapter| chapter.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b", "bad"]);
    }

    fn album_with_series(series: Vec<SeriesItem>) -> Album {
        Album {
            id: "album".to_owned(),
            name: "Album".to_owned(),
            images: Vec::new(),
            description: None,
            total_views: "0".to_owned(),
            likes: "0".to_owned(),
            series,
            series_id: String::new(),
            author: Vec::new(),
            tags: Vec::new(),
            works: Vec::new(),
            actors: Vec::new(),
        }
    }
}
