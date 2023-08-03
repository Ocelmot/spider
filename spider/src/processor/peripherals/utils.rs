use std::{path::{PathBuf, Path}, str::FromStr, process::Stdio, io::SeekFrom, env};

use regex::Regex;
use spider_link::message::UiInput;
use tokio::{process::{Command, Child}, fs::{File, self, OpenOptions}, io::{AsyncReadExt, AsyncWriteExt, AsyncSeekExt}};

use crate::processor::{message::ProcessorMessage, ui::UiProcessorMessage};

use super::{PeripheralProcessorState, PeripheralProcessorMessage, manifest::PeripheralManifest};




impl PeripheralProcessorState{

    pub(crate) fn get_service_directory(&self, name: &str)->PathBuf{
        let base = &self.config.peripheral_path();
        let path = base.join(PathBuf::from_str(name).expect("paths should be joinable"));
        path
    }

    pub(crate) async fn download_with_git(&self, path: &PathBuf, addr: &String){
        let x = Command::new("git")
            .current_dir(path.clone())
            .arg("clone")
            .arg(addr)
            .arg(".")
            .output().await;
        println!("{:?}", x);
        
        // Fix for nexted crates while developing
        // if let Ok(val) = env::var("CARGO"){
            let path = path.join("Cargo.toml");
            if let Ok(s) = fs::read_to_string(path.clone()).await {
                let re = Regex::new(r"^[workspace]$").unwrap();
                if !re.is_match(&s){
                    // if there is no [workspace] in the cargo manifest,
                    // tag one at the bottom
                    let mut file = OpenOptions::new();
                    file.write(true);
                    file.append(true);
                    let mut file = file.open(path).await.unwrap();
                    let buf = String::from("\n[workspace]\n");
                    file.write_all_buf(&mut buf.as_bytes()).await.unwrap();
                }
            }
        // }
    }

    pub(crate) async fn write_keyfile(&self, mut path: PathBuf){
        path.push("spider_keyfile.json");
        let data = serde_json::to_string(&self.state.self_id().await).unwrap();
        tokio::fs::write(&*path, data).await;
    }

    pub(crate) async fn launch_peripheral_service(&mut self, name: String) -> Option<Child>{
        let path = self.get_service_directory(&name);
        // let path = path.canonicalize().unwrap();
        println!("launching peripheral: {}", path.display());

        let manifest = PeripheralManifest::read(&path).await?;

        let mut command = match manifest.launch(){
            crate::processor::peripherals::manifest::LaunchConfig::Exe(exe_name) => {
                let mut exe_path = path.clone();
                exe_path.push(exe_name);
        
                Command::new(exe_path)
            },
            crate::processor::peripherals::manifest::LaunchConfig::Python(python_path) => {
                let mut cmd = Command::new("python");
                cmd.arg(python_path);
                cmd
            },
            crate::processor::peripherals::manifest::LaunchConfig::Cargo => {
                let mut cmd = Command::new("cargo");
                cmd.arg("run");
                cmd
            },
        };
        command.current_dir(path.clone());

        println!("launching child: {}", path.display());
        launch_child(command, Some(&path)).await
    }

    pub(crate) async fn make_setting_entry(&mut self, name: String, running: bool){
        let (start_stop, cb) = match running {
            true => ("Stop".to_string(), cb_with_stop as fn(u32, &String, UiInput, &mut String) -> Option<ProcessorMessage>),
            false => ("Start".to_string(), cb_with_start as fn(u32, &String, UiInput, &mut String) -> Option<ProcessorMessage>),
        };
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Peripheral Services"),
            title: name,
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

// ===== Settings Functions ===== (Remove when updgrade settings callback handling)

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

// ===== Child Functions =====

pub(crate) async fn launch_child(mut command: Command, stdio_dir: Option<&Path> ) -> Option<Child>{
    command.stdin(Stdio::null()); // dont pass inputs to child
    match stdio_dir{
        Some(stdio_dir) => {
            command.stdout(Stdio::piped()); //create pipes for stdio
            command.stderr(Stdio::piped());
            match command.spawn(){
                Ok(mut child) => {

                    wrap_child_stdio(&mut child, stdio_dir).await;
                    Some(child)
                },
                Err(e) => {
                    println!("Error launching peripheral service: {}", e);
                    None
                }, // ignore error for now
            }
        },
        None => {
            command.stdout(Stdio::null()); // dont pass outputs to parent
            command.stderr(Stdio::null());
            match command.spawn(){
                Ok(child) => Some(child),
                Err(e) => {
                    println!("Error launching peripheral service: {}", e);
                    None
                },
            }
        },
    }
}

async fn wrap_child_stdio(child: &mut Child, stdio_dir: &Path) {
    let mut stdout = child.stdout.take().expect("should have handle");
    let mut stdout_file = File::create(stdio_dir.join("stdout")).await.expect("file can be created");
    let mut stderr = child.stderr.take().expect("should have handle");
    let mut stderr_file = File::create(stdio_dir.join("stderr")).await.expect("file can be created");
    let file_max_len = 16000000;
    let child_handle = tokio::spawn(async move {
        loop{
            let mut outbuf = Vec::new();
            let mut errbuf = Vec::new();
            tokio::select! {
                out = stdout.read_buf(&mut outbuf) =>{
                    match out{
                        Ok(count) => {
                            if count == 0{
                                break;
                            }
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
                            if count == 0{
                                break;
                            }
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
}
