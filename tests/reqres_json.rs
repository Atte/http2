#![cfg(feature = "json")]
use http2::{Client, Request};

#[derive(Debug, Clone, serde::Serialize)]
struct CreateUserRequest {
    name: String,
    job: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct CreateUserResponse {
    name: String,
    job: String,
    id: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

#[tokio::test]
async fn create_user() {
    let client = Client::default();
    let response = client
        .request(
            Request::post_json(
                "https://reqres.in/api/users/".try_into().unwrap(),
                &CreateUserRequest {
                    name: "morpheus".to_string(),
                    job: "leader".to_string(),
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 201);

    let data: CreateUserResponse = response.json().unwrap();
    assert_eq!(data.name, "morpheus");
    assert_eq!(data.job, "leader");
}
