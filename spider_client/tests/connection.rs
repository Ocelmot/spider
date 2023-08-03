use spider_client::SpiderClientBuilder;
use spider_link::{
    message::{DatasetData, Message, RouterMessage},
    Link, Role, SelfRelation,
};

#[tokio::test]
async fn connect() {
    let host_relation = SelfRelation::generate_key(Role::Peer);
    let host_relation_relation = host_relation.relation.clone();
    let (mut host, _) = Link::listen(host_relation, "127.0.0.1:1950");

    let mut client_builder = SpiderClientBuilder::new();
    client_builder.enable_beacon(false);
    client_builder.enable_chord(false);
    client_builder.enable_last_addr(false);
    client_builder.set_fixed_addrs(vec![String::from("127.0.0.1:1950")]);
    client_builder.enable_fixed_addrs(true);
    client_builder.set_host_relation(host_relation_relation.clone());
    let client = client_builder.start(false);

    let event = RouterMessage::Event(
        String::from("test"),
        host_relation_relation.clone(),
        DatasetData::String(String::from("Test Data!")),
    );
    client.send(Message::Router(event)).await;

    let mut host_link = host.recv().await.expect("Failed to get Link");

    match host_link.recv().await {
        Some(Message::Router(RouterMessage::Event(
            kind,
            relation,
            DatasetData::String(string),
        ))) => {
            assert_eq!(kind, "test");
            assert_eq!(relation, host_relation_relation);
            assert_eq!(string, String::from("Test Data!"));
        }
        None => {
            panic!("Did not recieve message!");
        }
        _ => {
            panic!("Recieved incorrect data");
        }
    }
}
