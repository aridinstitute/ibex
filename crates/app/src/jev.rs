//! Shared Jev classification client.
//! Jev collection classification through OpenRouter's Decisions API.
//! Network access runs only on the DB worker runtime. The API key is sent
//! as a bearer token and never appears in errors or logs.
use std::{collections::HashMap, time::Duration};

use ibex_core::{Collection, CollectionId, ResourceDetail};
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const DEFAULT_JEV_MODEL: &str = "typesafe/jev-1.14";
/// Minimum `noul` probability for a collection to be suggested.
pub const JEV_THRESHOLD: f32 = 0.5;
pub const DECISIONS_URL: &str = "https://openrouter.ai/api/alpha/decisions";
const MAX_CANDIDATES: usize = 100;
const MAX_SUGGESTIONS: usize = 5;
/// Keeps the decision state well under Jev's default-mode size limit.
const ABSTRACT_MAX_CHARS: usize = 2_000;

/// Ranked Jev picks: probability descending, at most five.
pub type JevSuggestions = Vec<(CollectionId, f32)>;

/// POST one decision to `url` (normally [`DECISIONS_URL`]).
pub async fn classify_at(
    url: &str,
    detail: &ResourceDetail,
    candidates: &[Collection],
    key: &str,
    model: &str,
) -> Result<JevSuggestions, String> {
    if candidates.is_empty() {
        return Err("No collections are included in auto-classify".into());
    }
    if candidates.len() > MAX_CANDIDATES {
        return Err(format!(
            "Auto-classify supports at most {MAX_CANDIDATES} collections; {} are included",
            candidates.len()
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!(
            "Ibex/",
            env!("CARGO_PKG_VERSION"),
            " (jev classification)"
        ))
        .build()
        .map_err(|_| "Could not start the auto-classify request".to_owned())?;
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&request_body(detail, candidates, model))
        .send()
        .await
        // reqwest errors can embed the URL; keep the message fixed instead.
        .map_err(|_| "Auto-classify failed; try again shortly".to_owned())?;
    if let Some(error) = status_error(response.status()) {
        return Err(error);
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| "The classification model returned an unreadable response".to_owned())?;
    parse_answers(&body, candidates)
}

fn status_error(status: reqwest::StatusCode) -> Option<String> {
    if status.is_success() {
        return None;
    }
    Some(
        match status.as_u16() {
            401 | 403 => "OpenRouter rejected the API key",
            402 => "OpenRouter reports insufficient credits",
            429 => "OpenRouter rate limit reached; try again shortly",
            413 => "Classification request too large",
            _ => "Auto-classify failed; try again shortly",
        }
        .to_owned(),
    )
}

fn byline(detail: &ResourceDetail) -> Option<String> {
    detail
        .creators
        .iter()
        .find(|group| group.role == ibex_core::CreatorRole::Author)
        .or_else(|| detail.creators.first())
        .map(|group| group.names.join(", "))
}

fn request_body(detail: &ResourceDetail, candidates: &[Collection], model: &str) -> Value {
    let mut state = Map::new();
    state.insert("title".into(), json!(detail.title.as_str()));
    if let Some(byline) = byline(detail) {
        state.insert("authors".into(), json!(byline));
    }
    if let Some(text) = &detail.abstract_text {
        state.insert(
            "abstract".into(),
            json!(text.chars().take(ABSTRACT_MAX_CHARS).collect::<String>()),
        );
    }
    if !detail.keywords.is_empty() {
        state.insert("keywords".into(), json!(detail.keywords.join(", ")));
    }
    state.insert("kind".into(), json!(detail.kind.as_str()));
    let questions: Map<String, Value> = candidates
        .iter()
        .map(|collection| {
            let name = collection.name.as_str();
            let scope = if collection.keywords.is_empty() {
                format!("\"{name}\"")
            } else {
                format!("\"{name}\" ({})", collection.keywords.join(", "))
            };
            (
                collection.id.to_string(),
                json!({
                    "type": "noul",
                    "instructions": format!("Does this resource belong in the collection \"{name}\"?"),
                    "criteria": {
                        "true": format!("The resource matches the scope of the collection {scope}"),
                        "false": format!("The resource is unrelated to the scope of the collection {scope}"),
                    },
                }),
            )
        })
        .collect();
    json!({ "model": model, "state": state, "questions": questions })
}

#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "type")]
    kind: String,
    noul: Option<f64>,
}

fn parse_answers(body: &Value, candidates: &[Collection]) -> Result<JevSuggestions, String> {
    let answers: HashMap<String, Value> = body
        .get("answers")
        .cloned()
        .and_then(|answers| serde_json::from_value(answers).ok())
        .ok_or_else(|| "The classification model returned an unexpected response".to_owned())?;
    let mut picks: JevSuggestions = answers
        .into_iter()
        .filter_map(|(key, answer)| {
            let answer: Answer = serde_json::from_value(answer).ok()?;
            let p = answer.noul.filter(|p| p.is_finite())?.clamp(0., 1.) as f32;
            let id = candidates
                .iter()
                .find(|c| c.id.to_string() == key)
                .map(|c| c.id)?;
            (answer.kind == "noul" && p >= JEV_THRESHOLD).then_some((id, p))
        })
        .collect();
    picks.sort_by(|a, b| b.1.total_cmp(&a.1));
    picks.truncate(MAX_SUGGESTIONS);
    Ok(picks)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ibex_core::{
        CreatorRole, DetailCreators, NonEmptyStr, ResourceId, ResourceKind, UtcMillis,
    };

    fn collection(name: &str, keywords: &[&str]) -> Collection {
        let mut c = Collection::try_new(
            CollectionId::generate(),
            NonEmptyStr::try_new(name).unwrap(),
        );
        c.keywords = keywords.iter().map(|k| (*k).to_owned()).collect();
        c.jev_enabled = true;
        c
    }

    fn detail() -> ResourceDetail {
        ResourceDetail {
            id: ResourceId::generate(),
            kind: ResourceKind::JournalArticle,
            title: NonEmptyStr::try_new("Mapping citation networks").unwrap(),
            creators: vec![DetailCreators {
                role: CreatorRole::Author,
                names: vec!["Ada Lovelace".into(), "Grace Hopper".into()],
            }],
            publisher: None,
            published: None,
            accessed: None,
            container_title: None,
            series_title: None,
            series_number: None,
            favorite: false,
            tags: vec![],
            collections: vec![],
            attachments: vec![],
            date_added: UtcMillis::from_unix_ms(0),
            date_edited: UtcMillis::from_unix_ms(0),
            subtitle: None,
            title_addon: None,
            short_title: None,
            place: None,
            volume: None,
            issue: None,
            pages: None,
            total_pages: None,
            article_number: None,
            edition: None,
            doi: None,
            url: None,
            eprint: None,
            isbns: vec![],
            issns: vec![],
            isrns: vec![],
            original_date: None,
            event_date: None,
            language: None,
            document_type: None,
            publication_method: None,
            institution: None,
            organization: None,
            school: None,
            abstract_text: Some("x".repeat(5_000)),
            keywords: vec!["bibliometrics".into()],
        }
    }

    /// One-shot HTTP server returning `status` and `body`; the join handle
    /// yields the raw request.
    pub(crate) fn serve(
        body: String,
        status: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!(
            "http://{}/api/alpha/decisions",
            listener.local_addr().unwrap()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 8192];
            // Read headers, then exactly Content-Length body bytes.
            loop {
                let n = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (address, handle)
    }

    #[tokio::test]
    async fn sends_one_noul_question_per_collection_and_ranks_answers() {
        let networks = collection("Networks", &["citation networks"]);
        let methods = collection("Methods", &[]);
        let history = collection("History", &[]);
        let candidates = vec![networks.clone(), methods.clone(), history.clone()];
        let body = json!({"answers": {
            networks.id.to_string(): {"type": "noul", "noul": 0.71},
            methods.id.to_string(): {"type": "noul", "noul": 0.93},
            history.id.to_string(): {"type": "noul", "noul": 0.2},
            CollectionId::generate().to_string(): {"type": "noul", "noul": 0.99},
        }})
        .to_string();
        let (url, handle) = serve(body, "200 OK");
        let picks = classify_at(
            &url,
            &detail(),
            &candidates,
            "sk-or-secret",
            DEFAULT_JEV_MODEL,
        )
        .await
        .unwrap();
        assert_eq!(picks, vec![(methods.id, 0.93), (networks.id, 0.71)]);
        let request = handle.join().unwrap();
        assert!(request.starts_with("POST /api/alpha/decisions"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer sk-or-secret")
        );
        assert!(request.contains("(jev classification)"));
        let json: Value =
            serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(json["model"], DEFAULT_JEV_MODEL);
        assert_eq!(json["state"]["title"], "Mapping citation networks");
        assert_eq!(json["state"]["authors"], "Ada Lovelace, Grace Hopper");
        assert_eq!(json["state"]["kind"], "journalArticle");
        assert_eq!(
            json["state"]["abstract"].as_str().unwrap().chars().count(),
            ABSTRACT_MAX_CHARS
        );
        let questions = json["questions"].as_object().unwrap();
        assert_eq!(questions.len(), 3);
        let networks_question = &questions[&networks.id.to_string()];
        assert_eq!(networks_question["type"], "noul");
        assert!(
            networks_question["instructions"]
                .as_str()
                .unwrap()
                .contains("\"Networks\"")
        );
        assert!(
            networks_question["criteria"]["true"]
                .as_str()
                .unwrap()
                .contains("citation networks")
        );
    }

    #[test]
    fn answers_are_validated_thresholded_and_capped() {
        let candidates: Vec<_> = (0..8).map(|i| collection(&format!("C{i}"), &[])).collect();
        let mut answers = Map::new();
        for (i, c) in candidates.iter().enumerate() {
            answers.insert(
                c.id.to_string(),
                json!({"type": "noul", "noul": 0.5 + i as f64 * 0.05}),
            );
        }
        let picks = parse_answers(&json!({ "answers": answers }), &candidates).unwrap();
        assert_eq!(picks.len(), MAX_SUGGESTIONS);
        assert_eq!(picks[0].0, candidates[7].id);
        assert!(picks.windows(2).all(|w| w[0].1 >= w[1].1));

        let c = &candidates[0];
        let body = json!({"answers": {c.id.to_string(): {"type": "noul", "noul": 1.7}}});
        assert_eq!(
            parse_answers(&body, &candidates).unwrap(),
            vec![(c.id, 1.0)]
        );
        for malformed in [
            json!({"answers": {c.id.to_string(): {"type": "yesno", "noul": 0.9}}}),
            json!({"answers": {c.id.to_string(): {"type": "noul", "noul": "high"}}}),
            json!({"answers": {c.id.to_string(): {"type": "noul"}}}),
            json!({"answers": {c.id.to_string(): 0.9}}),
        ] {
            assert!(parse_answers(&malformed, &candidates).unwrap().is_empty());
        }
        assert!(parse_answers(&json!({"choices": []}), &candidates).is_err());
        assert!(parse_answers(&json!({"answers": [0.9]}), &candidates).is_err());
    }

    #[tokio::test]
    async fn http_errors_map_to_friendly_messages_without_the_key() {
        let candidates = vec![collection("Networks", &[])];
        for (status, expected) in [
            ("401 Unauthorized", "rejected the API key"),
            ("402 Payment Required", "insufficient credits"),
            ("429 Too Many Requests", "rate limit"),
            ("413 Payload Too Large", "too large"),
            ("503 Service Unavailable", "try again shortly"),
        ] {
            let (url, handle) = serve("{}".into(), status);
            let error = classify_at(&url, &detail(), &candidates, "sk-or-secret", "m")
                .await
                .unwrap_err();
            handle.join().unwrap();
            assert!(error.contains(expected), "{status}: {error}");
            assert!(!error.contains("sk-or-secret"));
        }
        let (url, handle) = serve("not json".into(), "200 OK");
        assert!(
            classify_at(&url, &detail(), &candidates, "k", "m")
                .await
                .unwrap_err()
                .contains("unreadable")
        );
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn candidate_limits_fail_before_any_request() {
        let none = classify_at("http://unused.invalid", &detail(), &[], "k", "m").await;
        assert!(none.unwrap_err().contains("No collections"));
        let many: Vec<_> = (0..101)
            .map(|i| collection(&format!("C{i}"), &[]))
            .collect();
        let error = classify_at("http://unused.invalid", &detail(), &many, "k", "m")
            .await
            .unwrap_err();
        assert!(error.contains("at most 100"));
    }
}
