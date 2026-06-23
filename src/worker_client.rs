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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;

    #[tokio::test]
    async fn search_uses_defaults_and_preserves_base_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/worker/search"))
            .and(query_param("query", "blue"))
            .and(query_param("page", "1"))
            .and(query_param("orderBy", "mr"))
            .and(query_param("time", "a"))
            .and(query_param("mainTag", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total": "1",
                "content": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = WorkerClient::new(format!("{}/worker", server.uri())).unwrap();
        let value = client
            .search(&SearchQuery {
                q: Some("blue".to_owned()),
                query: None,
                page: None,
                order_by: None,
                time: None,
                main_tag: None,
            })
            .await
            .unwrap();
        assert_eq!(value["total"], "1");
    }

    #[tokio::test]
    async fn search_rejects_missing_query() {
        let client = WorkerClient::new("http://127.0.0.1:1".to_owned()).unwrap();
        let error = client
            .search(&SearchQuery {
                q: None,
                query: None,
                page: None,
                order_by: None,
                time: None,
                main_tag: None,
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing query"));
    }

    #[tokio::test]
    async fn batch_photos_chunks_ids_and_maps_item_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/batch-photo"))
            .and(query_param("ids", ids(1, 20).join(",")))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_batch(1, 20)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/batch-photo"))
            .and(query_param("ids", "21"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {
                    "photoId": "21",
                    "photo": null,
                    "error": {
                        "message": "boom",
                        "stage": "get_photo",
                        "domain": null,
                        "reference": null,
                        "retryable": false
                    }
                }
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = WorkerClient::new(server.uri()).unwrap();
        let error = client
            .batch_photos(&ids(1, 21))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("failed to fetch photo 21"));
    }

    #[tokio::test]
    async fn worker_http_errors_are_mapped() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/album/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_string("missing"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/album/fail"))
            .respond_with(ResponseTemplate::new(500).set_body_string("fail"))
            .mount(&server)
            .await;

        let client = WorkerClient::new(server.uri()).unwrap();
        assert!(matches!(
            client.album("missing").await.unwrap_err(),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            client.album("fail").await.unwrap_err(),
            AppError::Upstream(_)
        ));
    }

    fn ids(start: usize, end: usize) -> Vec<String> {
        (start..=end).map(|id| id.to_string()).collect()
    }

    fn ok_batch(start: usize, end: usize) -> serde_json::Value {
        let items = (start..=end)
            .map(|id| {
                json!({
                    "photoId": id.to_string(),
                    "photo": {
                        "id": id.to_string(),
                        "name": format!("Chapter {id}"),
                        "images": [],
                        "scrambleId": 999999
                    }
                })
            })
            .collect::<Vec<_>>();
        serde_json::Value::Array(items)
    }
}
