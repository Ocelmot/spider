
use std::sync::Arc;

use spider_client::{
    link::{
         message::{DatasetData, Message, RouterMessage}, SelfRelation
    }, ClientResponse, SpiderClientBuilder
};
use spider_link::{
    discovery::{BaseAdvert, beacon::start_beacon_listen_handler},
    link_set::{Epoch, LinkSet, LinkSetMessage, links::Address},
    transports::{LinkListener, tcp::{TCP_SCHEME, TcpListener}},
};
use tokio::sync::{Mutex, mpsc::channel, watch};
use tracing::info;
use tracing_test::traced_test;
use serial_test::serial;

#[tokio::test]
#[traced_test]
#[serial]
async fn connect() {
    let host_relation = SelfRelation::debug_get(0);
    let client_relation = SelfRelation::debug_get(1);

    let listen_addr = "127.0.0.1:1950";
    info!("Starting listener");
    let listener = TcpListener::new(listen_addr.to_string(), Arc::new(Mutex::new(None)));
    let (listen_tx, mut listen_rx) = channel(10);
    listener.listen(host_relation.clone(), listen_tx.clone());

    let mut client_builder = SpiderClientBuilder::new_with_self_relation(Some("".into()), client_relation.clone());
    client_builder.enable_discovery(false);
    client_builder.disable_veilid();
    client_builder.enable_transport(TCP_SCHEME.to_owned());
    client_builder.set_fixed_addrs(vec![Address::new(TCP_SCHEME, listen_addr)]);
    client_builder.enable_fixed_addrs(true);
    client_builder.set_host_relation(host_relation.relation.clone());
    info!("Starting client");
    let client = client_builder.start(false).await.expect("Client should start");

    let event_name = String::from("test");
    let event_rel = host_relation.relation.clone();
    let event_data = DatasetData::String(String::from("Test Data!"));
    let event = RouterMessage::Event(
        event_name.clone(),
        event_rel.clone(),
        event_data.clone(),
    );
    info!("Sending message");
    client.send(Message::Router(event)).await.expect("client should be started");

    info!("Setting up listen link set");
    let host_authed = listen_rx.recv().await.expect("listener failed to get Link");
    let (_host_rel, host_link) = host_authed.into_parts();
    let mut listen_link_set = LinkSet::new();
    listen_link_set.add_link_boxed(host_link).await.unwrap();

    info!("Receiving connect message");
    let msg = listen_link_set.recv().await.expect("listen link closed");
    let LinkSetMessage::Connected(epoch) = msg else {panic!("Incorrect message")};
    assert_eq!(epoch, Epoch::ONE);

    info!("Receiving sent message");
    let msg = listen_link_set.recv().await.expect("listen link closed");
    let LinkSetMessage::Message(msg, epoch) = msg else {panic!("Incorrect message");};
    assert_eq!(epoch, Epoch::ONE);
    let Message::Router(msg) = msg else{panic!("Incorrect message type");};
    let RouterMessage::Event(name, rel, _) = msg else{panic!("Incorrect message type");};
    
    assert_eq!(name, event_name);
    assert_eq!(rel, event_rel);
}

// Wrong discoverer chosen on ios
#[cfg(all(feature = "discovery", not( target_os = "ios")))]
#[tokio::test]
#[traced_test]
#[serial]
async fn beacon_connect() {
    let host_relation = SelfRelation::debug_get(0);
    let client_relation = SelfRelation::debug_get(1);
    let (_template_tx, template_rx) = watch::channel(BaseAdvert::default());
    
    start_beacon_listen_handler(template_rx, 1960);

    let listen_addr = "127.0.0.1:1960";
    info!("Starting listener");
    let listener = TcpListener::new(listen_addr.to_string(), Arc::new(Mutex::new(None)));
    let (listen_tx, mut listen_rx) = channel(10); 
    listener.listen(host_relation.clone(), listen_tx.clone());

    let mut client_builder = SpiderClientBuilder::new_with_self_relation(Some("".into()), client_relation.clone());
    client_builder.enable_discovery(true);
    client_builder.disable_veilid();
    client_builder.enable_transport(TCP_SCHEME.to_owned());
    client_builder.enable_fixed_addrs(false);
    client_builder.set_host_relation(host_relation.relation.clone());
    info!("Starting client");
    let client = client_builder.start(false).await.expect("Client should start");

    let event_name = String::from("test");
    let event_rel = host_relation.relation.clone();
    let event_data = DatasetData::String(String::from("Test Data!"));
    let event = RouterMessage::Event(
        event_name.clone(),
        event_rel.clone(),
        event_data.clone(),
    );
    info!("Sending message");
    client.send(Message::Router(event)).await.expect("client should be started");

    info!("Setting up listen link set");
    let host_authed = listen_rx.recv().await.expect("listener failed to get Link");
    let (_host_rel, host_link) = host_authed.into_parts();
    let mut listen_link_set = LinkSet::new();
    listen_link_set.add_link_boxed(host_link).await.unwrap();

    info!("Receiving connect message");
    let msg = listen_link_set.recv().await.expect("listen link closed");
    let LinkSetMessage::Connected(epoch) = msg else {panic!("Incorrect message")};
    assert_eq!(epoch, Epoch::ONE);

    info!("Receiving sent message");
    let msg = listen_link_set.recv().await.expect("listen link closed");
    let LinkSetMessage::Message(msg, epoch) = msg else {panic!("Incorrect message");};
    assert_eq!(epoch, Epoch::ONE);
    let Message::Router(msg) = msg else{panic!("Incorrect message type")};
    let RouterMessage::Event(name, rel, _) = msg else{panic!("Incorrect message type");};
    
    assert_eq!(name, event_name);
    assert_eq!(rel, event_rel);
}


#[tokio::test]
#[traced_test]
#[serial]
async fn client_round_trip() {
    let host_relation = SelfRelation::debug_get(0);
    let client_relation = SelfRelation::debug_get(1);

    let listen_addr = "127.0.0.1:1970";
    info!("Starting listener");
    let listener = TcpListener::new(listen_addr.to_string(), Arc::new(Mutex::new(None)));
    let (listen_tx, mut listen_rx) = channel(10); 
    listener.listen(host_relation.clone(), listen_tx.clone());

    let mut client_builder = SpiderClientBuilder::new_with_self_relation(Some("".into()), client_relation.clone());
    client_builder.enable_discovery(false);
    client_builder.disable_veilid();
    client_builder.enable_transport(TCP_SCHEME.to_owned());
    client_builder.set_fixed_addrs(vec![Address::new(TCP_SCHEME, listen_addr)]);
    client_builder.enable_fixed_addrs(true);
    client_builder.set_host_relation(host_relation.relation.clone());
    info!("Starting client");
    let mut client = client_builder.start(true).await.expect("Client should start");

    let event_name = String::from("test");
    let event_rel = host_relation.relation.clone();
    let event_data = DatasetData::String(String::from("Test Data!"));
    let event = RouterMessage::Event(
        event_name.clone(),
        event_rel.clone(),
        event_data.clone(),
    );
    info!("Sending message to listener");
    client.send(Message::Router(event)).await.expect("Client should be started");

    info!("Setting up listen link set");
    let host_authed = listen_rx.recv().await.expect("listener failed to get Link");
    let (_host_rel, host_link) = host_authed.into_parts();
    let mut listen_link_set = LinkSet::new();
    listen_link_set.add_link_boxed(host_link).await.unwrap();

    // From client to listener

    info!("Listener receiving connect message");
    let msg = listen_link_set.recv().await.expect("listen link closed");
    let LinkSetMessage::Connected(epoch) = msg else {panic!("Incorrect message")};
    assert_eq!(epoch, Epoch::ONE);

    info!("Listener receiving sent message");
    let msg = listen_link_set.recv().await.expect("listen link closed");
    let LinkSetMessage::Message(msg, epoch) = msg else {panic!("Incorrect message");};
    assert_eq!(epoch, Epoch::ONE);
    let Message::Router(msg) = msg else{panic!("Incorrect message type");};
    let RouterMessage::Event(name, rel, _) = msg else{panic!("Incorrect message type");};
    
    assert_eq!(name, event_name);
    assert_eq!(rel, event_rel);
    
    // From listener to client
    let event2_name = String::from("test2");
    let event2_rel = client_relation.relation.clone();
    let event2_data = DatasetData::String(String::from("Test Data!"));
    let event2 = RouterMessage::Event(
        event2_name.clone(),
        event2_rel.clone(),
        event2_data.clone(),
    );
    info!("Sending message to client");
    listen_link_set.send(Message::Router(event2)).await.expect("Listener should be able to send");

    info!("Client receiving connect message");
    let msg = client.recv().await.expect("client link closed");
    let ClientResponse::Connected(_conn_epoch) = msg else {panic!("Incorrect message")};

    info!("Client receiving sent message");
    let msg = client.recv().await.expect("client link closed");
    let ClientResponse::Message(msg, _) = msg else {panic!("Incorrect message");};

    let Message::Router(msg) = msg else{panic!("Incorrect message type");};
    let RouterMessage::Event(name, rel, _) = msg else{panic!("Incorrect message type");};
    
    assert_eq!(name, event2_name);
    assert_eq!(rel, event2_rel);
}
