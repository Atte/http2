use http2::{Client, Request};

#[tokio::test]
async fn delete_user() {
    let client = Client::default();
    let response = client
        .request(Request::delete(
            "https://reqres.in/api/users/2".try_into().unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
}

#[tokio::test]
async fn interleaved() {
    let client = Client::default();
    let (response1, response2) = tokio::join!(
        client.request(Request::get(
            "https://reqres.in/api/users/1?delay=2".try_into().unwrap(),
        )),
        client.request(Request::get(
            "https://reqres.in/api/users/2".try_into().unwrap(),
        ))
    );
    let (response1, response2) = (response1.unwrap(), response2.unwrap());
    assert_eq!(response1.status(), 200);
    assert_eq!(response2.status(), 200);
    assert!(response1.text().contains(r#""id":1"#));
    assert!(response2.text().contains(r#""id":2"#));
}
