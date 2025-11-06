use std::{sync::Arc, time::Duration};

use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::info;
use tracing_test::traced_test;

use crate::{
    link::{
        impls::{PipeLinkBuilder, PipeLinkHub},
        LinkSet, LinkSetMsg,
    },
    message::Message,
    LinkError, SelfRelation,
};

#[tokio::test]
#[traced_test]
async fn test_link_set_round_trip() {
    let rel1 = SelfRelation::debug_get(0);
    let rel2 = SelfRelation::debug_get(1);
    info!("running test");
    let pb = PipeLinkBuilder::new().max_size(5);
    let (link1, link2) = pb.create_pair(rel1.clone(), rel2.clone());
    let mut link_set1 = LinkSet::new(rel1.clone(), rel2.relation.clone());
    info!("adding link 1");
    link_set1
        .add_link(link1)
        .await
        .expect("Link should be started and running");
    let mut link_set2 = LinkSet::new(rel2, rel1.relation);
    info!("adding link 2");
    link_set2
        .add_link(link2)
        .await
        .expect("Link should be started and running");

    // Send message
    let msg_text_1 = String::from("Test message #1");
    let msg = Message::Error(msg_text_1.clone());
    let msg_send = msg.clone();
    let msg_text_2 = String::from("Test message #2");
    let msg2 = Message::Error(msg_text_2.clone());
    let msg_send2 = msg2.clone();

    info!("sending message 1");
    link_set1.send(msg_send).await.expect("send should succeed");
    info!("sending message 2");
    link_set2
        .send(msg_send2)
        .await
        .expect("send should succeed");

    info!("receiving message 1");
    let msg_recv = link_set2.recv().await.expect("should recv message");
    info!("recvd 1 msg {:?}", msg_recv);
    let LinkSetMsg::Connected(_) = msg_recv else {
        panic!("failed to recv connected message")
    };
    let msg_recv = link_set2.recv().await.expect("should recv message");
    info!("recvd 2 msg {:?}", msg_recv);
    let LinkSetMsg::Message(Message::Error(msg_recv_txt), _) = msg_recv else {
        panic!("received incorrect message type")
    };

    let msg_recv2 = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Connected(_) = msg_recv2 else {
        panic!("failed to recv connected message")
    };
    info!("receiving message 2");
    let msg_recv2 = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Message(Message::Error(msg_recv_txt2), _) = msg_recv2 else {
        panic!("received incorrect message type")
    };

    assert_eq!(msg_text_1, msg_recv_txt);
    assert_eq!(msg_text_2, msg_recv_txt2);
}

#[tokio::test]
#[traced_test]
async fn link_set_timeout() {
    let rel1 = SelfRelation::debug_get(0);
    let rel2 = SelfRelation::debug_get(1);
    info!("running test");
    let pb = PipeLinkBuilder::new()
        .max_size(5)
        .expiration(Some(Duration::from_secs(8)));
    let (link1, link2) = pb.create_pair(rel1.clone(), rel2.clone());

    // Set up first link set
    let link_set1 = LinkSet::new(rel1.clone(), rel2.relation.clone());
    info!("adding link 1");
    link_set1
        .add_link(link1)
        .await
        .expect("Link should be started and running");

    // Set up second link set
    let mut link_set2 = LinkSet::new(rel2, rel1.relation);
    info!("adding link 2");
    link_set2
        .add_link(link2)
        .await
        .expect("Link should be started and running");

    // Send message
    let msg_text_1 = String::from("Test message #1");
    let msg = Message::Error(msg_text_1.clone());
    let msg_send = msg.clone();
    link_set1.send(msg_send).await.expect("send should succeed");

    // Wait for link to disconnect
    sleep(Duration::from_secs(20)).await;

    // message sequence should be Connected -> Message -> Disconnected
    let msg_recv = link_set2.recv().await.expect("should recv message");
    let LinkSetMsg::Connected(_) = msg_recv else {
        panic!("failed to recv connected message")
    };

    let msg_recv = link_set2.recv().await.expect("should recv message");
    info!("recvd msg {:?}", msg_recv);
    let LinkSetMsg::Message(Message::Error(msg_recv_txt), _) = msg_recv else {
        panic!("received incorrect message type")
    };

    let msg_recv = link_set2.recv().await.expect("should recv message");
    let LinkSetMsg::Disconnected = msg_recv else {
        panic!("failed to recv connected message")
    };

    assert_eq!(msg_text_1, msg_recv_txt);
}

#[tokio::test]
#[traced_test]
async fn link_set_connect() {
    info!("running test");
    let rel1 = SelfRelation::debug_get(0);
    let rel2 = SelfRelation::debug_get(1);

    let pb = PipeLinkBuilder::new()
        .max_size(5)
        .expiration(Some(Duration::from_secs(3)));
    let mut plh = PipeLinkHub::new(pb);

    // Set up first link set
    let mut link_set1 = LinkSet::new(rel1.clone(), rel2.relation.clone());

    // setup listener for link set 1
    info!("Setting up listener");
    let mut listener = plh.listen(rel1.clone(), rel1.relation.sha256());
    let link_set1_sender = link_set1.clone_sender();
    tokio::spawn(async move {
        loop {
            if let Some(link) = listener.recv().await {
                info!("listener got new link");
                link_set1_sender
                    .add_link(link)
                    .await
                    .unwrap();
            } else {
                break;
            }
        }
    });

    // wrap plh in arc mutex
    let plh = Arc::new(Mutex::new(plh));
    // Set up second link set
    let link_set2 = LinkSet::new(rel2, rel1.relation.clone());
    info!("adding link 2");
    link_set2
        .add_connector(move |sr, _r, addr: String| {
            info!("Connector called with addr {}", &addr);
            let plh_inner = plh.clone();
            async move {
                let mut plh = plh_inner.lock().await;
                let x = plh.connect(sr, &addr).await;
                x.ok_or(LinkError::new())
            }
        })
        .await
        .expect("Link should be started and running");
    link_set2.add_addr(rel1.relation.sha256()).await.unwrap();

    // Send message
    info!("sending message 1");
    let msg_text_1 = String::from("Test message #1");
    let msg = Message::Error(msg_text_1.clone());
    let msg_send = msg.clone();
    link_set2.send(msg_send).await.expect("send should succeed");

    // Wait long enough for link to disconnect
    info!("sleeping");
    sleep(Duration::from_secs(31)).await;

    // send another message to reconnect
    info!("sending message 2");
    let msg_text_2 = String::from("Test message #2");
    let msg2 = Message::Error(msg_text_2.clone());
    let msg2_send = msg2.clone();
    link_set2
        .send(msg2_send)
        .await
        .expect("send should succeed");

    // message sequence should be Connected -> Message -> Disconnected -> Connected -> Message
    info!("reading connect message");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Connected(_) = msg_recv else {
        panic!("failed to recv connected message")
    };

    info!("reading message 1");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    info!("recvd msg {:?}", msg_recv);
    let LinkSetMsg::Message(Message::Error(msg_recv_txt1), _) = msg_recv else {
        panic!("received incorrect message type")
    };

    info!("reading disconnect message");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Disconnected = msg_recv else {
        panic!("failed to recv connected message")
    };

    info!("reading connect message");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Connected(_) = msg_recv else {
        panic!("failed to recv connected message")
    };

    info!("reading message 2");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    info!("recvd msg {:?}", msg_recv);
    let LinkSetMsg::Message(Message::Error(msg_recv_txt2), _) = msg_recv else {
        panic!("received incorrect message type")
    };

    info!("reading disconnect message");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Disconnected = msg_recv else {
        panic!("failed to recv connected message")
    };

    assert_eq!(msg_text_1, msg_recv_txt1);
    assert_eq!(msg_text_2, msg_recv_txt2);
}

#[tokio::test]
#[traced_test]
async fn link_set_reconnect() {
    info!("running test");
    let rel1 = SelfRelation::debug_get(0);
    let rel2 = SelfRelation::debug_get(1);

    let pb = PipeLinkBuilder::new()
        .max_size(5)
        .expiration(Some(Duration::from_secs(3)));
    let mut plh = PipeLinkHub::new(pb);

    // Set up first link set
    let mut link_set1 = LinkSet::new(rel1.clone(), rel2.relation.clone());
    link_set1.set_allow_reconnect(Some(30)).await.unwrap();

    // setup listener for link set 1
    info!("Setting up listener");
    let mut listener = plh.listen(rel1.clone(), rel1.relation.sha256());
    let link_set1_sender = link_set1.clone_sender();
    tokio::spawn(async move {
        loop {
            if let Some(link) = listener.recv().await {
                info!("listener got new link");
                link_set1_sender
                    .add_link(link)
                    .await
                    .unwrap();
            } else {
                break;
            }
        }
    });

    // wrap plh in arc mutex
    let plh = Arc::new(Mutex::new(plh));
    // Set up second link set
    let link_set2 = LinkSet::new(rel2, rel1.relation.clone());
    // Enable reconnect
    link_set2.set_reconnect(true).await.unwrap();
    info!("adding link 2");
    link_set2
        .add_connector(move |sr, _r, addr: String| {
            info!("Connector called with addr {}", &addr);
            let plh_inner = plh.clone();
            async move {
                let mut plh = plh_inner.lock().await;
                let x = plh.connect(sr, &addr).await;
                x.ok_or(LinkError::new())
            }
        })
        .await
        .expect("Link should be started and running");
    link_set2.add_addr(rel1.relation.sha256()).await.unwrap();

    // Send message
    info!("sending message 1");
    let msg_text_1 = String::from("Test message #1");
    let msg = Message::Error(msg_text_1.clone());
    let msg_send = msg.clone();
    link_set2.send(msg_send).await.expect("send should succeed");

    // Wait long enough for link to disconnect
    info!("sleeping");
    sleep(Duration::from_secs(31)).await;

    // send another message to reconnect
    info!("sending message 2");
    let msg_text_2 = String::from("Test message #2");
    let msg2 = Message::Error(msg_text_2.clone());
    let msg2_send = msg2.clone();
    link_set2
        .send(msg2_send)
        .await
        .expect("send should succeed");

    // message sequence should be Connected -> Message -> Message
    info!("reading connect message");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Connected(_) = msg_recv else {
        panic!("failed to recv connected message")
    };

    info!("reading message 1");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    info!("recvd msg {:?}", msg_recv);
    let LinkSetMsg::Message(Message::Error(msg_recv_txt1), _) = msg_recv else {
        panic!("received incorrect message type")
    };

    info!("reading message 2");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    info!("recvd msg {:?}", msg_recv);
    let LinkSetMsg::Message(Message::Error(msg_recv_txt2), _) = msg_recv else {
        panic!("received incorrect message type")
    };

    assert_eq!(msg_text_1, msg_recv_txt1);
    assert_eq!(msg_text_2, msg_recv_txt2);
}

#[tokio::test]
#[traced_test]
async fn allow_reconnect_disconnect() {
    info!("running test");
    let rel1 = SelfRelation::debug_get(0);
    let rel2 = SelfRelation::debug_get(1);

    let pb = PipeLinkBuilder::new()
        .max_size(5)
        .expiration(Some(Duration::from_secs(3)));
    let mut plh = PipeLinkHub::new(pb);

    // Set up first link set
    let mut link_set1 = LinkSet::new(rel1.clone(), rel2.relation.clone());
    link_set1.set_allow_reconnect(Some(10)).await.unwrap();

    // setup listener for link set 1
    info!("Setting up listener");
    let mut listener = plh.listen(rel1.clone(), rel1.relation.sha256());
    let link_set1_sender = link_set1.clone_sender();
    tokio::spawn(async move {
        loop {
            if let Some(link) = listener.recv().await {
                info!("listener got new link");
                link_set1_sender
                    .add_link(link)
                    .await
                    .unwrap();
            } else {
                break;
            }
        }
    });

    // wrap plh in arc mutex
    let plh = Arc::new(Mutex::new(plh));
    // Set up second link set
    let link_set2 = LinkSet::new(rel2, rel1.relation.clone());
    info!("adding link 2");
    link_set2
        .add_connector(move |sr, _r, addr: String| {
            info!("Connector called with addr {}", &addr);
            let plh_inner = plh.clone();
            async move {
                let mut plh = plh_inner.lock().await;
                let x = plh.connect(sr, &addr).await;
                x.ok_or(LinkError::new())
            }
        })
        .await
        .expect("Link should be started and running");
    link_set2.add_addr(rel1.relation.sha256()).await.unwrap();

    // Send message
    info!("sending message 1");
    let msg_text_1 = String::from("Test message #1");
    let msg = Message::Error(msg_text_1.clone());
    let msg_send = msg.clone();
    link_set2.send(msg_send).await.expect("send should succeed");

    // message sequence should be Connected -> Message -> Disconnected
    info!("reading connect message");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Connected(_) = msg_recv else {
        panic!("failed to recv connected message")
    };

    info!("reading message 1");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    info!("recvd msg {:?}", msg_recv);
    let LinkSetMsg::Message(Message::Error(msg_recv_txt1), _) = msg_recv else {
        panic!("received incorrect message type")
    };

    info!("reading disconnect message");
    let msg_recv = link_set1.recv().await.expect("should recv message");
    let LinkSetMsg::Disconnected = msg_recv else {
        panic!("failed to recv connected message")
    };

    assert_eq!(msg_text_1, msg_recv_txt1);
}
