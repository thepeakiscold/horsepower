//! HorsePower power metering service.
//!
//! Samples the hardware energy counters once a second, keeps a local SQLite
//! log, and answers the `org.horsepower.Daemon` D-Bus interface. The GUI is a
//! separate process and cannot tell that this side is no longer Python.

mod cli;
mod config;
mod logging;
mod power;
mod service;
mod store;

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::object_server::SignalEmitter;
use zbus::Connection;

use crate::cli::{Action, BusKind, Config};
use crate::power::Meter;
use crate::service::{Daemon, Sampler, Status};
use crate::store::Store;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let action = match cli::parse(arguments) {
        Ok(action) => action,
        Err(message) => {
            eprintln!("horsepowerd: error: {message}");
            eprint!("{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };

    match action {
        Action::Help => {
            print!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        Action::Version => {
            println!("{}", cli::version_line());
            ExitCode::SUCCESS
        }
        Action::Run(config) => {
            logging::set_verbose(config.verbose);
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    logging::error(&format!("could not start the runtime: {err}"));
                    return ExitCode::FAILURE;
                }
            };
            match runtime.block_on(run(config)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(code) => ExitCode::from(code),
            }
        }
    }
}

async fn run(config: Config) -> Result<(), u8> {
    // Claim the bus name first. A second copy started by hand, or by a race
    // between systemd and D-Bus activation, then exits before touching the
    // database, scanning sysfs or opening the graphics driver.
    let connection = match config.bus {
        BusKind::System => Connection::system().await,
        BusKind::Session => Connection::session().await,
    };
    let connection = match connection {
        Ok(connection) => connection,
        Err(err) => {
            logging::error(&format!("could not connect to the bus: {err}"));
            return Err(1);
        }
    };
    claim_name(&connection).await?;
    logging::info(&format!("Connected to the bus as {}", config::BUS_NAME));

    let store = match Store::open(&config.db_path, config.retention_days) {
        Ok(store) => store,
        Err(err) => {
            logging::error(&format!(
                "cannot open the database {}: {err}",
                config.db_path.display()
            ));
            return Err(1);
        }
    };
    let store = Arc::new(Mutex::new(store));

    let meter = Meter::new(config.sysfs_root.clone(), true);
    logging::info(&format!("Measuring with: {}", meter.source_name()));
    let mut sampler = Sampler::new(meter, Arc::clone(&store), config.interval);
    let status = Arc::new(Mutex::new(sampler.status_now()));

    if let Err(err) = connection
        .object_server()
        .at(
            config::OBJECT_PATH,
            Daemon::new(Arc::clone(&status), Arc::clone(&store)),
        )
        .await
    {
        logging::error(&format!("could not export {}: {err}", config::OBJECT_PATH));
        return Err(1);
    }

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let sampler_connection = connection.clone();
    let sampler_status = Arc::clone(&status);
    let interval = config.interval;
    let worker = tokio::spawn(async move {
        sample_loop(
            &mut sampler,
            sampler_status,
            sampler_connection,
            interval,
            stop_rx,
        )
        .await;
    });

    wait_for_stop().await;
    logging::info("Shutting down");

    // Ask the sampler to flush and exit, then give it a moment to finish.
    let _ = stop_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), worker).await;

    Ok(())
}

/// Take the well-known name, distinguishing "already running" from "not
/// allowed by the D-Bus policy" so the log tells an administrator what to fix.
async fn claim_name(connection: &Connection) -> Result<(), u8> {
    use zbus::fdo::RequestNameFlags;

    let reply = connection
        .request_name_with_flags(config::BUS_NAME, RequestNameFlags::DoNotQueue.into())
        .await;

    match reply {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => Ok(()),
        Ok(_) => {
            logging::error(&format!(
                "another HorsePower service already owns {}",
                config::BUS_NAME
            ));
            // Mirrors the Python service: a duplicate exits quietly, because
            // the running instance is doing the work.
            Err(0)
        }
        Err(zbus::Error::NameTaken) => {
            // With DoNotQueue, "already owned" arrives as an error rather than
            // as `RequestNameReply::Exists`, so it has to be recognised before
            // the generic failure branch or a duplicate would look like a
            // policy problem.
            logging::error(&format!(
                "another HorsePower service already owns {}",
                config::BUS_NAME
            ));
            Err(0)
        }
        Err(err) => {
            logging::error(&format!(
                "not allowed to own {}: {err}. Check \
                 /usr/share/dbus-1/system.d/org.horsepower.Daemon.conf",
                config::BUS_NAME
            ));
            Err(1)
        }
    }
}

async fn sample_loop(
    sampler: &mut Sampler,
    status: Arc<Mutex<Status>>,
    connection: Connection,
    interval: f64,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let emitter = match SignalEmitter::new(&connection, config::OBJECT_PATH) {
        Ok(emitter) => emitter,
        Err(err) => {
            logging::error(&format!("could not create the signal emitter: {err}"));
            return;
        }
    };

    // `interval` panics on a zero duration; the CLI already clamps, this is
    // the second line of defence for direct construction in tests.
    let period = Duration::from_secs_f64(interval.max(0.2));
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick completes immediately, which gives the GUI a reading as
    // soon as it asks rather than one interval later.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = ticker.tick() => {
                let (snapshot, history_changed) = sampler.tick();
                if let Ok(mut shared) = status.lock() {
                    *shared = snapshot.clone();
                }
                if let Err(err) = Daemon::status_changed(&emitter, snapshot.to_dict()).await {
                    logging::debug(&format!("could not emit StatusChanged: {err}"));
                }
                if history_changed {
                    if let Err(err) = Daemon::history_changed(&emitter).await {
                        logging::debug(&format!("could not emit HistoryChanged: {err}"));
                    }
                }
            }
        }
    }

    sampler.shutdown();
}

/// Wait for SIGINT or SIGTERM, whichever comes first.
async fn wait_for_stop() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(err) => {
            logging::error(&format!("could not install SIGTERM handler: {err}"));
            // Fall back to waiting for Ctrl-C only.
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    let mut interrupt = signal(SignalKind::interrupt()).ok();

    tokio::select! {
        _ = terminate.recv() => {}
        _ = async {
            match interrupt.as_mut() {
                Some(stream) => { stream.recv().await; }
                None => std::future::pending::<()>().await,
            }
        } => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
