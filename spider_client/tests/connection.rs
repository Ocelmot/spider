
use spider_client::SpiderClient;



#[tokio::test]
async fn connect() {
    let mut client = SpiderClient::new();
    let mut handle = client.start().await;

    handle.join().await;
}