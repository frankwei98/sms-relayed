use axum::body::Body;
use axum::http::{header, HeaderValue, Request, Response, StatusCode};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct FrontendAssets;

pub async fn serve(req: Request<Body>) -> Response<Body> {
    let requested_path = req.uri().path().trim_start_matches('/');
    let requested_path = if requested_path.is_empty() {
        "index.html"
    } else {
        requested_path
    };
    let (asset_path, asset) = match FrontendAssets::get(requested_path) {
        Some(content) => (requested_path, Some(content)),
        None => ("index.html", FrontendAssets::get("index.html")),
    };
    match asset {
        Some(content) => {
            let mime = mime_guess::from_path(asset_path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime.as_ref()).unwrap(),
                )
                .body(Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request};

    use super::serve;

    #[tokio::test]
    async fn spa_routes_are_served_as_html() {
        let response = serve(
            Request::builder()
                .uri("/modem")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html"
        );
    }
}
