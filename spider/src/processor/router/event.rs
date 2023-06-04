use std::{time::Duration, collections::HashSet};

use spider_link::{Relation, message::{DatasetData, RouterMessage, Message}};
use tokio::time::Instant;

use super::RouterProcessorState;










// event handling functions
impl RouterProcessorState{
    pub(crate) async fn handle_send_event(&mut self, from: Relation, name: String, externals: Vec<Relation>, data: DatasetData){
        // route event to peripherals, and relevant peers
        // Send to subscribers
        let recipients = self.event_to_subscribers(&name, &from, &data).await;
        // send to externals
        for external in externals{
            if recipients.contains(&external){
                continue; // this recipient already recieved message via subscription
            }
            println!("Sending message to external...");
            match self.links.get_mut(&external){
                Some(link) => {
                    // send to already-connected link
                    println!("Link is connected");
                    let router_msg = RouterMessage::Event(name.clone(), from.clone(), data.clone());
                    let msg = Message::Router(router_msg);
                    link.send(msg).await;
                    println!("Sent");
                },
                None => {
                    // insert into pending links
                    println!("Link is pending");
                    match self.pending_links.get_mut(&external) {
                        Some((_, tries, pending_msgs)) => {
                            println!("adding message to entry");
                            let router_msg = RouterMessage::Event(name.clone(), from.clone(), data.clone());
                            let msg = Message::Router(router_msg);
                            pending_msgs.push(msg);
                            *tries = 0;
                        },
                        None => {
                            // not already in, need to init connection requests
                            println!("new pending entry");
                            let router_msg = RouterMessage::Event(name.clone(), from.clone(), data.clone());
                            let msg = Message::Router(router_msg);
                            let pending_msgs = vec![msg];
                            let mut t = Instant::now();
                            t = t - Duration::from_secs(600);
                            self.pending_links.insert(external.clone(), (t, 0u8, pending_msgs));
                            // start connection process
                            self.process_pending_link(external).await;
                        },
                    }
                },
            }
        }
    }

    pub(crate) async fn handle_event(&mut self, name: String, from: Relation, data: DatasetData){
        // route event to subscribers
        self.event_to_subscribers(&name, &from, &data).await;
    }


}

// Helper functions
impl RouterProcessorState{
    async fn event_to_subscribers(&mut self, name: &String, from: &Relation, data: &DatasetData) -> HashSet<Relation>{
        let mut recipients = HashSet::new();
        if let Some(subscriber_set) = self.subscribers.get(name){
            for subscriber in subscriber_set{
                // Check if source is external and dest is external, skip
                if from.is_peer() && subscriber.is_peer(){
                    continue;
                }
                if let Some(link) = self.links.get_mut(subscriber){
                    recipients.insert(subscriber.clone());
                    let router_msg = RouterMessage::Event(name.clone(), from.clone(), data.clone());
                    let msg = Message::Router(router_msg);
                    link.send(msg).await;
                }
            }
        }
        recipients
    }
}