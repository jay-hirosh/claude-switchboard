use claude_switchboard_lib::providers::ollama::fetch_models;
use mockito::Server;

#[tokio::test]
async fn returns_sorted_model_names_on_200() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/tags")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"models":[{"name":"llama3.2:latest"},{"name":"gpt-oss:20b"}]}"#)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let models = fetch_models(&client, &server.url()).await.unwrap();
    assert_eq!(models, vec!["gpt-oss:20b".to_string(), "llama3.2:latest".to_string()]);
}

#[tokio::test]
async fn trailing_slash_on_base_url_does_not_double_up() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/api/tags")
        .with_status(200)
        .with_body(r#"{"models":[]}"#)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let base_url = format!("{}/", server.url());
    assert!(fetch_models(&client, &base_url).await.is_ok());
}

#[tokio::test]
async fn non_success_status_is_an_error() {
    let mut server = Server::new_async().await;
    let _m = server.mock("GET", "/api/tags").with_status(404).create_async().await;

    let client = reqwest::Client::new();
    assert!(fetch_models(&client, &server.url()).await.is_err());
}

#[tokio::test]
async fn unreachable_server_is_an_error() {
    let client = reqwest::Client::new();
    // Port 0 never accepts connections — this exercises the connection-error
    // path, not just non-2xx statuses.
    assert!(fetch_models(&client, "http://127.0.0.1:0").await.is_err());
}
