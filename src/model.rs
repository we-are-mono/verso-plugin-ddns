// SPDX-License-Identifier: GPL-2.0-only
// SPDX-FileCopyrightText: 2026 Mono Technologies Inc.

//! ddns-scripts' services as this page reads them: each `config service`
//! section, translated from the options each provider spells its own way into
//! one hostname and one secret, beside what its updater last did.

use verso_plugin::{Map, Request, Section, Value};

pub const CONFIG: &str = "ddns";

/// SAMPLES are the sections the ddns-scripts package ships, switched off and
/// naming yourhost.example.com: the config's shape, not a name anyone keeps.
const SAMPLES: [&str; 2] = ["myddns_ipv4", "myddns_ipv6"];

/// User is what a provider takes as `username`.
pub enum User {
    /// Nothing; the token alone signs the update in.
    None,
    /// An account name the person types, labelled so.
    Typed(&'static str),
    /// Always this word: Cloudflare's "Bearer" before an API token.
    Fixed(&'static str),
    /// The hostname itself: deSEC signs a name's update in as the name.
    Hostname,
}

pub struct Provider {
    /// The `service_name` ddns-scripts knows it by.
    pub id: &'static str,
    pub label: &'static str,
    pub user: User,
    /// What the provider calls the secret that goes in `password`.
    pub secret: &'static str,
}

/// PROVIDERS are the services the page names. Any other is an update URL,
/// which is what every one of these is underneath.
pub const PROVIDERS: [Provider; 5] = [
    Provider {
        id: "cloudflare.com-v4",
        label: "Cloudflare",
        user: User::Fixed("Bearer"),
        secret: "API token",
    },
    Provider {
        id: "duckdns.org",
        label: "DuckDNS",
        user: User::None,
        secret: "Token",
    },
    Provider {
        id: "no-ip.com",
        label: "No-IP",
        user: User::Typed("Username"),
        secret: "Password",
    },
    Provider {
        id: "desec.io",
        label: "deSEC",
        user: User::Hostname,
        secret: "Token",
    },
    Provider {
        id: "dynv6.com",
        label: "dynv6",
        user: User::None,
        secret: "Token",
    },
];

/// CUSTOM is the provider choice that is a URL rather than a named service.
pub const CUSTOM: &str = "custom";
pub const CLOUDFLARE: &str = "cloudflare.com-v4";
pub const DUCKDNS: &str = "duckdns.org";
pub const DUCKDNS_SUFFIX: &str = ".duckdns.org";

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

pub struct Ddns {
    pub services: Vec<Service>,
    /// The networks an update can take its address from.
    pub networks: Vec<String>,
    /// Whether the helper's read of the updaters arrived. Without it their
    /// state is unknown, which is not the same as stopped.
    pub known: bool,
}

/// Service is one section as the drawer edits it. Read from the section, or
/// posted from the drawer: a password is never read back, only whether one is
/// saved.
#[derive(Default, Clone)]
pub struct Service {
    pub name: String,
    pub enabled: bool,
    /// A `service_name`, or [`CUSTOM`] for an `update_url`.
    pub provider: String,
    /// The full name kept pointing here: `lookup_host`.
    pub hostname: String,
    /// Cloudflare's zone, the part after `@` in its `domain`.
    pub zone: String,
    pub username: String,
    pub password: String,
    pub saved_password: bool,
    pub url: String,
    pub v6: bool,
    /// `ip_source` as the section holds it; empty is ddns-scripts' `network`.
    pub source: String,
    /// The network the address is read from: `ip_network`, or what stands for it.
    pub network: String,
    /// `interface` as the section holds it: the network whose ifup starts the
    /// updater.
    pub interface: String,
    /// The section's options as uci holds them, for the drawer's preview.
    pub values: Map<String, Value>,
    pub live: Option<Live>,
}

#[derive(Default, Clone)]
pub struct Live {
    pub address: String,
    pub running: bool,
    pub updated: Option<u64>,
}

impl Ddns {
    pub fn read(request: &Request) -> Ddns {
        let state = request.ubus.get("ddnsState");
        let services = request
            .snapshot
            .sections_of_type(CONFIG, "service")
            .iter()
            .map(|s| Service::of(s, state))
            .filter(|s| s.enabled || !SAMPLES.contains(&s.name.as_str()))
            .collect();
        let networks = request
            .snapshot
            .sections_of_type("network", "interface")
            .iter()
            .map(Section::name)
            .filter(|n| n != "loopback")
            .collect();
        Ddns {
            services,
            networks,
            known: state.is_some(),
        }
    }

    pub fn service(&self, name: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.name == name)
    }
}

impl Service {
    fn of(s: &Section, state: Option<&Value>) -> Service {
        let name = s.name();
        let domain = s.scalar("domain");
        let (host, zone) = domain.split_once('@').unwrap_or((&domain, ""));
        // A section written before lookup_host was asked for names only its
        // domain; Cloudflare's spells the name host@zone.
        let hostname = match (s.scalar("lookup_host"), zone) {
            (h, _) if !h.is_empty() => h,
            (_, "") => domain.clone(),
            (_, z) if host.is_empty() => z.to_string(),
            (_, z) => format!("{host}.{z}"),
        };
        let v6 = s.scalar("use_ipv6") == "1";
        let network = [s.scalar("ip_network"), s.scalar("interface")]
            .into_iter()
            .find(|n| !n.is_empty())
            .unwrap_or_else(|| if v6 { "wan6" } else { "wan" }.into());
        let url = s.scalar("update_url");
        Service {
            enabled: s.scalar("enabled") == "1",
            provider: match s.scalar("service_name") {
                p if p.is_empty() && !url.is_empty() => CUSTOM.into(),
                p => p,
            },
            hostname,
            zone: zone.into(),
            username: s.scalar("username"),
            password: String::new(),
            saved_password: !s.scalar("password").is_empty(),
            url,
            v6,
            source: s.scalar("ip_source"),
            network,
            interface: s.scalar("interface"),
            values: s
                .entries()
                .filter(|(key, _)| !key.starts_with('.'))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            live: state
                .and_then(|st| st["services"].get(&name))
                .map(|l| Live {
                    address: l["address"].as_str().unwrap_or_default().into(),
                    running: l["running"].as_bool().unwrap_or(false),
                    updated: l["updated"].as_u64(),
                }),
            name,
        }
    }

    /// by_network is whether the address is read from one of the router's
    /// networks, the only source the drawer edits.
    pub fn by_network(&self) -> bool {
        matches!(self.source.as_str(), "" | "network")
    }

    pub fn provider_label(&self) -> &str {
        match (provider(&self.provider), self.provider.as_str()) {
            (Some(p), _) => p.label,
            (None, CUSTOM) => "Update URL",
            (None, other) => other,
        }
    }
}
