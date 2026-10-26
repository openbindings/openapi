#[tokio::test]
async fn received_responses_and_redirects() {
    // Ten calls per kind; each of the three redirects followed makes a second host call.
    for v30 in [false, true] {
        assert_eq!(openapi_json_path_consumer::responses::suite(v30).await, 13);
    }
}
