use std::{collections::HashSet, sync::Arc, time::Duration};

use spider_link::{
    message::{Message, RouterMessage, UiMessage},
    Link,
};
use tokio::{
    select, spawn,
    sync::{
        mpsc::{channel, Sender},
        watch,
    },
    time::Instant,
};

use crate::processor::{
    message::ProcessorMessage, sender::ProcessorSender, ui::UiProcessorMessage,
};

use super::{RouterProcessorMessage, RouterProcessorState};

// External message processing
impl RouterProcessorState {}

// Internal message processing
impl RouterProcessorState {
    pub(super) async fn new_link_handler(&mut self, link: Link) {
        // check against directory and approval codes for authorization
        let approved = match self.directory.get(link.other_relation()) {
            // if authorized, send authorized link message
            Some(entry) => {
                match entry.get("blocked") {
                    Some(blocked) => {
                        if blocked == "true" {
                            println!("Blocked: entry in directory says block");
                            return; // blocked relations dont pend
                        } else {
                            println!("Approved: entry in directory says no block");
                            true
                        }
                    }
                    None => {
                        println!("Approved: entry in directory exists");
                        true
                    }
                }
            }
            // if not, add settings menu entry for approval
            None => {
                println!("Pending: entry in directory does not exist");
                false
            }
        };
        if approved {
            let msg = RouterProcessorMessage::ApprovedLink(link);
            let msg = ProcessorMessage::RouterMessage(msg);
            self.sender.send(msg).await;
        } else {
            // pending
            let rel = link.other_relation().clone();

            let sender = self.sender.clone();
            let codes = self.approval_codes.clone();
            let codes = codes.keys().cloned().collect();
            let should_approve_ui = self.should_approve_ui.clone();
            // let x = self.should_approve_ui.clone();
            let ctrl = pending_link_processor(sender, link, codes, should_approve_ui);

            self.incoming_links.insert(rel.clone().to_base64(), ctrl);
            // Insert setting
            let sig = rel.id.to_base64();
            let sig: String = sig.chars().skip(sig.len().saturating_sub(15)).collect();
            let title = format!("{:?}: {}", rel.role, sig);
            let msg = UiProcessorMessage::SetSetting {
                header: String::from("Pending Connections"),
                title,
                inputs: vec![
                    ("button".to_string(), "Approve".to_string()),
                    ("button".to_string(), "Deny".to_string()),
                ],
                cb: |idx, _, _, data| {
                    if idx == 0 {
                        // Approve
                        let msg = RouterProcessorMessage::ApproveLink(data.clone());
                        return Some(ProcessorMessage::RouterMessage(msg));
                    }
                    if idx == 1 {
                        // Deny
                        let msg = RouterProcessorMessage::DenyLink(data.clone());
                        return Some(ProcessorMessage::RouterMessage(msg));
                    }
                    None
                },
                data: rel.to_base64(),
            };
            self.sender.send_ui(msg).await;
        }
    }

    pub(super) async fn approve_link_handler(&mut self, relation: String) {
        match self.incoming_links.remove(&relation) {
            Some(ctrl) => {
                ctrl.send(PendingLinkControl::Approve).await;
            }
            None => {}
        }
    }

    pub(super) async fn deny_link_handler(&mut self, relation: String) {
        match self.incoming_links.remove(&relation) {
            Some(ctrl) => {
                ctrl.send(PendingLinkControl::Deny).await;
            }
            None => {}
        }
    }

    pub(super) async fn set_approval_code_handler(&mut self, code: String) {
        // add code to valid set, send code to each pending link processor
        // for approval
        self.approval_codes
            .insert(code.clone(), Instant::now() + Duration::from_secs(300));
        for (_, pending_link) in &self.incoming_links {
            pending_link
                .send(PendingLinkControl::AddCode(code.clone()))
                .await;
        }
    }
}

pub enum PendingLinkControl {
    Approve,
    Deny,
    AddCode(String),
}

fn pending_link_processor(
    sender: ProcessorSender,
    mut link: Link,
    mut codes: HashSet<String>,
    mut should_approve_ui: Arc<watch::Sender<bool>>,
) -> Sender<PendingLinkControl> {
    let (tx, mut rx) = channel(50);
    spawn(async move {
        link.send(Message::Router(RouterMessage::Pending)).await;

        let mut code = Option::<String>::None;
        let mut rx_closed = false;
        let mut code_attempts = 0;
        let mut backlog = Vec::new();
        let mut recvd_ui = false;
        let mut should_approve_ui_recv = should_approve_ui.subscribe();
        loop {
            select! {
                msg = rx.recv(), if !rx_closed => {
                    // Unwrap message
                    let msg = if let Some(msg) = msg{
                        msg
                    } else {
                        rx_closed = true;
                        continue;
                    };

                    // Process message
                    match msg{
                        PendingLinkControl::Approve => {
                            approve_link(sender, link, backlog).await;
                            break;
                        }
                        PendingLinkControl::AddCode(new_code) => {
                            match &code {
                                Some(code) => {
                                    if code == &new_code {
                                        approve_link(sender, link, backlog).await;
                                        break;
                                    } else {
                                        codes.insert(new_code);
                                    }
                                },
                                None => {
                                    codes.insert(new_code);
                                },
                            }
                        }
                        PendingLinkControl::Deny => {
                            // cancel this pending link
                            deny_link(sender, link).await;
                            break;
                        }
                    }
                },
                msg = link.recv() => {
                    match msg {
                        Some(msg) => {

                            // check incoming message for approval code
                            if let Message::Router(RouterMessage::ApprovalCode(new_code)) = &msg {
                                if codes.contains(new_code) {
                                    approve_link(sender, link, backlog).await;
                                    break;
                                } else {
                                    code = Some(new_code.clone());
                                    code_attempts += 1;
                                    if code_attempts > 5 {
                                        // too many incorrect attempts
                                        deny_link(sender, link).await;
                                        break;
                                    }
                                }
                            }

                            // check incoming message is ui subscription
                            if let Message::Ui(UiMessage::Subscribe) = &msg {
                                recvd_ui = true;
                                if *should_approve_ui_recv.borrow() {
                                    // a new connection is added because it is a ui.
                                    // reset the condition to stop further connections being approved
                                    should_approve_ui.send_if_modified(|val|{if !*val {false} else {*val = false; true}});
                                    backlog.push(msg);
                                    approve_link(sender, link, backlog).await;
                                    break;
                                }
                            }

                            // add the message to the backlog
                            backlog.push(msg);
                            if backlog.len() > 100 {
                                // too many messages in backlog
                                deny_link(sender, link).await;
                                break;
                            }
                        },
                        None => {
                            // link is closed, no need to wait for approval
                            deny_link(sender, link).await;
                            break;
                        },
                    }
                },
                _ = should_approve_ui_recv.changed() => {

                    if *should_approve_ui_recv.borrow() && recvd_ui{

                        approve_link(sender, link, backlog).await;
                        break;
                    }
                }
            };
        }
    });
    tx
}

async fn approve_link(mut sender: ProcessorSender, link: Link, backlog: Vec<Message>) {
    // Send approved link to link
    link.send(Message::Router(RouterMessage::Approved)).await;

    // update settings page
    let rel = link.other_relation().clone();
    let sig = rel.id.to_base64();
    let sig: String = sig.chars().skip(sig.len().saturating_sub(15)).collect();
    let title = format!("{:?}: {}", rel.role, sig);
    let msg = UiProcessorMessage::RemoveSetting {
        header: String::from("Pending Connections"),
        title,
    };
    sender.send_ui(msg).await;

    // send approved link message to Router processor
    let msg = RouterProcessorMessage::ApprovedLink(link);
    let msg = ProcessorMessage::RouterMessage(msg);
    sender.send(msg).await;
    // send link backlog
    for msg in backlog {
        sender
            .send(ProcessorMessage::RemoteMessage(rel.clone(), msg))
            .await;
    }
}

async fn deny_link(mut sender: ProcessorSender, link: Link) {
    link.send(Message::Router(RouterMessage::Denied)).await;

    // update settings page
    let rel = link.other_relation().clone();
    let sig = rel.id.to_base64();
    let sig: String = sig.chars().skip(sig.len().saturating_sub(15)).collect();
    let title = format!("{:?}: {}", rel.role, sig);
    let msg = UiProcessorMessage::RemoveSetting {
        header: String::from("Pending Connections"),
        title,
    };
    sender.send_ui(msg).await;
}
