//! Settings from the environment, validated once at startup.

use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

use massive_proto::ids::HostName;

use crate::{
    grid::GridConfig,
    groups::GroupConfig,
    programme::{self, PrimetimeSlots, Programme},
    schedule::ScheduleConfig,
    storage::PostgresUrl,
    store::SimulationOptions,
    time::Zone,
    wire::Protocol,
};

/// One environment variable: what `.env.example` documents (checked by `tests/env_example.rs`).
pub struct Setting {
    pub name: &'static str,
    /// The value when unset, as written in `.env.example`; `None` for required or optional ones.
    pub default: Option<&'static str>,
}

const fn setting(name: &'static str, default: Option<&'static str>) -> Setting {
    Setting { name, default }
}

pub const SETTINGS: &[Setting] = &[
    setting("GAMESERVER_BIND", None),
    setting("GAMESERVER_LISTEN_IP", None),
    setting("GAMESERVER_PROTOCOL", None),
    setting("GAMESERVER_HTTP_ADDR", None),
    setting("GAMESERVER_ADMIN_ADDR", Some("127.0.0.1:8081")),
    setting("GAMESERVER_STATUS_ADDR", Some("127.0.0.1:8082")),
    setting("GAMESERVER_DATA_DIR", Some("data")),
    setting("GAMESERVER_DATABASE_URL", None),
    setting("GAMESERVER_LOG_DIR", Some("logs")),
    setting("GAMESERVER_LOG", Some("info")),
    setting("GAMESERVER_SYNTHETIC_MOB", Some("1")),
    setting("GAMESERVER_SERVER_FLOW", Some("1")),
    setting("GAMESERVER_SYNTHETIC_CROWD", Some("60")),
    setting("GAMESERVER_PT_MIN_END_MINUTES", Some("5")),
    setting("GAMESERVER_PROGRAMME_ZONE", Some("America/New_York")),
    setting("GAMESERVER_PRIMETIME_AT", Some("20:00")),
    setting("GAMESERVER_PRIMETIME_MINUTES", Some("60")),
    setting("GAMESERVER_EXTENDED_PLAY_MINUTES", Some("30")),
    setting("GAMESERVER_GROUP_SIZE", Some("4")),
    setting("GAMESERVER_GROUP_GRACE_SECS", Some("30")),
    setting(
        "GAMESERVER_LAUNCH_CONTENT_ID",
        Some("988237803712CB87513237A914D367C6AAEE2410"),
    ),
    setting(
        "GAMESERVER_SHOW_CONTENT_ID",
        Some("77205BC1241C708A8A10B7303AF90E5EDD0286B1"),
    ),
    setting(
        "GAMESERVER_EPISODE_CONTENT_ID",
        Some("83136FF343124795DC1806A2DB044476589BD5C0"),
    ),
    setting("GAMESERVER_DUMP_GAMESTATE", None),
    setting("GAMESERVER_RECORD_SBG", None),
    setting("GAMESERVER_ADS_CONFIG", None),
    setting("GAMESERVER_PUBLIC_HOST", None),
    setting("GAMESERVER_LIVEHOST_FILE", None),
];

/// Server settings; test `Default`s keep simulation and flow off.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The host's LAN IP: the address the consoles reach and are told about.
    pub bind_ip: IpAddr,
    /// The IP the game sockets bind to. A container binds its own address while the host
    /// publishes the ports on `bind_ip`.
    pub listen_ip: IpAddr,
    pub protocol: Protocol,
    /// Grid and livehost HTTP; the console itself always dials port 80 of `bind_ip`, which is
    /// this address unless a proxy or a container's published port forwards it.
    pub http_addr: SocketAddr,
    pub admin_addr: SocketAddr,
    /// The public, read-only status API (`status`).
    pub status_addr: SocketAddr,
    pub database: Database,
    pub simulation: SimulationOptions,
    pub server_flow: bool,
    pub schedule: ScheduleConfig,
    /// Starting programme for an empty database; then the saved one rules.
    pub programme: Programme,
    pub grid: GridConfig,
    pub groups: GroupConfig,
    /// Directory to write every posted GameState document to.
    pub dump_game_state: Option<PathBuf>,
    /// File to record every SBG frame to (`sbg::record`).
    pub record_sbg: Option<PathBuf>,
    /// massived on the ads port; `None` serves the built-in endpoint without ad content.
    pub ads: Option<AdsConfig>,
    /// ASF file served as the live host stream; `None` serves the built-in asset.
    pub livehost_file: Option<PathBuf>,
}

/// Where persistent data lives.
#[derive(Clone, Debug, PartialEq)]
pub enum Database {
    /// Nothing survives a restart.
    Memory,
    /// The SQLite file `onevs100.db` in this directory.
    Sqlite {
        dir: PathBuf,
    },
    Postgres {
        url: PostgresUrl,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdsConfig {
    /// massived configuration file: zones and assets.
    pub config: PathBuf,
    /// Where the ad client is told to send its later requests, with the ads port.
    pub advertise_host: HostName,
}

/// Logging; set up first so setting errors are logged.
pub struct Logging {
    /// A `tracing` filter; when unset or invalid, `RUST_LOG`, then `info`.
    pub filter: Option<String>,
    pub dir: PathBuf,
}

/// Retired settings and replacements; setting one stops the server.
const RETIRED: &[(&str, &str)] = &[
    (
        "GAMESERVER_SCHEDULE",
        "the daily programme replaced it: GAMESERVER_PROGRAMME_ZONE, GAMESERVER_PRIMETIME_AT, \
         ... seed it, the admin page (/admin/programme) changes it",
    ),
    (
        "GAMESERVER_SCHEDULE_UTC_OFFSET_MIN",
        "the programme has a time zone instead: GAMESERVER_PROGRAMME_ZONE",
    ),
    (
        "GAMESERVER_STABLE_SESSION",
        "the programme's shows are fixed and saved; remove it",
    ),
    (
        "GAMESERVER_SESSION_LEAD_SECS",
        "the programme always has a show running; remove it",
    ),
];

#[allow(clippy::disallowed_methods)]
pub fn logging() -> Logging {
    Logging {
        filter: std::env::var("GAMESERVER_LOG").ok(),
        dir: std::env::var_os("GAMESERVER_LOG_DIR").map_or_else(|| "logs".into(), Into::into),
    }
}

impl Config {
    #[allow(clippy::disallowed_methods)]
    pub fn from_env() -> Result<Config, Vec<String>> {
        let mut vars = BTreeMap::new();
        let mut errors = Vec::new();
        for (name, value) in std::env::vars_os() {
            let Some(name) = name.to_str().filter(|name| name.starts_with("GAMESERVER_")) else {
                continue;
            };
            match value.into_string() {
                Ok(value) => {
                    vars.insert(name.to_owned(), value);
                }
                Err(_) => errors.push(format!("{name} must be valid UTF-8")),
            }
        }
        match Config::from_map(&vars) {
            Ok(config) if errors.is_empty() => Ok(config),
            Ok(_) => Err(errors),
            Err(more) => Err(errors.into_iter().chain(more).collect()),
        }
    }

    pub fn from_map(vars: &BTreeMap<String, String>) -> Result<Config, Vec<String>> {
        let mut errors: Vec<String> = RETIRED
            .iter()
            .filter(|(name, _)| vars.contains_key(*name))
            .map(|(name, why)| format!("{name} is no longer a setting: {why}"))
            .collect();
        let mut read = Reader {
            vars,
            errors: &mut errors,
        };
        // The consoles are told this address, so it must name one interface.
        let bind_ip = read.required_with(
            "GAMESERVER_BIND",
            "required (the host's LAN IP the console reaches)",
            "a concrete IP address, not 0.0.0.0 or ::",
            |value| {
                value
                    .parse::<IpAddr>()
                    .ok()
                    .filter(|ip| !ip.is_unspecified())
            },
        );
        let listen_ip = read.value("GAMESERVER_LISTEN_IP", "an IP address", |_| true);
        let protocol = read.required_with(
            "GAMESERVER_PROTOCOL",
            "required: 1149 or 1317 (the title build)",
            "1149 or 1317",
            Protocol::parse,
        );
        let http_addr = read.value(
            "GAMESERVER_HTTP_ADDR",
            "an ip:port address with a port other than 0",
            |addr: &SocketAddr| addr.port() != 0,
        );
        let admin_addr = read
            .value("GAMESERVER_ADMIN_ADDR", "an ip:port address", |_| true)
            .unwrap_or(crate::server::DEFAULT_ADMIN_ADDR);
        let status_addr = read
            .value("GAMESERVER_STATUS_ADDR", "an ip:port address", |_| true)
            .unwrap_or(crate::server::DEFAULT_STATUS_ADDR);
        let database = read.database();
        let simulation = SimulationOptions {
            synthetic_mob: read.flag("GAMESERVER_SYNTHETIC_MOB"),
            synthetic_crowd: read
                .value("GAMESERVER_SYNTHETIC_CROWD", "a member count", |_| true)
                .unwrap_or(DEFAULT_SYNTHETIC_CROWD),
        };
        let server_flow = read.flag("GAMESERVER_SERVER_FLOW");
        let schedule = read.schedule();
        let programme = read.programme();
        let grid = read.grid();
        let groups = read.groups();
        let dump_game_state = vars.get("GAMESERVER_DUMP_GAMESTATE").map(Into::into);
        let record_sbg = vars.get("GAMESERVER_RECORD_SBG").map(Into::into);
        let livehost_file = vars.get("GAMESERVER_LIVEHOST_FILE").map(Into::into);
        let public_host = read.value_with(
            "GAMESERVER_PUBLIC_HOST",
            "a host name or IPv4 address",
            |value| HostName::new(value).ok(),
        );
        let ads = match (vars.get("GAMESERVER_ADS_CONFIG"), public_host, bind_ip) {
            (None, _, _) => None,
            (Some(config), Some(advertise_host), _) => Some(AdsConfig {
                config: config.into(),
                advertise_host,
            }),
            (Some(config), None, Some(IpAddr::V4(ip))) => Some(AdsConfig {
                config: config.into(),
                advertise_host: HostName::new(ip.to_string())
                    .expect("an IPv4 address is a host name"),
            }),
            (Some(_), None, Some(_)) => {
                read.errors.push(
                    "GAMESERVER_PUBLIC_HOST is required with GAMESERVER_ADS_CONFIG when GAMESERVER_BIND is not an IPv4 address".into(),
                );
                None
            }
            (Some(_), None, None) => None,
        };
        match (bind_ip, protocol) {
            (Some(bind_ip), Some(protocol)) if errors.is_empty() => {
                // Outside a container the sockets bind the address the consoles reach.
                let listen_ip = listen_ip.unwrap_or(bind_ip);
                Ok(Config {
                    bind_ip,
                    listen_ip,
                    protocol,
                    http_addr: http_addr.unwrap_or(SocketAddr::new(listen_ip, DEFAULT_HTTP_PORT)),
                    admin_addr,
                    status_addr,
                    database,
                    simulation,
                    server_flow,
                    schedule,
                    programme,
                    grid,
                    groups,
                    dump_game_state,
                    record_sbg,
                    ads,
                    livehost_file,
                })
            }
            _ => Err(errors),
        }
    }
}

/// The port the console sends Grid and livehost requests to.
const DEFAULT_HTTP_PORT: u16 = 80;

/// Simulated crowd members unless `GAMESERVER_SYNTHETIC_CROWD` says otherwise (`0` = none).
const DEFAULT_SYNTHETIC_CROWD: u32 = 60;

struct Reader<'a> {
    vars: &'a BTreeMap<String, String>,
    errors: &'a mut Vec<String>,
}

impl Reader<'_> {
    /// `name`'s value when set and valid; an invalid one is noted as "`name` must be `what`".
    fn value<T: FromStr>(
        &mut self,
        name: &str,
        what: &str,
        valid: impl Fn(&T) -> bool,
    ) -> Option<T> {
        self.value_with(name, what, |value| {
            value.parse().ok().filter(|parsed| valid(parsed))
        })
    }

    /// `GAMESERVER_DATABASE_URL` when set, else `GAMESERVER_DATA_DIR`. An invalid URL is
    /// reported without its value, which can hold a password.
    fn database(&mut self) -> Database {
        if let Some(url) = self.vars.get("GAMESERVER_DATABASE_URL") {
            return match PostgresUrl::parse(url) {
                Some(url) => Database::Postgres { url },
                None => {
                    self.errors.push(
                        "GAMESERVER_DATABASE_URL must be a PostgreSQL connection string".into(),
                    );
                    Database::Memory
                }
            };
        }
        match self.vars.get("GAMESERVER_DATA_DIR").map(String::as_str) {
            Some("memory") => Database::Memory,
            Some(dir) => Database::Sqlite { dir: dir.into() },
            None => Database::Sqlite { dir: "data".into() },
        }
    }

    fn value_with<T>(
        &mut self,
        name: &str,
        what: &str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Option<T> {
        let value = self.vars.get(name)?;
        let parsed = parse(value);
        if parsed.is_none() {
            self.errors
                .push(format!("{name} must be {what}, got {value:?}"));
        }
        parsed
    }

    fn required_with<T>(
        &mut self,
        name: &str,
        missing: &str,
        what: &str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Option<T> {
        if !self.vars.contains_key(name) {
            self.errors.push(format!("{name} is {missing}"));
        }
        self.value_with(name, what, parse)
    }

    /// A switch that is on unless set to `0`.
    fn flag(&mut self, name: &str) -> bool {
        self.value_with(name, "0 or 1", |value| match value {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        })
        .unwrap_or(true)
    }

    fn secs(&mut self, name: &str, default: Duration) -> Duration {
        self.value(name, "a number of seconds", |_| true)
            .map_or(default, Duration::from_secs)
    }

    fn schedule(&mut self) -> ScheduleConfig {
        let defaults = ScheduleConfig::default();
        ScheduleConfig {
            pt_min_end: self
                .value(
                    "GAMESERVER_PT_MIN_END_MINUTES",
                    "a number of minutes",
                    |_| true,
                )
                .map_or(defaults.pt_min_end, programme::minutes),
        }
    }

    fn programme(&mut self) -> Programme {
        let defaults = Programme::default();
        let length = |read: &mut Self, name: &str, default: Duration| {
            read.value(name, "a number of minutes", |_| true)
                .map_or(default, programme::minutes)
        };
        let programme = Programme {
            zone: self
                .value_with(
                    "GAMESERVER_PROGRAMME_ZONE",
                    "an IANA time zone (e.g. America/New_York)",
                    Zone::get,
                )
                .unwrap_or(defaults.zone),
            primetime_at: self
                .value_with(
                    "GAMESERVER_PRIMETIME_AT",
                    "times HH:MM separated by commas, each once",
                    |text| PrimetimeSlots::parse(text).ok(),
                )
                .unwrap_or(defaults.primetime_at),
            primetime: length(self, "GAMESERVER_PRIMETIME_MINUTES", defaults.primetime),
            extended_play: length(
                self,
                "GAMESERVER_EXTENDED_PLAY_MINUTES",
                defaults.extended_play,
            ),
            min_extended_play: defaults.min_extended_play,
        };
        // The shortest Extended Play show is never longer than a whole one.
        let programme = Programme {
            min_extended_play: programme.min_extended_play.min(programme.extended_play),
            ..programme
        };
        if let Err(error) = programme.validate() {
            self.errors.push(format!("programme: {error}"));
        }
        programme
    }

    fn grid(&mut self) -> GridConfig {
        let defaults = GridConfig::default();
        let mut content_id = |name: &str, default: String| {
            self.value_with(name, "40 hex digits", |id| {
                is_content_id(id).then(|| id.to_ascii_uppercase())
            })
            .unwrap_or(default)
        };
        GridConfig {
            launch_content_id: content_id(
                "GAMESERVER_LAUNCH_CONTENT_ID",
                defaults.launch_content_id,
            ),
            show_content_id: content_id("GAMESERVER_SHOW_CONTENT_ID", defaults.show_content_id),
            episode_content_id: content_id(
                "GAMESERVER_EPISODE_CONTENT_ID",
                defaults.episode_content_id,
            ),
        }
    }

    fn groups(&mut self) -> GroupConfig {
        let defaults = GroupConfig::default();
        let max = crate::groups::MAX_GROUP_SIZE;
        GroupConfig {
            size: self
                .value(
                    "GAMESERVER_GROUP_SIZE",
                    &format!("a group size from 1 to {max}"),
                    |size: &usize| (1..=max).contains(size),
                )
                .unwrap_or(defaults.size),
            grace: self.secs("GAMESERVER_GROUP_GRACE_SECS", defaults.grace),
        }
    }
}

/// A marketplace content id: 40 hex digits.
pub fn is_content_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    const REQUIRED: [(&str, &str); 2] = [
        ("GAMESERVER_BIND", "192.0.2.10"),
        ("GAMESERVER_PROTOCOL", "1317"),
    ];

    #[test]
    fn the_documented_defaults_are_the_ones_in_effect() {
        let unset = Config::from_map(&vars(&REQUIRED)).unwrap();
        let mut spelled_out = vars(&REQUIRED);
        for setting in SETTINGS {
            if let Some(default) = setting.default {
                spelled_out.insert(setting.name.into(), default.into());
            }
        }
        assert_eq!(Config::from_map(&spelled_out).unwrap(), unset);
        assert_eq!(unset.admin_addr, crate::server::DEFAULT_ADMIN_ADDR);
        assert_eq!(unset.status_addr, crate::server::DEFAULT_STATUS_ADDR);
        assert_eq!(unset.http_addr, "192.0.2.10:80".parse().unwrap());
        assert_eq!(unset.database, Database::Sqlite { dir: "data".into() });
        assert!(unset.simulation.synthetic_mob && unset.server_flow);
        assert_eq!(unset.simulation.synthetic_crowd, 60);
    }

    #[test]
    fn every_invalid_setting_is_reported_at_once() {
        let errors = Config::from_map(&vars(&[
            ("GAMESERVER_BIND", "not an ip"),
            ("GAMESERVER_SERVER_FLOW", "off"),
            ("GAMESERVER_PRIMETIME_AT", "25:00"),
            ("GAMESERVER_GROUP_SIZE", "5"),
            ("GAMESERVER_SHOW_CONTENT_ID", "1VS100SHOWPKG0000010"),
            ("GAMESERVER_STABLE_SESSION", "1"),
        ]))
        .unwrap_err();
        let expected = [
            "GAMESERVER_STABLE_SESSION is no longer a setting: the programme's shows are fixed and saved; remove it",
            "GAMESERVER_BIND must be a concrete IP address, not 0.0.0.0 or ::, got \"not an ip\"",
            "GAMESERVER_PROTOCOL is required: 1149 or 1317 (the title build)",
            "GAMESERVER_SERVER_FLOW must be 0 or 1, got \"off\"",
            "GAMESERVER_PRIMETIME_AT must be times HH:MM separated by commas, each once, got \"25:00\"",
            "GAMESERVER_SHOW_CONTENT_ID must be 40 hex digits, got \"1VS100SHOWPKG0000010\"",
            "GAMESERVER_GROUP_SIZE must be a group size from 1 to 4, got \"5\"",
        ];
        assert_eq!(errors, expected);
    }

    #[test]
    fn ads_advertise_the_public_host_else_an_ipv4_bind() {
        let ads = |pairs: &[(&str, &str)]| {
            let mut vars = vars(pairs);
            vars.insert("GAMESERVER_PROTOCOL".into(), "1149".into());
            vars.insert("GAMESERVER_ADS_CONFIG".into(), "massived.toml".into());
            Config::from_map(&vars).map(|config| config.ads.map(|ads| ads.advertise_host))
        };
        let host = |name: &str| Ok(Some(HostName::new(name).unwrap()));
        assert_eq!(
            ads(&[("GAMESERVER_BIND", "192.0.2.10")]),
            host("192.0.2.10")
        );
        assert_eq!(
            ads(&[
                ("GAMESERVER_BIND", "2001:db8::10"),
                ("GAMESERVER_PUBLIC_HOST", "ads.example.net")
            ]),
            host("ads.example.net")
        );
        assert!(ads(&[("GAMESERVER_BIND", "2001:db8::10")]).is_err());
    }

    #[test]
    fn values_are_strict() {
        let with = |name: &str, value: &str| {
            let mut vars = vars(&REQUIRED);
            vars.insert(name.into(), value.into());
            Config::from_map(&vars)
        };
        for bad in ["", "+", "-3", "7 ", "seven"] {
            assert!(with("GAMESERVER_GROUP_GRACE_SECS", bad).is_err(), "{bad:?}");
        }
        assert!(with("GAMESERVER_PROTOCOL", "1167").is_err());
        assert!(with("GAMESERVER_SYNTHETIC_MOB", "true").is_err());
        for retired in [
            "GAMESERVER_SCHEDULE",
            "GAMESERVER_SCHEDULE_UTC_OFFSET_MIN",
            "GAMESERVER_STABLE_SESSION",
            "GAMESERVER_SESSION_LEAD_SECS",
        ] {
            let errors = with(retired, "1").unwrap_err();
            assert!(errors[0].starts_with(&format!("{retired} is no longer a setting")));
        }
        assert_eq!(
            with("GAMESERVER_HTTP_ADDR", "127.0.0.1:8090")
                .unwrap()
                .http_addr,
            "127.0.0.1:8090".parse().unwrap()
        );
        assert!(with("GAMESERVER_HTTP_ADDR", "127.0.0.1").is_err());
        assert!(with("GAMESERVER_HTTP_ADDR", "127.0.0.1:0").is_err());
        let unset = Config::from_map(&vars(&REQUIRED)).unwrap();
        assert_eq!(unset.listen_ip, unset.bind_ip);
        let container = with("GAMESERVER_LISTEN_IP", "0.0.0.0").unwrap();
        assert_eq!(container.bind_ip, "192.0.2.10".parse::<IpAddr>().unwrap());
        assert_eq!(container.listen_ip, "0.0.0.0".parse::<IpAddr>().unwrap());
        assert_eq!(container.http_addr, "0.0.0.0:80".parse().unwrap());
        assert!(with("GAMESERVER_LISTEN_IP", "not an ip").is_err());
        assert!(with("GAMESERVER_BIND", "0.0.0.0").is_err());
        assert!(with("GAMESERVER_BIND", "::").is_err());
        assert_eq!(
            with("GAMESERVER_DATA_DIR", "memory").unwrap().database,
            Database::Memory
        );
        let url = "postgres://onevs100:secret@postgres/onevs100";
        assert_eq!(
            with("GAMESERVER_DATABASE_URL", url).unwrap().database,
            Database::Postgres {
                url: PostgresUrl::parse(url).unwrap()
            }
        );
        for bad in ["postgres://:secret@[", "", "postgres:///onevs100"] {
            assert_eq!(
                with("GAMESERVER_DATABASE_URL", bad).unwrap_err(),
                ["GAMESERVER_DATABASE_URL must be a PostgreSQL connection string"],
                "{bad:?}"
            );
        }
        let id = "77205bc1241c708a8a10b7303af90e5edd0286b1";
        let grid = with("GAMESERVER_SHOW_CONTENT_ID", id).unwrap().grid;
        assert_eq!(grid.show_content_id, id.to_ascii_uppercase());
        assert!(!is_content_id(&"G".repeat(40)));
    }

    #[test]
    fn the_programme_seed_is_validated() {
        let with = |pairs: &[(&str, &str)]| {
            let mut vars = vars(&REQUIRED);
            for (name, value) in pairs {
                vars.insert((*name).into(), (*value).into());
            }
            Config::from_map(&vars)
        };
        let programme = with(&[
            ("GAMESERVER_PROGRAMME_ZONE", "America/Los_Angeles"),
            ("GAMESERVER_PRIMETIME_AT", "22:00,19:30"),
            ("GAMESERVER_EXTENDED_PLAY_MINUTES", "25"),
        ])
        .unwrap()
        .programme;
        assert_eq!(programme.zone.name(), "America/Los_Angeles");
        assert_eq!(programme.primetime_at.to_string(), "19:30,22:00");
        assert_eq!(programme.extended_play, crate::programme::minutes(25));
        for bad in [
            [("GAMESERVER_PROGRAMME_ZONE", "Pacific Time")],
            [("GAMESERVER_PRIMETIME_AT", "8pm")],
            [("GAMESERVER_PRIMETIME_AT", "20:00,20:00")],
            [("GAMESERVER_PRIMETIME_AT", "20:00,20:30")],
            [("GAMESERVER_PRIMETIME_MINUTES", "5")],
            [("GAMESERVER_EXTENDED_PLAY_MINUTES", "600")],
            [("GAMESERVER_EXTENDED_PLAY_MINUTES", "45")],
        ] {
            assert!(with(&bad).is_err(), "{bad:?}");
        }
    }
}
