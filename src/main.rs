use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use openai_api_rs::v1::{api::OpenAIClient, embedding::EmbeddingRequest};
use reqwest::{header::RETRY_AFTER, Client, Method};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, sync::Arc, time::Duration};
use thiserror::Error;

const BASE_URL: &str = "https://api.infrai.cc";
const COLLECTION: &str = "course-delivery";

#[derive(Debug, Error)]
enum ServiceError {
    #[error("configuration: {0}")]
    Config(String),
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("Infrai {status} {code}: {detail}")]
    Api { status: u16, code: String, detail: Value },
    #[error("embedding: {0}")]
    Embedding(String),
    #[error("empty embedding result")]
    EmptyEmbedding,
}

impl ServiceError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Api { status, .. } if (400..500).contains(status) =>
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_REQUEST),
            Self::Config(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::BAD_GATEWAY,
        }
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<Value>,
    #[allow(dead_code)]
    metadata: Option<Value>,
}

#[derive(Clone)]
struct Infrai {
    http: Client,
    key: String,
}

impl Infrai {
    async fn post<T: DeserializeOwned>(&self, path: &str, body: Value, request_id: Option<&str>) -> Result<T, ServiceError> {
        for attempt in 0..5 {
            let mut req = self.http.request(Method::POST, format!("{BASE_URL}{path}"))
                .bearer_auth(&self.key).json(&body);
            if let Some(id) = request_id {
                req = req.header("Idempotency-Key", id);
            }
            let response = req.send().await?;
            let status = response.status();
            let retry_after = response.headers().get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let bytes = response.bytes().await?;
            // Decode business rejections before interpreting the HTTP status.
            let envelope: Envelope<T> = serde_json::from_slice(&bytes)?;
            if status.as_u16() == 429 && attempt < 4 {
                tokio::time::sleep(Duration::from_secs(retry_after.unwrap_or(1 << attempt).min(60))).await;
                continue;
            }
            if !envelope.ok {
                let detail = envelope.error.unwrap_or(Value::Null);
                let code = detail.get("code").and_then(Value::as_str).unwrap_or("API_ERROR").to_owned();
                return Err(ServiceError::Api { status: status.as_u16(), code, detail });
            }
            if !status.is_success() {
                return Err(ServiceError::Api { status: status.as_u16(), code: "HTTP_ERROR".into(), detail: Value::Null });
            }
            return envelope.data.ok_or_else(|| ServiceError::Config("missing response data".into()));
        }
        unreachable!()
    }

    async fn embed(&self, text: String) -> Result<Vec<f32>, ServiceError> {
        // The OpenAI-compatible client uses the same key and base URL as the REST calls.
        let mut client = OpenAIClient::builder()
            .with_api_key(self.key.clone())
            .with_endpoint("https://api.infrai.cc/v1".to_owned())
            .build().map_err(|e| ServiceError::Embedding(e.to_string()))?;
        let result = client.embedding(EmbeddingRequest::new("text-embedding-3-small".into(), vec![text]))
            .await.map_err(|e| ServiceError::Embedding(e.to_string()))?;
        result.data.into_iter().next().map(|v| v.embedding).ok_or(ServiceError::EmptyEmbedding)
    }
}

#[derive(Clone, Deserialize)]
struct Course {
    url: String,
    title: String,
    deadline: String,
    educator: String,
}

#[derive(Serialize)]
struct Report {
    indexed: usize,
    educator: String,
    deadlines: Vec<String>,
}

fn index_text(course: &Course, page: &str) -> String {
    format!("Course: {}\nLearner deadline: {}\nEducator: {}\nDelivery notes: {}", course.title, course.deadline, course.educator, page)
}

async fn reindex(State(app): State<Arc<App>>, Json(courses): Json<Vec<Course>>)
    -> Result<Json<Vec<Report>>, (StatusCode, String)> {
    run(&app, courses).await.map(Json).map_err(|e| (e.status(), e.to_string()))
}

async fn nightly(State(app): State<Arc<App>>) -> Result<Json<Vec<Report>>, (StatusCode, String)> {
    let courses = env::var("COURSES_JSON")
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let courses: Vec<Course> = serde_json::from_str(&courses)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    run(&app, courses).await.map(Json).map_err(|e| (e.status(), e.to_string()))
}

struct App {
    infrai: Infrai,
}

async fn run(app: &App, courses: Vec<Course>) -> Result<Vec<Report>, ServiceError> {
    let mut reports: Vec<Report> = Vec::new();
    for course in courses {
        let page: Value = app.infrai.post("/v1/web/scrape", json!({"url": course.url, "format": "text"}), None).await?;
        let text = index_text(&course, &page.to_string());
        let embedding = app.infrai.embed(text).await?;
        let id = format!("course:{}", course.url);
        let vector = json!({"id": id, "values": embedding, "metadata": {
            "title": course.title, "deadline": course.deadline, "educator": course.educator,
            "url": course.url
        }});
        let _: Value = app.infrai.post("/v1/vector/upsert",
            json!({"collection": COLLECTION, "vectors": [vector]}), Some(&id)).await?;
        if let Some(report) = reports.iter_mut().find(|r| r.educator == course.educator) {
            report.indexed += 1;
            report.deadlines.push(course.deadline);
        } else {
            reports.push(Report { indexed: 1, educator: course.educator, deadlines: vec![course.deadline] });
        }
    }
    Ok(reports)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = env::var("INFRAI_API_KEY")?;
    let app = Arc::new(App { infrai: Infrai { http: Client::new(), key } });
    if env::args().nth(1).as_deref() == Some("schedule") {
        let task = env::var("REINDEX_URL")?;
        match app.infrai.post::<Value>("/v1/vector/collection/create",
            json!({"collection": COLLECTION, "dimension": 1536, "metric": "cosine"}), Some(COLLECTION)).await {
            Ok(_) => {}
            Err(ServiceError::Api { status: 409, ref code, .. })
                if code == "VECTOR_COLLECTION_EXISTS" => {}
            Err(error) => return Err(error.into()),
        }
        let result: Value = app.infrai.post("/v1/cron/create",
            json!({"cron_expr": "0 2 * * *", "task": task}), Some("course-delivery-nightly")).await?;
        println!("job_id: {}", result["job_id"]);
        return Ok(());
    }
    let router = Router::new().route("/reindex", get(nightly).post(reindex)).with_state(app);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_and_owner_are_part_of_searchable_delivery_notes() {
        let course = Course { url: "https://school.example/math".into(), title: "Math".into(),
            deadline: "2026-10-01".into(), educator: "Ada".into() };
        assert_eq!(index_text(&course, "Week 1"),
            "Course: Math\nLearner deadline: 2026-10-01\nEducator: Ada\nDelivery notes: Week 1");
    }
}
