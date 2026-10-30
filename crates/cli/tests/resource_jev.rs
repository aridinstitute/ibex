mod common;

use std::io::{Read, Write};

#[test]
fn jev_rejects_insecure_non_loopback_endpoint_as_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "openrouter_api_key = 'sk-secret'\n",
    )
    .unwrap();
    let output = common::ibex()
        .arg("--data")
        .arg(dir.path().join("library"))
        .env("IBEX_CONFIG_DIR", dir.path())
        .env("IBEX_JEV_URL", "http://example.com/api/alpha/decisions")
        .args([
            "resource",
            "classify",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "--engine",
            "jev",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("IBEX_JEV_URL"), "{error}");
    assert!(
        error.contains("HTTPS") && error.contains("loopback"),
        "{error}"
    );
    assert!(!error.contains("sk-secret"));
}

#[test]
fn jev_requires_key_and_combines_predictions_with_keywords() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str], endpoint: Option<&str>| {
        let mut cmd = common::ibex();
        cmd.arg("--data")
            .arg(dir.path())
            .env("IBEX_CONFIG_DIR", dir.path())
            .args(args);
        if let Some(endpoint) = endpoint {
            cmd.env("IBEX_JEV_URL", endpoint);
        }
        cmd.output().unwrap()
    };
    let person = run(&["person", "add", "--last", "Lovelace"], None);
    let person_id = String::from_utf8(person.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let collection = run(&["collection", "add", "Jev pick"], None);
    let collection_id = String::from_utf8(collection.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert!(
        run(&["collection", "update", &collection_id, "--jev"], None)
            .status
            .success()
    );
    let keyword = run(&["collection", "add", "Keyword pick"], None);
    let keyword_id = String::from_utf8(keyword.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert!(
        run(
            &[
                "collection",
                "update",
                &keyword_id,
                "--keyword",
                "analytical"
            ],
            None
        )
        .status
        .success()
    );
    let resource = run(
        &[
            "resource",
            "add",
            "--title",
            "Analytical Engine",
            "--creator",
            &format!("{person_id}:author"),
        ],
        None,
    );
    let id = String::from_utf8(resource.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let missing = run(&["resource", "classify", &id, "--engine", "jev"], None);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("OpenRouter"));
    std::fs::write(
        dir.path().join("config.toml"),
        "openrouter_api_key = 'sk-secret'\n",
    )
    .unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/api/alpha/decisions",
        listener.local_addr().unwrap()
    );
    let body =
        serde_json::json!({"answers": {collection_id.clone(): {"type": "noul", "noul": 0.95}}})
            .to_string();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let n = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..n]);
            if let Some(end) = String::from_utf8_lossy(&request).find("\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
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
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });
    let classified = run(
        &["resource", "classify", &id, "--engine", "both", "--apply"],
        Some(&url),
    );
    assert!(
        classified.status.success(),
        "{}",
        String::from_utf8_lossy(&classified.stderr)
    );
    let output = String::from_utf8(classified.stdout).unwrap();
    assert!(
        output.contains("Jev pick") && output.contains("Keyword pick"),
        "{output}"
    );
    assert!(output.find("Jev pick").unwrap() < output.find("Keyword pick").unwrap());
    let request = server.join().unwrap();
    assert!(request.contains("sk-secret"));
    let detail = String::from_utf8(run(&["resource", "show", &id], None).stdout).unwrap();
    assert!(detail.contains("Jev pick") && detail.contains("Keyword pick"));
}
