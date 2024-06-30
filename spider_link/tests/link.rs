use std::collections::HashMap;

use log::info;
use rsa::RsaPrivateKey;
use spider_link::{
    id::SpiderId,
    link::Link,
    message::{AbsoluteDatasetPath, DatasetData, DatasetPath, Message, UiElement, UiElementKind},
    Role, SelfRelation,
};

#[tokio::test]
async fn send_message() {
    // setup base listener
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
    let role = Role::Peer;
    let base_relation = SelfRelation::from_key(priv_key, role);
    let (mut listener, _) = Link::listen(base_relation.clone(), "0.0.0.0:1930");

    // setup peripheral link
    let peripheral_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
    let peripheral_role = Role::Peer;
    let peripheral_relation = SelfRelation::from_key(peripheral_key, peripheral_role);
    let mut to_host = Link::connect(
        peripheral_relation,
        "127.0.0.1:1930",
        base_relation.relation,
    )
    .await
    .expect("failed to connect to base");

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

#[test]
fn test_ui_element_dataset_iterator() {
    let mut data_map: HashMap<AbsoluteDatasetPath, Vec<DatasetData>> = HashMap::new();

    let path = AbsoluteDatasetPath::new_public(vec!["test".into()]);
    let mut dataset = vec![
        DatasetData::String("data 1".into()),
        DatasetData::String("data 2".into()),
    ];
    dataset.push(DatasetData::String("data 3".into()));
    data_map.insert(path.clone(), dataset);

    let mut elem = UiElement::new(UiElementKind::Rows);
    elem.append_child({ UiElement::from_string("Child 1") });
    // elem.append_child({
    //     UiElement::from_string("Child 2")
    // });
    // elem.append_child({
    //     UiElement::from_string("Child 3")
    // });

    elem.set_dataset(Some(path.clone()));

    info!("===== plain iteration =====");
    for (dataset_index, child, datum) in elem.children_dataset(&None, &data_map) {
        info!("idx: {:?}", dataset_index);
        info!("child:{:?}", child.render_content_opt(&datum));
        info!("datum: {:?}", datum);
    }

    info!("===== take 1 =====");
    for (dataset_index, child, datum) in elem.children_dataset(&None, &data_map).take(1) {
        info!("idx: {:?}", dataset_index);
        info!("child:{:?}", child.render_content_opt(&datum));
        info!("datum: {:?}", datum);
    }

    info!("===== collect =====");
    let mut iter = elem.children_dataset(&None, &data_map).take(1).rev();
    info!("{:?}", iter.size_hint());
    info!("{:?}", iter.len());
    info!("{:?}", iter.next_back());

    info!("===== manual =====");
    let mut iter = elem.children_dataset(&None, &data_map);
    info!("len: {:?}", iter.len());
    info!("size_hint: {:?}", iter.size_hint());
    info!("count {:?}", iter.count());
    // info!("next: {:?}", iter.next());
    // info!("next back: {:?}", iter.next_back());
    // info!("next: {:?}", iter.next());
    // info!("next: {:?}", iter.next());
}

#[test]
fn test_iter() {
    let v = vec![0, 1, 2, 3];
    info!("size_hint: {:?}", v.iter().size_hint());

    for val in v.iter().take(1).rev() {
        info!("val: {:?}", val);
    }
}
