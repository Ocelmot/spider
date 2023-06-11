use std::path::Path;

use serde::Deserialize;
use tokio::fs;
use toml::Value;





#[derive(Debug, Deserialize)]
pub enum BuildConfig{
    None,
    Cargo,
}

#[derive(Debug, Deserialize)]
pub enum LaunchConfig{
    Exe(String),
    Python(String),
    Cargo,
}

#[derive(Debug, Deserialize)]
pub struct PeripheralManifest{
    #[serde(default)]
    build: Option<BuildConfig>,
    launch: LaunchConfig,
}


impl PeripheralManifest{
    pub async fn read(path: &Path) -> Option<Self>{
        let data = if path.is_dir(){
            fs::read_to_string(path.join("Manifest.toml")).await
        }else{
            fs::read_to_string(path).await
        };

        

        match data{
            Ok(data) => {
                match toml::from_str(&data){
                    Ok(val) => {
                        let val: Value = val;


                        let build = match val.get("build"){
                            Some(build_val) => {
                                match build_val{
                                    Value::String(s) if s == "cargo" => {
                                        Some(BuildConfig::Cargo)
                                    },
                                    _ => None
                                }
                            },
                            None => None,
                        };
                        let launch = match val.get("launch")?{
                            Value::Table(launch_table) => {
                                match launch_table.get("method")? {
                                    Value::String(method) if method == "exe" => {
                                        if let Value::String(path) = launch_table.get("path")?{
                                            LaunchConfig::Exe(path.to_string())
                                        }else{
                                            return None;
                                        }
                                    },
                                    Value::String(method) if method == "python" => {
                                        if let Value::String(path) = launch_table.get("path")?{
                                            LaunchConfig::Python(path.to_string())
                                        }else{
                                            return None;
                                        }
                                    },
                                    Value::String(method) if method == "cargo" => {
                                        LaunchConfig::Cargo
                                    },
                                    _ => return None,
                                }
                            },
                            _ => return None,
                        };
                        Some(PeripheralManifest{
                            build,
                            launch,
                        })
                    },
                    Err(_) => None,
                }
            },
            Err(_) => None,
        }
    }


    pub fn build(&self) -> &Option<BuildConfig>{
        &self.build
    }

    pub fn launch(&self) -> &LaunchConfig{
        &self.launch
    }
}
