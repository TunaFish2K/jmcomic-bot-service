use reqwest::{Client, Url};

use crate::{
    error::{AppError, AppResult},
    models::{Album, BatchPhotoItem, Photo, SearchQuery},
};

#[derive(Debug, Clone)]
pub struct WorkerClient {
    client: Client,
    base_url: Url,
}

impl WorkerClient {
    pub fn new(base_url: String) -> AppResult<Self> {
        let mut base_url = Url::parse(&base_url).map_err(|error| {
            AppError::BadRequest(format!("invalid config.worker_base_url: {error}"))
        })?;
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }

        Ok(Self {
            client: Client::builder()
                .user_agent("jmcomic-bot-service/0.1")
                .build()?,
            base_url,
        })
    }

    pub async fn search(&self, query: &SearchQuery) -> AppResult<serde_json::Value> {
        let search = query
            .q
            .as_deref()
            .or(query.query.as_deref())
            .ok_or_else(|| AppError::BadRequest("missing query q".to_owned()))?;
        let mut url = self.join("search")?;
        url.query_pairs_mut()
            .append_pair("query", search)
            .append_pair("page", &query.page.unwrap_or(1).to_string())
            .append_pair("orderBy", query.order_by.as_deref().unwrap_or("mr"))
            .append_pair("time", query.time.as_deref().unwrap_or("a"))
            .append_pair("mainTag", &query.main_tag.unwrap_or(0).to_string());

        self.get_json(url).await
    }

    pub async fn album(&self, album_id: &str) -> AppResult<Album> {
        let url = self.join(&format!("album/{album_id}"))?;
        self.get_json(url).await
    }

    pub async fn photo(&self, photo_id: &str) -> AppResult<Photo> {
        let url = self.join(&format!("photo/{photo_id}"))?;
        self.get_json(url).await
    }

    pub async fn batch_photos(&self, photo_ids: &[String]) -> AppResult<Vec<Photo>> {
        if photo_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut photos = Vec::with_capacity(photo_ids.len());
        for chunk in photo_ids.chunks(20) {
            let mut url = self.join("batch-photo")?;
            url.query_pairs_mut().append_pair("ids", &chunk.join(","));
            let items: Vec<BatchPhotoItem> = self.get_json(url).await?;
            for item in items {
                match item {
                    BatchPhotoItem::Ok { photo, .. } => photos.push(photo),
                    BatchPhotoItem::Err {
                        photo_id, error, ..
                    } => {
                        return Err(AppError::Upstream(format!(
                            "failed to fetch photo {photo_id}: {}",
                            error.message
                        )));
                    }
                }
            }
        }

        Ok(photos)
    }

    async fn get_json<T>(&self, url: Url) -> AppResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self.client.get(url.clone()).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status.as_u16() == 404 {
                return Err(AppError::NotFound(body));
            }
            return Err(AppError::Upstream(format!(
                "worker returned {status} for {url}: {body}"
            )));
        }

        Ok(response.json::<T>().await?)
    }

    fn join(&self, path: &str) -> AppResult<Url> {
        self.base_url
            .join(path)
            .map_err(|error| AppError::BadRequest(format!("invalid worker URL path: {error}")))
    }
}
