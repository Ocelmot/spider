#![deny(missing_docs)]

//! The Spider application serves as the base for the Spider network.
//!
//! It coordinates communication between the members of the network that
//! connect to it, in order to provide various digital services. It is able to
//! host these services without the need for a centralized third party host.
//! Services that are not shared between users are provided by the base, while
//! services that require communication between many users are provided by
//! each involved user's base. This should provide sufficient computational
//! power for most services we expect from the internet today, with few
//! exceptions.
//!
//! These connections fall into two broad categories: Peers
//! and Peripherals. Peers represent connections to other bases used by other
//! users.
//! Peripherals are connections to devices or services under the same ownership
//! as the base. Peripherals can also be further categorized into the following
//! categories.
//! - A Service is a peripheral that executes as a subprocess of the base, and
//! therefore on the same hardware. This can help minimize setup and
//! configuration for these kinds of peripherals. It is the only type of
//! peripheral that is automatically paired to the base.
//! - An Application peripheral executes on a device that also executes other
//! user software. E.g. personal computers or mobile devices. These peripherals
//! may use the base's UI or provide their own.
//! - A UI Peripheral is an application peripheral that specializes in
//! displaying UI pages on behalf of the base. This could be either on a
//! personal computer or a mobile device.
//! - A Standalone peripheral executes on its own device. E.g IOT type devices.
//! As these devices are typical headless or have limited UI capabilities,
//! they can register a UI page with the base to be displayed through the
//! base's interface.
//! - A Satellite peripheral is a standalone peripheral that does not register
//! a UI page or receive messages. It only sends messages to the base. E.g.
//! some type of low power probe or sensor.

/// included libraries
use std::{
    env,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    time::Duration,
};

use simple_logger::SimpleLogger;
use tracing::{debug, error, info, level_filters::LevelFilter, trace, Level};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    filter::filter_fn, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

/// Module structure
mod config;
use config::SpiderConfig;

mod state_data;
use state_data::StateData;

mod processor;
use crate::processor::ProcessorBuilder;

mod error;

/// Command line arguments: <filename>
/// filename is name of config file, defaults to config.json
#[tokio::main]
async fn main() -> Result<(), io::Error> {
    // setup tokio debugger
    // console_subscriber::ConsoleLayer::builder()
    //     .retention(Duration::from_secs(600))
    //     .server_addr(([127, 0, 0, 1], 6669))
    //     .init();

    // load config file
    let config = load_config();

    // Setup tracing
    let filter = filter_fn(|metadata| {
        if metadata.target().contains("spider") {
            return true;
        }
        // if metadata.target().contains("veilid") && metadata.level() <= &Level::INFO {
        //     return true
        // }
        false
    });

    // let log_path = config.log_path.clone();
    // let file_appender = RollingFileAppender::new(Rotation::NEVER, "", log_path);
    // let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let (non_blocking, _guard) = tracing_appender::non_blocking(std::io::stdout());
    let subscriber = tracing_subscriber::fmt()
        .compact()
        // .with_ansi(false)
        .with_writer(non_blocking)
        .with_max_level(LevelFilter::TRACE)
        .finish();
    subscriber.with(filter).init();

    trace!("trace");
    debug!("debug");

    info!("Starting!");
    info!("Loaded config: {:?}", config);

    let mut pb = ProcessorBuilder::new();
    pb.config(config.clone());
    pb.state_file(Path::new(&config.state_data_path));

    // if state is empty, enter new mode to create an id and establish first UI connection/owner
    // (could also migrate id from other spider)
    // else, use loaded state with known id
    if pb.is_new() {
        let state = StateData::with_generated_key(Path::new(&config.state_data_path));
        pb.state(state);
    }

    // start processor
    let processor_handle = pb
        .start_processor()
        .await
        .expect("processor was able to start");

    processor_handle.join().await;
    Ok(())
}

fn load_config() -> SpiderConfig {
    let mut args = env::args().skip(1);
    let path_str = args.next().unwrap_or("spider_config.json".to_string());
    let config_path = Path::new(&path_str);
    println!("Current working directory: {:?}", std::env::current_dir());
    println!("Loading config file from path: {:?}", config_path);
    let config = SpiderConfig::from_file(&config_path);
    config
}
