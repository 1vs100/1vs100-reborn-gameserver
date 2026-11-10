use std::{
    io,
    net::{IpAddr, SocketAddr, TcpListener, UdpSocket},
    panic::{self, AssertUnwindSafe},
    sync::{Arc, mpsc},
    thread,
};

use rsa::{RsaPrivateKey, pkcs1::DecodeRsaPrivateKey};

use crate::{
    admin,
    app::App,
    build_info::CommitId,
    config::{AdsConfig, Config},
    http, matchmaking, sbg, sntp,
    store::Store,
    wire::Protocol,
};

/// Loopback only: the admin API is unauthenticated and can mutate state.
pub const DEFAULT_ADMIN_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 8081);

/// Loopback by default: a reverse proxy publishes it.
pub const DEFAULT_STATUS_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 8082);

/// What one server instance listens on; one title build per instance (`Ports::for_protocol`).
#[derive(Clone)]
struct Ports {
    /// UDP. v2.0.1149.0's own NTP client (`sub_82232848`) targets the SBG host on 1123,
    /// v2.0.1317.0 uses standard 123.
    sntp: u16,
    http: SocketAddr,
    /// v2.0.1149.0 hard-codes SBG port 1000 (`li r5,1000` at 0x8223ADB0)
    /// v2.0.1317.0 hard-codes SBG port 2007 (0x8224134C).
    /// sysdll ensures port is patched according to ini / default value (1000)
    sbg: u16,
    matchmaking: u16,
    /// Massive ad HTTP listeners. Both builds originally dial 1000; patched to port 1002 via sysdll.
    ads: u16,
    admin: SocketAddr,
    status: SocketAddr,
}

/// The listeners whose addresses are configured, not fixed by the title.
struct Listeners {
    http: SocketAddr,
    admin: SocketAddr,
    status: SocketAddr,
}

impl Ports {
    fn for_protocol(
        protocol: Protocol,
        Listeners {
            http,
            admin,
            status,
        }: Listeners,
    ) -> Self {
        let sntp = match protocol {
            Protocol::V1149 => 1123,
            Protocol::V1317 => 123,
        };
        Self {
            sntp,
            http,
            sbg: 1000,
            matchmaking: 1001,
            ads: 1002,
            admin,
            status,
        }
    }
}

struct BoundSockets {
    sntp: UdpSocket,
    http: TcpListener,
    matchmaking: TcpListener,
    sbg: TcpListener,
    ads: TcpListener,
    admin: TcpListener,
    status: TcpListener,
}

struct Service {
    name: &'static str,
    run: Box<dyn FnOnce() -> io::Result<()> + Send + 'static>,
}

impl Service {
    fn new(name: &'static str, run: impl FnOnce() -> io::Result<()> + Send + 'static) -> Self {
        Self {
            name,
            run: Box::new(run),
        }
    }
}

struct ServiceReport {
    name: &'static str,
    result: io::Result<()>,
}

pub fn run(config: Config, storage: Arc<dyn crate::storage::Storage>) -> io::Result<()> {
    let Config {
        bind_ip,
        listen_ip,
        protocol,
        http_addr,
        admin_addr,
        status_addr,
        simulation: options,
        server_flow,
        schedule,
        programme,
        grid,
        groups,
        dump_game_state,
        record_sbg,
        ads,
        livehost_file,
        database: _,
    } = config;
    let ports = Ports::for_protocol(
        protocol,
        Listeners {
            http: http_addr,
            admin: admin_addr,
            status: status_addr,
        },
    );
    let ads = match ads {
        Some(ads) => Ads::Massive(Box::new(load_massive_ads(&ads, ports.ads)?)),
        None => Ads::Empty,
    };
    let livehost =
        http::LivehostStream::load(livehost_file.as_deref()).map_err(io::Error::other)?;
    let build = crate::build_info::current();
    tracing::info!(
        built = %build.built.to_xsor(),
        commit = ?build.commit.map(CommitId::as_str),
        %bind_ip, %listen_ip, ?protocol, %http_addr, %admin_addr, %status_addr, ?options, server_flow,
        "starting onevs100-gameserver"
    );
    let settings = sbg::Settings {
        protocol,
        server_flow,
        dump_dir: dump_game_state,
        service_host: bind_ip.to_string(),
    };
    seed_programme(storage.as_ref(), &programme)?;
    let mut schedule = crate::schedule::Schedule::load(schedule, Arc::clone(&storage))
        .map_err(|error| io::Error::other(format!("schedule in storage: {error}")))?;
    schedule
        .fit_extended_play_to_one_episode(crate::time::now())
        .map_err(|error| io::Error::other(format!("programme in storage: {error}")))?;
    // Materialise the coming shows before a recording takes its storage snapshot: a replay serves
    // the recorded shows and their ids, it never plans its own.
    let planned = schedule.shows(crate::time::now()).len();
    tracing::info!(planned, "show programme loaded");
    let schedule = Arc::new(std::sync::Mutex::new(schedule));
    let key = signing_key()?;
    let questions = crate::questions::QuestionBank::in_effect(
        storage
            .question_bank()
            .map_err(|error| io::Error::other(format!("question bank in storage: {error}")))?,
    );
    tracing::info!(
        questions = questions.questions().len(),
        "question bank loaded"
    );
    let store = Store::bootstrap_with(options)
        .with_storage(Arc::clone(&storage))
        .with_questions(Arc::new(questions));
    let mut members = matchmaking::Members::default();
    members.set_group_config(groups);
    let app = Arc::new(App::new(settings, schedule, store, members));
    if let Some(path) = record_sbg {
        sbg::capture::start_recording(&app, &path, sbg::capture::ExistingFile::Replace)
            .map_err(io::Error::other)?;
    }
    with_bound_sockets(listen_ip, &ports, |sockets| {
        serve_bound(sockets, protocol, key, grid, app, ads, livehost)
    })?
}

/// A database without a programme starts with the one from the settings; a saved one (set in the
/// admin page) stays in effect.
fn seed_programme(
    storage: &dyn crate::storage::Storage,
    programme: &crate::programme::Programme,
) -> io::Result<()> {
    let failed = |error: crate::storage::StorageError| {
        io::Error::other(format!("programme in storage: {error}"))
    };
    match storage.programme().map_err(failed)? {
        Some(saved) if saved != *programme => tracing::info!(
            zone = saved.zone.name(),
            primetime_at = %saved.primetime_at,
            "saved programme in effect; the GAMESERVER_PROGRAMME_* settings only seed an empty database"
        ),
        Some(_) => {}
        None => {
            storage.set_programme(programme).map_err(failed)?;
            tracing::info!(
                zone = programme.zone.name(),
                primetime_at = %programme.primetime_at,
                "programme seeded from the settings"
            );
        }
    }
    Ok(())
}

fn with_bound_sockets<T>(
    listen_ip: IpAddr,
    ports: &Ports,
    after_bind: impl FnOnce(BoundSockets) -> T,
) -> io::Result<T> {
    let sockets = bind_all(listen_ip, ports)?;
    tracing::info!(%listen_ip, http = %ports.http, admin = %ports.admin, status = %ports.status, "all sockets bound");
    Ok(after_bind(sockets))
}

fn bind_all(listen_ip: IpAddr, ports: &Ports) -> io::Result<BoundSockets> {
    let tcp = |port| bind_tcp(SocketAddr::new(listen_ip, port));
    Ok(BoundSockets {
        sntp: bind_udp(SocketAddr::new(listen_ip, ports.sntp))?,
        http: bind_tcp(ports.http)?,
        matchmaking: tcp(ports.matchmaking)?,
        sbg: tcp(ports.sbg)?,
        ads: tcp(ports.ads)?,
        admin: bind_tcp(ports.admin)?,
        status: bind_tcp(ports.status)?,
    })
}

fn bind_udp(addr: SocketAddr) -> io::Result<UdpSocket> {
    tracing::debug!(%addr, "binding UDP");
    UdpSocket::bind(addr).map_err(|error| {
        tracing::error!(%addr, %error, "UDP bind failed");
        io::Error::new(error.kind(), format!("bind UDP {addr}: {error}"))
    })
}

fn bind_tcp(addr: SocketAddr) -> io::Result<TcpListener> {
    tracing::debug!(%addr, "binding TCP");
    TcpListener::bind(addr).map_err(|error| {
        tracing::error!(%addr, %error, "TCP bind failed");
        io::Error::new(error.kind(), format!("bind TCP {addr}: {error}"))
    })
}

/// What answers the ads port.
enum Ads {
    /// The built-in endpoint: it locates the service but offers no ad content.
    Empty,
    /// massived, with the content of its configuration file.
    Massive(Box<MassiveAds>),
}

struct MassiveAds {
    config: massived::config::Config,
    handler: massived::handler::Handler,
}

/// Reads the configuration and every asset before any port opens.
fn load_massive_ads(ads: &AdsConfig, port: u16) -> io::Result<MassiveAds> {
    let failed = |error: &dyn std::fmt::Display| {
        io::Error::other(format!("ads config {}: {error}", ads.config.display()))
    };
    let config = massived::config::Config::from_file(&ads.config).map_err(|e| failed(&e))?;
    let advertise = massived::config::Advertise {
        host: ads.advertise_host.clone(),
        port: massive_proto::ids::Port(port),
    };
    let handler = massived::handler::Handler::new(&config, &advertise).map_err(|e| failed(&e))?;
    Ok(MassiveAds { config, handler })
}

/// `service_host` is the address the built-in endpoint tells the ad client to use.
fn serve_ads(listener: TcpListener, ads: Ads, service_host: String) -> io::Result<()> {
    let MassiveAds { config, handler } = match ads {
        Ads::Empty => return http::serve_ads(listener, service_host),
        Ads::Massive(ads) => *ads,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let server = massived::server::Server::with_std_listeners(&config, handler, vec![listener])
            .await
            .map_err(io::Error::other)?;
        server.serve(std::future::pending()).await;
        Ok(())
    })
}

fn serve_bound(
    sockets: BoundSockets,
    protocol: Protocol,
    key: RsaPrivateKey,
    grid: crate::grid::GridConfig,
    app: Arc<App>,
    ads: Ads,
    livehost: http::LivehostStream,
) -> io::Result<()> {
    let BoundSockets {
        sntp,
        http,
        matchmaking,
        sbg,
        ads: ads_listener,
        admin,
        status,
    } = sockets;

    let services = vec![
        Service::new("sntp", move || sntp::serve(&sntp)),
        Service::new("http", {
            let service = http::GridService {
                key,
                protocol,
                config: grid,
                schedule: Arc::clone(&app.schedule),
                livehost,
            };
            move || http::serve(http, service)
        }),
        Service::new("matchmaking", {
            let members = Arc::clone(&app.members);
            let registry = matchmaking::Registry::new(crate::sync::lock(&app.store).storage());
            move || matchmaking::serve(matchmaking, members, registry)
        }),
        Service::new("sbg", {
            let app = Arc::clone(&app);
            move || sbg::serve(sbg, app)
        }),
        Service::new("ads-http", {
            let service_host = app.sbg.service_host.clone();
            move || serve_ads(ads_listener, ads, service_host)
        }),
        Service::new("status", {
            let app = Arc::clone(&app);
            move || crate::status::serve(status, crate::status::Status::new(app))
        }),
        Service::new("admin", move || admin::serve(admin, app)),
    ];
    supervise_services(services)
}

fn supervise_services(services: Vec<Service>) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    for service in services {
        let tx = tx.clone();
        thread::spawn(move || {
            let name = service.name;
            tracing::info!(service = name, "service starting");
            let result = match panic::catch_unwind(AssertUnwindSafe(service.run)) {
                Ok(Ok(())) => Err(io::Error::other("service exited unexpectedly")),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(io::Error::other("service thread panicked")),
            };
            if let Err(error) = &result {
                tracing::error!(service = name, %error, "service stopped");
            }
            let _ = tx.send(ServiceReport { name, result });
        });
    }
    drop(tx);

    let report = rx
        .recv()
        .map_err(|_| io::Error::other("all service threads stopped without reporting"))?;
    report.result.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{} service failed: {error}", report.name),
        )
    })
}

fn signing_key() -> io::Result<RsaPrivateKey> {
    RsaPrivateKey::from_pkcs1_pem(include_str!("../keys/grid_lan_private.pem")).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("parse packaged grid_lan_private.pem: {error}"),
        )
    })
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn the_settings_seed_an_empty_database_only() {
        use crate::{
            programme::{PrimetimeSlots, Programme},
            storage::Storage,
        };
        let storage = crate::storage::MemoryStorage::default();
        let settings = Programme::default();
        seed_programme(&storage, &settings).unwrap();
        assert_eq!(storage.programme().unwrap(), Some(settings.clone()));

        let saved = Programme {
            primetime_at: PrimetimeSlots::parse("21:00").unwrap(),
            ..settings.clone()
        };
        storage.set_programme(&saved).unwrap();
        seed_programme(&storage, &settings).unwrap();
        assert_eq!(
            storage.programme().unwrap(),
            Some(saved),
            "the saved one stays"
        );
    }
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn reserve_tcp_ports(count: usize) -> Vec<TcpListener> {
        (0..count)
            .map(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
            .collect()
    }

    fn udp_reservation() -> UdpSocket {
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap()
    }

    #[test]
    fn ports_follow_the_protocol() {
        let listeners = || Listeners {
            http: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8090),
            admin: DEFAULT_ADMIN_ADDR,
            status: DEFAULT_STATUS_ADDR,
        };
        let p1149 = Ports::for_protocol(Protocol::V1149, listeners());
        assert_eq!(
            (p1149.sntp, p1149.sbg, p1149.matchmaking, p1149.ads),
            (1123, 1000, 1001, 1002)
        );
        let p1317 = Ports::for_protocol(Protocol::V1317, listeners());
        assert_eq!(
            (p1317.sntp, p1317.sbg, p1317.matchmaking, p1317.ads),
            (123, 1000, 1001, 1002)
        );
    }

    /// Loopback ports that were free a moment ago, in `Ports` shape (two ads listeners).
    fn free_ports() -> Ports {
        let udp = udp_reservation();
        let tcp = reserve_tcp_ports(6);
        let port = |i: usize| tcp[i].local_addr().unwrap().port();
        Ports {
            sntp: udp.local_addr().unwrap().port(),
            http: tcp[0].local_addr().unwrap(),
            matchmaking: port(1),
            sbg: port(2),
            ads: port(3),
            admin: tcp[4].local_addr().unwrap(),
            status: tcp[5].local_addr().unwrap(),
        }
    }

    #[test]
    fn startup_bind_all_acquires_every_configured_loopback_socket() {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let ports = free_ports();

        let bound = bind_all(ip, &ports).unwrap();

        let at = |port| SocketAddr::new(ip, port);
        assert_eq!(bound.sntp.local_addr().unwrap(), at(ports.sntp));
        assert_eq!(bound.http.local_addr().unwrap(), ports.http);
        assert_eq!(
            bound.matchmaking.local_addr().unwrap(),
            at(ports.matchmaking)
        );
        assert_eq!(bound.sbg.local_addr().unwrap(), at(ports.sbg));
        assert_eq!(bound.ads.local_addr().unwrap(), at(ports.ads));
        assert_eq!(bound.admin.local_addr().unwrap(), ports.admin);
        assert_eq!(bound.status.local_addr().unwrap(), ports.status);
    }

    /// Linux routes all of 127.0.0.0/8 to loopback, so a second address is at hand.
    #[cfg(target_os = "linux")]
    #[test]
    fn startup_bind_all_binds_http_on_its_own_address() {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let other = TcpListener::bind((Ipv4Addr::new(127, 0, 0, 2), 0)).unwrap();
        let ports = Ports {
            http: other.local_addr().unwrap(),
            ..free_ports()
        };
        drop(other);

        let bound = bind_all(ip, &ports).unwrap();

        assert_eq!(bound.http.local_addr().unwrap(), ports.http);
    }

    #[test]
    fn startup_bind_all_rolls_back_earlier_sockets_on_late_failure() {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let ports = free_ports();
        let occupied_admin = TcpListener::bind(ports.admin).unwrap();
        let serving_started = AtomicBool::new(false);

        let error = with_bound_sockets(ip, &ports, |_| {
            serving_started.store(true, Ordering::SeqCst);
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(!serving_started.load(Ordering::SeqCst));
        UdpSocket::bind(SocketAddr::new(ip, ports.sntp)).unwrap();
        for port in [ports.matchmaking, ports.sbg, ports.ads].into_iter() {
            TcpListener::bind(SocketAddr::new(ip, port)).unwrap();
        }
        TcpListener::bind(ports.http).unwrap();
        drop(occupied_admin);
        TcpListener::bind(ports.admin).unwrap();
    }

    #[test]
    fn startup_supervisor_reports_immediate_failure_while_another_service_blocks() {
        use std::sync::mpsc;
        use std::time::Duration;

        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel::<()>();

        let error = supervise_services(vec![
            Service::new("blocked", move || {
                release_rx.recv().unwrap();
                done_tx.send(()).unwrap();
                Ok(())
            }),
            Service::new("failing", || {
                Err(io::Error::new(io::ErrorKind::InvalidData, "boom"))
            }),
        ])
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("failing"));
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn startup_supervisor_reports_unexpected_normal_return_as_failure() {
        let error = supervise_services(vec![Service::new("done", || Ok(()))]).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(error.to_string().contains("done"));
        assert!(error.to_string().contains("exited unexpectedly"));
    }

    #[test]
    fn startup_supervisor_reports_panic_as_service_failure() {
        type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync>;
        struct PanicHookGuard(Option<PanicHook>);

        impl Drop for PanicHookGuard {
            fn drop(&mut self) {
                if let Some(hook) = self.0.take() {
                    std::panic::set_hook(hook);
                }
            }
        }

        let _guard = PanicHookGuard(Some(std::panic::take_hook()));
        std::panic::set_hook(Box::new(|_| {}));

        let error = supervise_services(vec![Service::new("panicker", || -> io::Result<()> {
            panic!("boom")
        })])
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(error.to_string().contains("panicker"));
        assert!(error.to_string().contains("panicked"));
    }
}
