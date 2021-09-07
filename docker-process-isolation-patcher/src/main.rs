#[cfg(windows)]

use std::error::Error;
use std::ffi::OsString;
use std::io::Write;
use std::time::Duration;

use clap::{App, Arg};
use is_elevated::is_elevated;
use log::{error, info};
use windows_service::{
    service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType},
    service_manager::{ServiceManager, ServiceManagerAccess}
};
use windows_service::service::Service;

use human_panic_logger::setup_panic_logger;
use shared::*;

mod service;
mod shared;

macro_rules! print_flush {
    ( $($t:tt)* ) => {
        {
            let mut h = std::io::stdout();
            write!(h, $($t)* ).unwrap();
            h.flush().unwrap();
        }
    }
}

fn main() {
    let log_path = std::env::current_exe().unwrap().with_file_name("app.log");
    setup_panic_logger!(log_path);

    if let Err(e) = run() {
        error!("Caught error: {:?}", e);
        std::process::exit(1);
    }
}

fn run() -> windows_service::Result<()> {
    let matches = App::new("Docker Process Isolation Service")
        .version("1.0")
        .author("Cherryleafroad")
        .about("Makes docker Windows service always run in process isolation mode (run with admin privileges)")
        .arg(Arg::new("command")
            .about("\"install-service\" to install the service.\n\"uninstall-service\" to uninstall the service.\n\"start-service\" to start the service.\n\"run-service\" to run the service (cannot be used directly)\n\"stop-service\" to stop the service.")
            .required(true)
            .index(1))
        .get_matches();

    if !is_elevated() {
        println!("Please run as administrator");
        info!("main::run:: tried to run without administrator");
        return Ok(())
    }

    let command = matches.value_of("command").unwrap();

    match command {
        "install-service" => {
            let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
            let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

            let service_access = ServiceAccess::QUERY_STATUS;
            let res = service_manager.open_service(SERVICE_NAME, service_access);

            if let Err(e) = res {
                if let Some(code) = std::io::Error::last_os_error().raw_os_error() {
                    // ERROR_SERVICE_DOES_NOT_EXIST = 1060
                    if code == 1060 {
                        let service_binary_path = std::env::current_exe().unwrap();

                        let service_info = ServiceInfo {
                            name: OsString::from(SERVICE_NAME),
                            display_name: OsString::from("Docker Process Isolation Patcher"),
                            service_type: ServiceType::OWN_PROCESS,
                            start_type: ServiceStartType::AutoStart,
                            error_control: ServiceErrorControl::Normal,
                            executable_path: service_binary_path,
                            launch_arguments: vec![OsString::from("run-service")],
                            dependencies: vec![],
                            account_name: None, // run as System
                            account_password: None
                        };

                        let service = service_manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;
                        service.set_description("Docker Process Isolation Manager will automatically set Windows docker service to run in process isolation mode")?;

                        println!("Installed service");
                        info!("main::run::install-service: installed service");
                    } else {
                        if let Some(s) = e.source() {
                            error!("main::run::install-service: {}", s.to_string());
                        }

                        Result::Err(e)?
                    }
                }
            } else {
                println!("Service already installed. Try the uninstall-service command");
                info!("main::run::install-service: service already installed");
            }
        },

        "start-service" => {
            let manager_access = ServiceManagerAccess::CONNECT;
            let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

            let res = service_manager.open_service(SERVICE_NAME,
                ServiceAccess::START | ServiceAccess::QUERY_STATUS
            );

            match res {
                Ok(service) => {
                    let status = service.query_status()?.current_state;
                    match status {
                        ServiceState::Stopped => {
                            let res = service.start(&[] as &[&str]);

                            if let Err(e) = res {
                                error!("main::run::start-service: failed to start service: {:?}", e);
                                println!("Failed to start service");
                            } else {
                                info!("main::run::start-service: started service");
                                println!("Started service");
                            }
                        }

                        ServiceState::Running => {
                            info!("main::run::start-service: service already running");
                            println!("Service already running");
                        }

                        _ => {
                            info!("main::run::start-service: service not in running or stopped state - service state: {:?}", status);
                            println!("Service neither stopped nor running. Please try again");
                        }
                    }
                }

                Err(_) => {
                    info!("main::run::start-service: tried to start service, but it's missing");
                    println!("Service not found. Is it installed?");
                }
            }
        }

        "stop-service" => {
            let manager_access = ServiceManagerAccess::CONNECT;
            let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

            let service_access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP;
            let res = service_manager.open_service(SERVICE_NAME, service_access);
            if let Ok(service) = res {
                stop_service(&service, true)?;
            } else {
                info!("main::run::stop-service: tried to stop service, but it's missing");
                println!("Service not found. Is it installed?");
            }
        }

        "uninstall-service" => {
            let manager_access = ServiceManagerAccess::CONNECT;
            let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

            let service_access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
            let res = service_manager.open_service(SERVICE_NAME, service_access);
            if let Ok(service) = res {
                stop_service(&service, false)?;

                if let Some(e)  = service.delete().err() {
                    if let Some(code) = std::io::Error::last_os_error().raw_os_error() {
                        // bubble up error if not the right one
                        // ERROR_SERVICE_MARKED_FOR_DELETE - it's already uninstalled
                        if code == 1072 {
                            info!("main::run::uninstall-service: tried to uninstall missing service - ERROR_SERVICE_MARKED_FOR_DELETE");
                            println!("Service not found. Is it installed? If Windows services manager is open, please close it to let the service delete");
                        } else {
                            error!("main::run::uninstall-service: {:?}", e);
                            println!("Failed to uninstall service");
                            Result::Err(e)?
                        }
                    }
                } else {
                    info!("main::run::uninstall-service: uninstalled service");
                    println!("Uninstalled service");
                }
            } else {
                info!("main::run::uninstall-service: tried to uninstall missing service");
                println!("Service not found. Is it installed?");
            }
        }

        "run-service" => {
            let manager_access = ServiceManagerAccess::CONNECT;
            let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

            let service_access = ServiceAccess::QUERY_STATUS;
            let res = service_manager.open_service(SERVICE_NAME, service_access);

            if let Ok(service) = res {
                let service_status = service.query_status()?.current_state;

                match service_status {
                    ServiceState::Stopped | ServiceState::StartPending => {
                        info!("main::run::run-service: running service");
                        service::run()?;

                        if let Some(code) = std::io::Error::last_os_error().raw_os_error() {
                            // this was run directly
                            // ERROR_FAILED_SERVICE_CONTROLLER_CONNECT
                            if code == 1063 {
                                info!("main::run::run-service: tried to run service directly - ERROR_FAILED_SERVICE_CONTROLLER_CONNECT");
                                println!("Do not run directly. Please use the start-service command");
                            }
                        }
                    }

                    _ => {
                        info!("main::run::run-service: tried to run service, but its status is {:?}", service_status);
                        println!("Service already running");
                    }
                }
            } else {
                info!("main::run::run-service: tried to run service, but it wasn't found");
                println!("Service not found. Is it installed?");
            }
        }

        _ => {
            info!("main::run::_: invalid command {}", command);
            println!("Invalid command. Please see help for commands");
        }
    }

    Ok(())
}

fn stop_service(service: &Service, is_stop_command: bool) -> windows_service::Result<()> {
    let mut service_status = service.query_status()?;

    if service_status.current_state != ServiceState::Stopped {
        info!("main::stop_service: stopping service");

        if let Ok(_) = service.stop() {
            // Wait for service to stop
            let mut elapsed_time = 0;
            let mut printed = false;
            loop {
                service_status = service.query_status()?;

                if service_status.current_state != ServiceState::Stopped {
                    if !printed {
                        print_flush!("Stopping service.");
                        printed = true;
                    }

                    if elapsed_time >= 10000 {
                        // really, should've been long enough..
                        error!("main::stop_service: service timed out");
                        print_flush!("failed");

                        if is_stop_command {
                            println!();
                        } else {
                            print_flush!("...");
                        }
                        break;
                    }

                    print_flush!(".");

                    std::thread::sleep(Duration::from_millis(250));
                    elapsed_time += 250;
                } else {
                    info!("main::stop_service: stopped service");

                    if printed == true {
                        print_flush!("stopped");
                        if is_stop_command {
                            println!();
                        } else {
                            print_flush!("...");
                        }
                    } else if is_stop_command {
                        println!("Stopped service")
                    }
                    break;
                }
            }
        }
    } else {
        info!("main::stop_service: tried to stop service, but it was already stopped");
        if is_stop_command {
            println!("Service already stopped");
        }
    }

    Ok(())
}
