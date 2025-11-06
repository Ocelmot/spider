
use tracing::info;
use tracing_test::traced_test;

use crate::{error::ProblemWrap, link::{protocol::LinkProtocol, LinkSet, LinkSetMsg, PinnedLink, PipeLink, TCPLink}, message::Message, LinkResult, SelfRelation};

#[tokio::test]
async fn test_link_transmission() {
    let rel1 = SelfRelation::debug_get(0);
    let rel2 = SelfRelation::debug_get(1);
    let (mut link1, mut link2) = PipeLink::create_pair(rel1, rel2);

    // Send message
    let msg = LinkProtocol::Ack { epoch: 8, seq: 25, last_index: 45 };
    let msg_send = msg.clone();
    let msg2 = LinkProtocol::Ack { epoch: 8, seq: 56, last_index: 231 };
    let msg_send2 = msg2.clone();

    link1.send(msg_send).await.expect("send should succeed");
    link2.send(msg_send2).await.expect("send should succeed");

    let msg_recv = link2.recv().await.expect("should recv message");
    let msg_recv2 = link1.recv().await.expect("should recv message");

    assert_eq!(msg, msg_recv);
    assert_eq!(msg2, msg_recv2);
}


#[tokio::test]
#[traced_test]
async fn test_tcp_link_set_round_trip() -> LinkResult {
    // setup relations
    info!("generating keys...");
    let rel1 = SelfRelation::debug_get(0);
    let rel2 = SelfRelation::debug_get(1);
    info!("running test...");

    // set up links
    let mut listener = TCPLink::listen(rel1.clone(), "0.0.0.0:1929");
    info!("listening...");
    let link2 = TCPLink::connect(rel2.clone(), rel1.relation.clone(), "127.0.0.1:1929")
        .await
        .wrap()?;
    info!("connected...");
    let link1 = listener.recv().await.wrap()?;
    info!("got from listener");

    // install into link sets
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
    let LinkSetMsg::Connected(_) = msg_recv else {
        panic!("failed to recv connected message")
    };
    let msg_recv = link_set2.recv().await.expect("should recv message");
    info!("recvd msg {:?}", msg_recv);
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
    Ok(())
}

