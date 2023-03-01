/// included libraries
use std::{env, io::{self, ErrorKind}, path::{Path, PathBuf}};

use tracing::{info, debug, error};
use tracing_appender::rolling::{Rotation, RollingFileAppender};
use tracing_subscriber::{filter::filter_fn, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};

/// Module structure
mod config;
use config::SpiderConfig;

mod state_data;
use state_data::StateData;

mod processor;
use crate::processor::{ProcessorBuilder};


#[tokio::main]
async fn main() -> Result<(), io::Error> {
	// command line arguments: <filename>
	// filename is name of config file, defaults to config.json

	// load config file
	let config = load_config();

	

	// Setup tracing
	let filter = filter_fn(|metadata|{
		metadata.target() == "spider"
	});
	
	let log_path = config.log_path.clone();
	let file_appender = RollingFileAppender::new(Rotation::NEVER, "", log_path);
	let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
	let subscriber = tracing_subscriber::fmt()
		.pretty().with_ansi(false)
		.with_writer(non_blocking)
		.finish();
	subscriber.with(filter).init();

	info!("Starting!");
	info!("Loaded config: {:?}", config);


	let mut pb = ProcessorBuilder::new();
	pb.config(config.clone());
	pb.state_file(Path::new(&config.state_data_path));


	// if state is empty, enter new mode to create an id and establish first UI connection/owner
	// (could also migrate id from other spider)
	// else, use loaded state with known id
	if pb.is_new(){
		let state = StateData::with_generated_key(Path::new(&config.state_data_path));
		pb.state(state);
	}

	// start processor 
	let processor_handle = pb.start_processor().expect("processor was able to start");

	processor_handle.join().await;
	Ok(())
}

fn load_config() -> SpiderConfig {
	let mut args = env::args().skip(1);
	let path_str = args.next().unwrap_or("spider_config.json".to_string());
	let config_path = Path::new(&path_str);
	let config = SpiderConfig::from_file(&config_path);
	config
}
