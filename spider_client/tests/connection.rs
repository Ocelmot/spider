
use spider_client::SpiderClient;



#[tokio::test]
async fn connect() {
    let mut client = SpiderClient::new();
    client.connect().await;


}