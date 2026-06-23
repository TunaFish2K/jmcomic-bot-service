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
