








use rsa::RsaPrivateKey;
use spider_link::{link::Link, SelfRelation, Role, message::Message};






#[tokio::test]
async fn send_message(){
    // setup base listener
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
    let role = Role::Peer;
    let base_relation = SelfRelation::from_key(priv_key, role);
    let mut listener = Link::listen(base_relation.clone(), "0.0.0.0:1930");

    // setup peripheral link
    let peripheral_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
    let peripheral_role = Role::Peer;
    let peripheral_relation = SelfRelation::from_key(peripheral_key, peripheral_role);
    let mut to_host = Link::connect(peripheral_relation, "127.0.0.1:1930", base_relation.relation).await.expect("failed to connect to base");

    // get link from base listener
    let to_peripheral = listener.recv().await.expect("failed to get new link");

    todo!("Implement message passing test")
    // send messages back and forth!
    // let msg = Message::Event {name: "test".into(), data: "message".as_bytes().to_vec() };
    // to_peripheral.send(msg).await;
    // let recv_msg = to_host.recv().await.expect("link closed");

    // if let Message::Event {name, data } = recv_msg{
    //     assert_eq!(name, String::from("test"));
    //     assert_eq!(data, "message".as_bytes().to_vec());
    // }else{
    //     panic!("incorrect message recieved: {:?}", recv_msg);
    // }
    
}





