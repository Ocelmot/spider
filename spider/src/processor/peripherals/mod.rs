
use std::{path::PathBuf, str::FromStr, collections::HashMap, process::Stdio, io::SeekFrom};

use crate::{config::SpiderConfig, state_data::StateData};

use super::{sender::ProcessorSender, ui::UiProcessorMessage, message::ProcessorMessage};

mod message;
pub use message::PeripheralProcessorMessage;

use regex::Regex;
use spider_link::message::UiInput;
use tokio::{
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle}, fs::{create_dir_all, File, remove_dir_all},
    process::{Command, Child}, io::{AsyncReadExt, AsyncWriteExt, AsyncSeekExt},
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
                    PeripheralProcessorMessage::Install(addr) => self.install(addr).await,
                    PeripheralProcessorMessage::Start(name) => self.start_service(name).await,
                    PeripheralProcessorMessage::Stop(name) => self.stop_service(name).await,
                    PeripheralProcessorMessage::Remove(name) => self.uninstall(name).await,

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
                self.launch_peripheral_service(name).await;
            } else {
                self.make_setting_entry(name, false).await;
            }
        }
        
    }

    async fn install(&mut self, addr: String){
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

        let path = self.get_peripheral_directory(name.clone());
        println!("Produced path: {}", path.display());
        // create directory
        create_dir_all(path.clone()).await.unwrap();

        // launch git in directory
        println!("launching git...");
        self.launch_git(&path, addr).await;

        // copy keyfile into directory
        println!("writing keyfile...");
        self.write_keyfile(path.clone()).await;

        // list process in state file
        let mut ps = self.state.peripheral_services().await;
        ps.insert(name.clone(), true);
        drop(ps);

        // launch peripheral as sub-process
        println!("launching subprocess...");
        self.launch_peripheral_service(name).await
    }


    async fn uninstall(&mut self, name: String){
        println!("========== Uninstalling! ============");
        println!("package name: {}", name);

        let path = self.get_peripheral_directory(name.clone());
        println!("Produced path: {}", path.display());

        match self.children.remove(&name) {
            Some(mut child) => {
                // remove from state
                let mut ps = self.state.peripheral_services().await;
                ps.remove(&name);
                drop(ps);
                self.state.save_file().await;
                // remove setting entry
                let msg = UiProcessorMessage::RemoveSetting {
                    header: String::from("Peripheral Services"),
                    title: name.clone()
                };
                self.sender.send_ui(msg).await;
                // stop child if started
                child.kill().await;
                // remove folder
                remove_dir_all(path).await;
            },
            None => {}, // Nothing to remove
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
        self.launch_peripheral_service(name).await;
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

    fn get_peripheral_directory(&self, name: String)->PathBuf{
        let base = &self.config.peripheral_path();
        let mut path = base.join(PathBuf::from_str(&name).expect("paths should be joinable"));
        // path = path.canonicalize().expect("canonicalizable");
        path
    }

    async fn launch_git(&self, path: &PathBuf, addr: String){
        let x = Command::new("git")
            .current_dir(path)
            .arg("clone")
            .arg(addr)
            .arg(".")
            .output().await;
        println!("{:?}", x);
    }

    async fn write_keyfile(&self, mut path: PathBuf){
        path.push("spider_keyfile.json");
        let data = serde_json::to_string(&self.state.self_id().await).unwrap();
        tokio::fs::write(&*path, data).await;
    }

    async fn launch_peripheral_service(&mut self, name: String){
        let path = self.get_peripheral_directory(name.clone());
        let full_path = path.canonicalize().unwrap();
        let mut exe_path = full_path.clone();
        exe_path.push(format!("{}.exe", name));

        println!("launching peripheral: {}", exe_path.display());
        let x = Command::new(exe_path)
            .current_dir(full_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match x{
            Ok(mut child) => {
                // map subprocess stdio to files
                let mut stdout = child.stdout.take().expect("should have handle");
                let mut stdout_file = File::create(path.join("stdout")).await.expect("file can be created");
                let mut stderr = child.stderr.take().expect("should have handle");
                let mut stderr_file = File::create(path.join("stderr")).await.expect("file can be created");
                let file_max_len = 16000;
                let child_handle = tokio::spawn(async move {
                    loop{
                        let mut outbuf = Vec::new();
                        let mut errbuf = Vec::new();
                        tokio::select! {
                            out = stdout.read_buf(&mut outbuf) =>{
                                match out{
                                    Ok(count) => {
                                        // write to file, if file would be too long rotate it
                                        stdout_file.write(&outbuf[..count]).await;
                                        let file_len = stdout_file.metadata().await.unwrap().len();
                                        
                                        if file_len > file_max_len {
                                            let mut rotate_buf = String::new();
                                            stdout_file.read_to_string(&mut rotate_buf).await;
                                            let new_content: String = rotate_buf.chars()
                                                .skip(rotate_buf.len() / 2)
                                                .skip_while(|x| *x != '\n')
                                                .skip_while(|x| *x == '\n')
                                                .collect();
                                            stdout_file.seek(SeekFrom::Start(0)).await;
                                            let bytes = new_content.as_bytes();
                                            stdout_file.write_all(bytes).await;
                                            stdout_file.set_len(bytes.len().try_into().unwrap()).await;
                                            stdout_file.flush().await;
                                        }
                                    },
                                    Err(_) => {break;},
                                }
                            },
                            err = stderr.read_buf(&mut errbuf) =>{
                                match err{
                                    Ok(count) => {
                                        // write to file, if file would be too long rotate it
                                        stderr_file.write(&errbuf[..count]).await;
                                        let file_len = stderr_file.metadata().await.unwrap().len();
                                        
                                        if file_len > file_max_len {
                                            let mut rotate_buf = String::new();
                                            stderr_file.read_to_string(&mut rotate_buf).await;
                                            let new_content: String = rotate_buf.chars()
                                                .skip(rotate_buf.len() / 2)
                                                .skip_while(|x| *x != '\n')
                                                .skip_while(|x| *x == '\n')
                                                .collect();
                                            stderr_file.seek(SeekFrom::Start(0)).await;
                                            let bytes = new_content.as_bytes();
                                            stderr_file.write_all(bytes).await;
                                            stderr_file.set_len(bytes.len().try_into().unwrap()).await;
                                            stderr_file.flush().await;
                                        }
                                    },
                                    Err(_) => {break;},
                                }
                            }
                        }        
                    }
                });

                self.children.insert(name.clone(), child);
                // insert into settings as well
                self.make_setting_entry(name.clone(), true).await;
                

            },
            Err(e) => {
                println!("Error launching peripheral service: {}", e);
            }, // ignore error for now
        }
    }

    async fn make_setting_entry(&mut self, name: String, running: bool){
        let (start_stop, cb) = match running {
            true => ("Stop".to_string(), cb_with_stop as fn(u32, &String, UiInput, &mut String) -> Option<ProcessorMessage>),
            false => ("Start".to_string(), cb_with_start as fn(u32, &String, UiInput, &mut String) -> Option<ProcessorMessage>),
        };
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Peripheral Services"),
            title: String::from(name),
            inputs: vec![
                ("button".to_string(), start_stop),
                ("button".to_string(), "Remove".to_string())
            ],
            cb,
            data: String::new(),
        };
        self.sender.send_ui(msg).await;
    }
}



fn cb_with_stop(idx: u32, name: &String, input: UiInput, data: &mut String) -> Option<ProcessorMessage>{
    match idx{
        0 => {
            match input{
                UiInput::Click => {
                    let peripheral_msg = PeripheralProcessorMessage::Stop(name.clone());
                    let msg = ProcessorMessage::PeripheralMessage(peripheral_msg);
                    Some(msg)
                },
                UiInput::Text(_) => None,
            }
        }
        1 => {
            match input{
                UiInput::Click => {
                    let peripheral_msg = PeripheralProcessorMessage::Remove(name.clone());
                    let msg = ProcessorMessage::PeripheralMessage(peripheral_msg);
                    Some(msg)
                },
                UiInput::Text(_) => None,
            }
        }
        _ => None
    }
}

fn cb_with_start(idx: u32, name: &String, input: UiInput, data: &mut String) -> Option<ProcessorMessage>{
    match idx{
        0 => {
            match input{
                UiInput::Click => {
                    let peripheral_msg = PeripheralProcessorMessage::Start(name.clone());
                    let msg = ProcessorMessage::PeripheralMessage(peripheral_msg);
                    Some(msg)
                },
                UiInput::Text(_) => None,
            }
        }
        1 => {
            match input{
                UiInput::Click => {
                    let peripheral_msg = PeripheralProcessorMessage::Remove(name.clone());
                    let msg = ProcessorMessage::PeripheralMessage(peripheral_msg);
                    Some(msg)
                },
                UiInput::Text(_) => None,
            }
        }
        _ => None
    }
}
