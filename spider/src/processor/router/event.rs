use std::collections::HashSet;

use tracing::{info, trace};
use spider_link::{Relation, message::{DatasetData, RouterMessage, Message}};

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
                continue; // this recipient already received message via subscription
            }
            info!("Sending message to external with relation {:?}", external);
            let router_msg = RouterMessage::Event(name.clone(), from.clone(), data.clone());
            let msg = Message::Router(router_msg);
            self.send_msg(external, msg).await;
        }
    }

    pub(crate) async fn handle_event(&mut self, name: String, from: Relation, data: DatasetData){
        trace!("sending event to subscribers");
        // route event to subscribers
        self.event_to_subscribers(&name, &from, &data).await;
    }


}

// Helper functions
impl RouterProcessorState{
    /// Forwards an event (a name and data) from some relation to active links
    /// that have subscribed to events with that name. Skips events from an
    /// external source and external subscriber to avoid routing events that do
    /// not have to do with us. Returns a set of relations to which the event
    /// was sent.
    async fn event_to_subscribers(&mut self, name: &String, from: &Relation, data: &DatasetData) -> HashSet<Relation>{
        let mut recipients = HashSet::new();
        if let Some(subscriber_set) = self.event_subscribers.get(name){
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