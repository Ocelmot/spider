
use std::collections::HashMap;

use crate::{config::SpiderConfig, state_data::StateData};

use super::{sender::ProcessorSender, ui::UiProcessorMessage, message::ProcessorMessage};

mod message;
pub use message::PeripheralProcessorMessage;

mod manifest;

mod utils;

use regex::Regex;

use tokio::{
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle}, fs::{create_dir_all, remove_dir_all},
    process::Child,
};



pub(crate) struct PeripheralsProcessor{
    sender: Sender<PeripheralProcessorMessage>,
    handle: JoinHandle<()>,
}

impl PeripheralsProcessor {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender)-> Self{
        let (peripheral_sender, peripheral_receiver) = channel(50);
        let processor = PeripheralProcessorState::new(config, state, sender, peripheral_receiver);
        let handle = processor.start();


        Self{
            sender: peripheral_sender,
            handle
        }
    }

    pub async fn send(
        &mut self,
        message: PeripheralProcessorMessage,
    ) -> Result<(), SendError<PeripheralProcessorMessage>> {
        self.sender.send(message).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}



struct PeripheralProcessorState{
    config: SpiderConfig,
    state: StateData,
    sender: ProcessorSender,
    receiver: Receiver<PeripheralProcessorMessage>,

    children: HashMap<String, Child>
}

impl PeripheralProcessorState{
    fn new(
        config: SpiderConfig,
        state: StateData,
        sender: ProcessorSender,
        receiver: Receiver<PeripheralProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
            receiver,

            children: HashMap::new(),
        }
    }



    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            self.init().await;
            loop {
                let msg = match self.receiver.recv().await {
                    Some(msg) => msg,
                    None => break,
                };

                match msg {                    
                    PeripheralProcessorMessage::Install(addr) => self.install_service(addr).await,
                    PeripheralProcessorMessage::Start(name) => self.start_service(name).await,
                    PeripheralProcessorMessage::Stop(name) => self.stop_service(name).await,
                    PeripheralProcessorMessage::Remove(name) => self.uninstall_service(name).await,

                    PeripheralProcessorMessage::Upkeep => {}
                }
            }
        });
        handle
    }

    async fn init(&mut self){
        // register settings section
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Peripheral Services"),
            title: String::from("Install:"),
            inputs: vec![("textentry".to_string(), "Git Path".to_string())],
            cb: |idx, name, input, _|{
                match input{
                    spider_link::message::UiInput::Click => None,
                    spider_link::message::UiInput::Text(addr) => {
                        let peripheral_msg = PeripheralProcessorMessage::Install(addr);
                        let msg = ProcessorMessage::PeripheralMessage(peripheral_msg);
                        Some(msg)
                    },
                }
            },
            data: String::new(),
        };
        self.sender.send_ui(msg).await;

        // iterate and launch
        let ps = self.state.peripheral_services().await;
        let mut x = Vec::new();
        for (name, status) in ps.iter(){
            x.push((name.clone(), *status));
        }
        drop(ps);
        for (name, status) in x{
            if status {
                let child = self.launch_peripheral_service(name.clone()).await;
                if let Some(child) = child {
                    self.children.insert(name.clone(), child);
                    // insert into settings as well
                    self.make_setting_entry(name.clone(), true).await;
                }else{
                    self.make_setting_entry(name, false).await;    
                }
            } else {
                self.make_setting_entry(name, false).await;
            }
        }
        
    }

    async fn install_service(&mut self, addr: String){
        println!("========== Installing! ============\n{}", addr);
        // parse addr
        let re = Regex::new(r"/([^/]*?)(\.git)?$").unwrap();
        let name = match re.captures(&addr){
            Some(captures) => {
                match captures.get(1){
                    Some(g1) => {
                        g1.as_str().to_string()
                    },
                    None => return,
                }
            },
            None => return ,
        };
        println!("package name: {}", name);

        let path = self.get_service_directory(&name);
        println!("Produced path: {}", path.display());
        // create directory
        create_dir_all(path.clone()).await.unwrap();

        // launch git in directory
        println!("launching git...");
        self.download_with_git(&path, &addr).await;

        // copy keyfile into directory
        println!("writing keyfile...");
        self.write_keyfile(path.clone()).await;

        // list process in state file
        let mut ps = self.state.peripheral_services().await;
        ps.insert(name.clone(), true);
        drop(ps);

        // launch peripheral as sub-process
        println!("launching subprocess...");
        let child = self.launch_peripheral_service(name.clone()).await;
        if let Some(child) = child {
            self.children.insert(name.clone(), child);
            // insert into settings as well
            self.make_setting_entry(name.clone(), true).await;
        }
    }


    

    async fn start_service(&mut self, name: String){
        match self.state.peripheral_services().await.get_mut(&name){
            Some(running) if *running == false => {
                // set to running
                *running = true;
            },
            _ => {
                return; // not installed, or already running
            }, 
        }

        // start child
        let child = self.launch_peripheral_service(name.clone()).await;
        if let Some(child) = child {
            self.children.insert(name.clone(), child);
            // insert into settings as well
            self.make_setting_entry(name.clone(), true).await;
        }
    }

    async fn stop_service(&mut self, name: String){
        match self.state.peripheral_services().await.get_mut(&name){
            Some(running) if *running == true => {
                // set to stopped
                *running = false;
            },
            _ => {
                return; // not installed, or already stopped
            }, 
        }

        if let Some(mut child) = self.children.remove(&name){
            child.kill().await;
        }

        self.make_setting_entry(name, false).await;
    }

    async fn uninstall_service(&mut self, name: String){
        println!("========== Uninstalling! ============");
        println!("package name: {}", name);

        let path = self.get_service_directory(&name);
        println!("Produced path: {}", path.display());

        // remove from state
        let mut ps = self.state.peripheral_services().await;
        ps.remove(&name);
        drop(ps);
        self.state.save_file().await;

        // stop child if started
        if let Some(mut child) = self.children.remove(&name) {
            child.kill().await;
        }

        // remove folder
        remove_dir_all(path).await;

        // remove setting entry
        let msg = UiProcessorMessage::RemoveSetting {
            header: String::from("Peripheral Services"),
            title: name.clone()
        };
        self.sender.send_ui(msg).await;
    }

}
