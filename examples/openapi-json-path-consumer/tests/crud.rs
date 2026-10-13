#[tokio::test]
async fn public_crud_consumers() {
    openapi_json_path_consumer::crud::suite(true).await;
    openapi_json_path_consumer::crud::suite(false).await;
}
