use axum::{
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
};

use crate::{error::AppError, routes::AppState};

pub async fn require_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);

    match token {
        Some(token)
            if state
                .config
                .bot_tokens
                .iter()
                .any(|allowed| allowed == token) =>
        {
            Ok(next.run(request).await)
        }
        _ => Err(AppError::Unauthorized),
    }
}
