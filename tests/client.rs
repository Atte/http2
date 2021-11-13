use http2::{Client, Request};

#[tokio::test]
async fn get_google() {
    let client = Client::default();
    let response = client
        .request(Request::get(
            "https://www.google.com/".try_into().unwrap(),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}
