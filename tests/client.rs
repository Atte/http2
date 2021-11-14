use http2::{Client, Request};

#[tokio::test]
#[ignore]
async fn google_redirect() {
    let client = Client::default();
    let request = Request::get("https://google.com/".try_into().unwrap());
    let response = client.request(request.clone()).await.unwrap();
    let response = client
        .request(dbg!(request.redirect(&response)).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn example_com() {
    let client = Client::default();
    let response = client
        .request(Request::get("https://example.com/".try_into().unwrap()))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response
        .text()
        .contains("This domain is for use in illustrative examples in documents."));
}
