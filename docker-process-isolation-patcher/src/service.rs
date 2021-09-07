#[cfg(windows)]

use super::shared::*;
use log::info;

use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher, Result,
};
use std::sync::mpsc;
use std::time::Duration;
use std::ffi::OsString;
use windows_service::service_manager::{ServiceManagerAccess, ServiceManager};
use windows_service::service::{ServiceAccess, ServiceInfo};

use splitty::*;
use std::path::PathBuf;

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

pub fn run() -> Result<()> {
    // Register generated `ffi_service_main` with the system and start the service, blocking
    // this thread until the service is stopped.
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

// Generate the windows service boilerplate.
// The boilerplate contains the low-level service entry function (ffi_service_main) that parses
// incoming service arguments into Vec<OsString> and passes them to user defined service
// entry (my_service_main).
define_windows_service!(ffi_service_main, service_main);

// Service entry function which is called on background thread by the system with service
// parameters. There is no stdout or stderr at this point so make sure to configure the log
// output to file if needed.
pub fn service_main(_arguments: Vec<OsString>) {
    if let Err(_e) = run_service() {
        // Handle the error, by logging or something.
    }
}

pub fn run_service() -> Result<()> {
    // Create a channel to be able to poll a stop event from the service worker loop.
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    // Define system service event handler that will be receiving service events.
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            // Notifies a service to report its current status information to the service
            // control manager. Always return NoError even if not implemented.
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,

            // Handle stop
            ServiceControl::Stop => {
                shutdown_tx.send(()).unwrap();
                ServiceControlHandlerResult::NoError
            }

            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    // Register system service event handler.
    // The returned status handle should be used to report service status changes to the system.
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // Tell the system that service is running
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
    let service_access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START | ServiceAccess::QUERY_CONFIG | ServiceAccess::CHANGE_CONFIG;

    let mut modified_docker = false;
    loop {
        // Poll shutdown event.
        match shutdown_rx.recv_timeout(Duration::from_secs(1)) {
            // Break the loop either upon stop or channel disconnect
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("service::run_service: stopping service");
                break
            },

            // Continue work if no events were received within the timeout
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        };

        let res = service_manager.open_service(DOCKER_SERVICE_NAME, service_access);
        if let Ok(service) = res {
            let mut service_state = service.query_status()?;

            if !modified_docker {
                if service_state.current_state == ServiceState::Running {
                    // don't change service immediately once detected, otherwise it'll fail
                    std::thread::sleep(Duration::from_secs(2));
                    // requery again to make sure it's still up
                    service_state = service.query_status()?;
                    if service_state.current_state != ServiceState::Running {
                        info!("service::run_service: docker state changed during 2 second wait: {:?}", service_state.current_state);
                        continue;
                    }

                    let config = service.query_config()?;

                    // this service was already patched - update and exit
                    if config.display_name == "Docker Engine - Patched Process Isolation" {
                        info!("service::run_service: docker service already patched - updating status to modified");
                        modified_docker = true;
                        continue;
                    }

                    info!("service::run_service: detected unmodified docker service");
                    info!("service::run_service: stopping docker service");

                    // stop service
                    service.stop().ok();

                    // wait for service to stop
                    loop {
                        service_state = service.query_status()?;

                        if service_state.current_state == ServiceState::Stopped {
                            info!("service::run_service: docker service stopped");
                            break;
                        }

                        std::thread::sleep(Duration::from_millis(250));
                    }

                    info!("service::run_service: patching docker service");

                    let path = config.executable_path.to_str().unwrap();

                    let mut split_path: Vec<&str> = split_unquoted_whitespace(path).unwrap_quotes(true).collect();
                    split_path.insert(1, "--exec-opt");
                    split_path.insert(2, "isolation=process");
                    let buffer: Vec<OsString> = split_path.iter().map(|x| OsString::from(x)).collect();

                    let new_config = ServiceInfo {
                        name: OsString::from(DOCKER_SERVICE_NAME),
                        display_name: OsString::from("Docker Engine - Patched Process Isolation"),
                        service_type: config.service_type,
                        start_type: config.start_type,
                        error_control: config.error_control,
                        executable_path: PathBuf::from(&buffer[0]),
                        launch_arguments: buffer[1..].to_vec(),
                        dependencies: config.dependencies,
                        account_name: None,
                        account_password: None
                    };

                    service.change_config(&new_config).unwrap();
                    service.set_description("Patched docker process isolated service").unwrap();

                    info!("service::run_service: successfully patched docker service");

                    info!("service::run_service: starting docker service");
                    service.start(&[] as &[&str])?;
                    info!("service::run_service: started docker service");

                    modified_docker = true;
                }
            } else {
                // docker stopped the process
                if service_state.current_state == ServiceState::Stopped {
                    info!("service::run_service: docker just stopped the service");
                    modified_docker = false;
                }
            }
        } else {
            // docker deleted the service
            if modified_docker {
                info!("service::run_service: docker deleted the service");
                modified_docker = false;
            }
        }
    }

    // Tell the system that service has stopped.
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}
