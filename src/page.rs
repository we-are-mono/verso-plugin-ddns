// SPDX-License-Identifier: GPL-2.0-only
// SPDX-FileCopyrightText: 2026 Mono Technologies Inc.

//! The Dynamic DNS page: one row per `config service`, each opened to its
//! drawer, and the heading's act opening a blank one.
//!
//! The drawer asks for a provider, a hostname and its secret, and writes what
//! ddns-scripts reads for that provider. It always writes `lookup_host`: an
//! updater that finds none sets it and runs `uci commit ddns` itself.

use std::net::IpAddr;

use verso_plugin::{
    json, uci_text, ApplyAction, ColumnWidth, CommitOp, Envelope, Errors, Field, Form, HeadingAct,
    Map, RowDrawer, SelectOption, Table, TableCell, TableChip, TableColumn, TableRow, TableRowAct,
    Tone, Value, Widget,
};

use crate::model::{
    provider, Ddns, Service, User, CLOUDFLARE, CONFIG, CUSTOM, DUCKDNS, DUCKDNS_SUFFIX, NETWORK,
    PROVIDERS, TIMING, WEB,
};

const HEADING: &str = "Dynamic DNS";
const EMPTY: &str = "No dynamic DNS yet. Add a hostname, and the router keeps it pointing at \
     its own address whenever the connection gets a new one.";
pub const REFUSED: &str = "Some values are missing, so nothing was saved. They’re marked below.";
const KEEP_HELP: &str = "Leave empty to keep the saved one.";
const DASH: &str = "—";
const MASK: &str = "••••••••";

/// OPEN is the query key naming the section whose drawer is open; NEW opens a
/// blank one.
pub const OPEN: &str = "open";
pub const NEW: &str = "new";
const DELETE: &str = "delete";
/// UPDATE is a row's "update now", naming its section.
pub const UPDATE: &str = "update";
/// SENDING is said once "update now" is on its way. It names no name: the
/// row pressed is the one, and a sentence holding a value has no catalog key.
const SENDING: &str = "Sending the address now.";

fn href(name: &str) -> String {
    format!("/plugins/ddns/?{OPEN}={name}")
}

/// page is the listing, with `open`'s drawer in front of it.
pub fn page(d: &Ddns, open: &str) -> Envelope {
    let drawer = match open {
        NEW => Some(drawer(d, None, &blank(d), &Errors::default())),
        name => d
            .service(name)
            .map(|s| drawer(d, Some(s), s, &Errors::default())),
    };
    listing(d, open, drawer)
}

/// post answers a row's "update now", or the open drawer: its save, or its
/// confirmed delete.
pub fn post(d: &Ddns, open: &str, form: &Form) -> Envelope {
    let update = form.get(UPDATE);
    if !update.is_empty() {
        return match d.service(&update).filter(|s| s.enabled) {
            Some(s) => {
                let mut page = listing(d, "", None).with_notice(Tone::Success, SENDING);
                page.commands = vec![ApplyAction {
                    name: "ddns-update".into(),
                    args: [("section".into(), s.name.clone())].into(),
                }];
                page
            }
            None => listing(d, "", None).with_notice(
                Tone::Danger,
                "This name is not switched on, so nothing was sent.",
            ),
        };
    }
    let existing = d.service(open);
    if open != NEW && existing.is_none() {
        return listing(d, "", None).with_notice(
            Tone::Danger,
            "This name is no longer set up, so nothing was saved.",
        );
    }
    if let (Some(s), "1") = (existing, form.get(DELETE).as_str()) {
        return listing(d, "", None).with_commit(vec![CommitOp {
            config: CONFIG.into(),
            section: s.name.clone(),
            section_type: String::new(),
            delete: true,
            values: Value::Null,
        }]);
    }
    let posted = posted(existing, form);
    // A new address source is the drawer drawn again around it, unsaved.
    if form.get("_action") == "reshape" {
        let panel = drawer(d, existing, &posted, &Errors::default());
        return listing(d, open, Some(panel));
    }
    let errors = validate(d, existing, &posted);
    let answer = listing(d, open, Some(drawer(d, existing, &posted, &errors)));
    if !errors.is_empty() {
        return answer.with_notice(Tone::Danger, REFUSED);
    }
    answer.with_commit(vec![CommitOp {
        config: CONFIG.into(),
        section: match existing {
            Some(s) => s.name.clone(),
            None => section_name(&posted),
        },
        section_type: if existing.is_some() {
            String::new()
        } else {
            "service".into()
        },
        delete: false,
        values: Value::Object(writes(existing, &posted)),
    }])
}

fn blank(d: &Ddns) -> Service {
    Service {
        enabled: true,
        provider: PROVIDERS[0].id.into(),
        network: if d.networks.iter().any(|n| n == "wan") {
            "wan".into()
        } else {
            String::new()
        },
        ..Default::default()
    }
}

fn posted(existing: Option<&Service>, f: &Form) -> Service {
    let trimmed = |key: &str| f.get(key).trim().to_string();
    Service {
        name: existing.map(|s| s.name.clone()).unwrap_or_default(),
        enabled: !f.get("enabled").is_empty(),
        provider: f.get("provider"),
        hostname: trimmed("lookup_host").to_lowercase(),
        zone: trimmed("zone").to_lowercase(),
        domain: trimmed("domain"),
        username: trimmed("username"),
        password: f.get("password"),
        saved_password: existing.is_some_and(|s| s.saved_password),
        url: trimmed("update_url"),
        v6: f.get("use_ipv6") == "1",
        // Options the drawer does not show carry over from the section.
        source: match (existing, f.get("ip_source")) {
            (Some(s), _) if !s.edits_source() => s.source.clone(),
            (Some(s), chosen) if chosen.is_empty() => s.source.clone(),
            (_, chosen) => chosen,
        },
        ip_url: match f.get("ip_source") == WEB {
            true => trimmed("ip_url"),
            false => existing.map(|s| s.ip_url.clone()).unwrap_or_default(),
        },
        network: match existing {
            Some(s) if !s.edits_source() => s.network.clone(),
            _ => f.get("ip_network"),
        },
        interface: existing.map(|s| s.interface.clone()).unwrap_or_default(),
        timing: TIMING
            .iter()
            .map(|(key, _)| (key.to_string(), trimmed(key)))
            .filter(|(_, value)| !value.is_empty())
            .collect(),
        values: Map::new(),
        live: None,
    }
}

/// section_name is a new section's name, from its hostname and family: the
/// updater names its files after it, and a hotplug event starts it by it.
fn section_name(s: &Service) -> String {
    let base: String = s
        .hostname
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if s.v6 {
        format!("{base}_v6")
    } else {
        base
    }
}

fn validate(d: &Ddns, existing: Option<&Service>, s: &Service) -> Errors {
    let mut e = Errors::default();
    let custom = s.provider == CUSTOM;
    // A provider outside the list stays only on the name that already has it.
    let kept = s.elsewhere() && existing.is_some_and(|x| x.provider == s.provider);
    e.check(
        "provider",
        custom || kept || provider(&s.provider).is_some(),
        "Choose a provider.",
    );
    if kept {
        e.check(
            "domain",
            !s.domain.is_empty(),
            "Enter the domain as the provider expects it.",
        );
    }
    e.check(
        "lookup_host",
        !s.hostname.is_empty(),
        "Enter the hostname to keep up to date.",
    );
    if existing.is_none() && !s.hostname.is_empty() {
        e.check(
            "lookup_host",
            d.service(&section_name(s)).is_none(),
            "This hostname is already kept up to date over this address family.",
        );
    }
    match s.provider.as_str() {
        CLOUDFLARE => {
            e.check(
                "zone",
                !s.zone.is_empty(),
                "Enter the zone the hostname is in.",
            );
            e.check(
                "lookup_host",
                s.zone.is_empty()
                    || s.hostname == s.zone
                    || s.hostname.ends_with(&format!(".{}", s.zone)),
                "The hostname has to be in the zone.",
            );
        }
        DUCKDNS => e.check(
            "lookup_host",
            s.hostname.is_empty() || s.hostname.ends_with(DUCKDNS_SUFFIX),
            "A DuckDNS hostname ends in .duckdns.org.",
        ),
        CUSTOM => e.check(
            "update_url",
            (s.url.starts_with("https://") || s.url.starts_with("http://"))
                && s.url.contains("[IP]"),
            "Enter the provider’s update URL, with [IP] where the address goes.",
        ),
        _ => {}
    }
    if let Some(User::Typed(_)) = provider(&s.provider).map(|p| &p.user) {
        e.check("username", !s.username.is_empty(), "Enter the username.");
    }
    e.check(
        "password",
        custom || s.saved_password || !s.password.is_empty(),
        "Enter the provider’s secret.",
    );
    e.check(
        "ip_network",
        !s.edits_source() || d.networks.contains(&s.network),
        "Choose the network.",
    );
    e.check(
        "ip_url",
        s.source != WEB
            || s.ip_url.is_empty()
            || s.ip_url.starts_with("https://")
            || s.ip_url.starts_with("http://"),
        "Enter an http:// or https:// address, or leave it empty.",
    );
    for (key, _) in TIMING {
        let value = s.timing.get(key).map(String::as_str).unwrap_or("");
        let (field, ok) = match key.strip_suffix("_unit") {
            Some(base) => (
                format!("{base}_interval"),
                value.is_empty() || UNITS.contains(&value),
            ),
            None => (
                key.to_string(),
                value.is_empty() || value.parse::<u32>().is_ok(),
            ),
        };
        e.check(
            &field,
            ok,
            "Enter a whole number of seconds, minutes, hours or days.",
        );
    }
    e
}

/// writes is the section as ddns-scripts reads it for this provider. Options
/// another provider would use are cleared, and an empty password keeps the
/// saved one. What the drawer does not show stays as the section has it: an
/// address source set by hand, an `interface` apart from the network, and a
/// Cloudflare sign-in by e-mail rather than "Bearer".
fn writes(existing: Option<&Service>, s: &Service) -> Map<String, Value> {
    let p = provider(&s.provider);
    let domain = match s.provider.as_str() {
        CLOUDFLARE => {
            let host = s
                .hostname
                .strip_suffix(&s.zone)
                .unwrap_or("")
                .trim_end_matches('.');
            format!("{host}@{}", s.zone)
        }
        DUCKDNS => s.hostname.trim_end_matches(DUCKDNS_SUFFIX).into(),
        _ if s.elsewhere() => s.domain.clone(),
        _ => s.hostname.clone(),
    };
    let signed_in = existing.is_some_and(|x| x.provider == s.provider && !x.username.is_empty());
    // None leaves `username` as the section has it.
    let username = match p.map(|p| &p.user) {
        Some(User::None) => Some(None),
        Some(User::Fixed(_)) if signed_in => None,
        Some(User::Fixed(word)) => Some(Some(word.to_string())),
        Some(User::Hostname) => Some(Some(s.hostname.clone())),
        Some(User::Typed(_)) | None => Some(Some(s.username.clone()).filter(|u| !u.is_empty())),
    };
    let text = |v: Option<String>| v.map(Value::String).unwrap_or(Value::Null);
    let flag = |on: bool| json!(if on { "1" } else { "0" });
    let mut out = Map::new();
    out.insert("enabled".into(), flag(s.enabled));
    // A provider outside the list is named as the section names it, and what
    // its drawer does not show — an update URL, HTTPS — stays as it is.
    let elsewhere = s.elsewhere();
    let named = p.map(|p| p.id.to_string());
    out.insert(
        "service_name".into(),
        text(named.or_else(|| elsewhere.then(|| s.provider.clone()))),
    );
    if !elsewhere {
        out.insert(
            "update_url".into(),
            text(p.is_none().then(|| s.url.clone())),
        );
    }
    out.insert("domain".into(), json!(domain));
    out.insert("lookup_host".into(), json!(s.hostname));
    if let Some(username) = username {
        out.insert("username".into(), text(username));
    }
    if !s.password.is_empty() {
        out.insert("password".into(), json!(s.password));
    }
    out.insert("use_ipv6".into(), flag(s.v6));
    if !elsewhere {
        out.insert(
            "use_https".into(),
            flag(p.is_some() || s.url.starts_with("https://")),
        );
    }
    // A timing option is written when it differs from what the section reads
    // as now, so a save that changed none writes none, and a default is never
    // spelled out for its own sake.
    for (key, default) in TIMING {
        let saved = existing.and_then(|x| x.timing.get(key)).map(String::as_str);
        let posted = s.timing.get(key).map(String::as_str).unwrap_or(default);
        if posted != saved.unwrap_or(default) {
            out.insert(key.into(), json!(posted));
        }
    }
    if !s.edits_source() {
        return out;
    }
    // The source is written when it changes; a section that never named one
    // reads as `network`. Each source's own option is written under it, and
    // the other's is left for a switch back.
    let was = existing.map(|x| if x.by_network() { NETWORK } else { WEB });
    let now = if s.by_network() { NETWORK } else { WEB };
    if was != Some(now) {
        out.insert("ip_source".into(), json!(now));
    }
    match s.by_network() {
        true => out.insert("ip_network".into(), json!(s.network)),
        false => out.insert(
            "ip_url".into(),
            text(Some(s.ip_url.clone()).filter(|u| !u.is_empty())),
        ),
    };
    if existing.is_none_or(|x| x.interface.is_empty() || x.interface == x.network) {
        out.insert("interface".into(), json!(s.network));
    }
    out
}

fn listing(d: &Ddns, open: &str, drawer: Option<RowDrawer>) -> Envelope {
    let mut drawer = drawer;
    let blank = if open == NEW { drawer.take() } else { None };
    let rows = d
        .services
        .iter()
        .map(|s| row(d, s, if s.name == open { drawer.take() } else { None }))
        .collect();
    Envelope::page(
        HEADING,
        Widget::Table(Table {
            dense: true,
            columns: columns(),
            rows,
            empty_text: EMPTY.into(),
            ..Default::default()
        }),
    )
    .with_width("wide")
    .with_tone("neutral")
    .with_act(HeadingAct {
        label: "Add hostname".into(),
        href: href(NEW),
        opens_panel: true,
        drawer: blank,
        ..Default::default()
    })
}

fn columns() -> Vec<TableColumn> {
    [
        ("Hostname", "name", ColumnWidth::Long),
        ("Provider", "text", ColumnWidth::Name),
        ("Network", "entity", ColumnWidth::Word),
        ("Points at", "mono", ColumnWidth::Address),
        ("State", "status", ColumnWidth::Word),
        ("Since last update", "runtime", ColumnWidth::Grow),
        ("", "actions", ColumnWidth::Short),
    ]
    .into_iter()
    .map(|(label, kind, width)| TableColumn {
        label: label.into(),
        kind: kind.into(),
        width,
    })
    .collect()
}

fn row(d: &Ddns, s: &Service, drawer: Option<RowDrawer>) -> TableRow {
    let door = href(&s.name);
    let (state, variant) = state(d, s);
    let live = s.live.clone().unwrap_or_default();
    let cell = |text: &str| TableCell {
        text: text.into(),
        ..Default::default()
    };
    TableRow {
        id: s.name.clone(),
        muted: !s.enabled,
        cells: vec![
            TableCell {
                text: s.hostname.clone(),
                href: door.clone(),
                ..Default::default()
            },
            cell(&format!(
                "{} · {}",
                s.provider_label(),
                if s.v6 { "IPv6" } else { "IPv4" }
            )),
            TableCell {
                chips: vec![TableChip {
                    icon: "network".into(),
                    label: s.network.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            match live
                .address
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(", ")
            {
                a if a.is_empty() => TableCell {
                    text: DASH.into(),
                    muted: true,
                    ..Default::default()
                },
                a => TableCell {
                    text: a,
                    emphasis: true,
                    ..Default::default()
                },
            },
            TableCell {
                text: state.into(),
                variant: variant.into(),
                ..Default::default()
            },
            match live.updated {
                Some(secs) => cell(&since(secs)),
                None => TableCell {
                    text: DASH.into(),
                    muted: true,
                    ..Default::default()
                },
            },
            TableCell {
                actions: s
                    .enabled
                    .then(|| TableRowAct {
                        icon: "refresh-cw".into(),
                        title: "Update now".into(),
                        name: UPDATE.into(),
                        value: s.name.clone(),
                        ..Default::default()
                    })
                    .into_iter()
                    .chain([TableRowAct {
                        icon: "square-pen".into(),
                        title: "Edit".into(),
                        href: door.clone(),
                        ..Default::default()
                    }])
                    .collect(),
                ..Default::default()
            },
        ],
        drawer,
        panel: door,
        ..Default::default()
    }
}

/// state is a service's state in a word and its tone: whether the name points
/// at the router, which is what the page is for. The name's address is what the
/// updater last resolved it to; the router's is its network's now. A running
/// updater is not a working one: ddns-scripts retries a refused update forever.
///
/// A private address under `network` is one no provider takes: the router sits
/// behind another. Asking the internet, it cannot see what the internet said,
/// so it claims no more than that the updater runs. An address read some other
/// way, set by hand, gets no verdict either.
fn state(d: &Ddns, s: &Service) -> (&'static str, &'static str) {
    let live = match (s.enabled, d.known, &s.live) {
        (false, _, _) => return ("off", ""),
        (true, false, _) => return ("unknown", ""),
        (true, true, Some(l)) if l.running => l,
        (true, true, _) => return ("not running", "danger"),
    };
    let Some(current) = d.current(s).filter(|_| s.edits_source()) else {
        return ("running", "");
    };
    // The updater writes every address the name resolves to, one a line.
    let named: Vec<IpAddr> = live
        .address
        .split_whitespace()
        .filter_map(|a| a.parse().ok())
        .collect();
    match (private(&current), s.by_network()) {
        (true, true) => ("private address", "warning"),
        (true, false) => ("running", ""),
        _ if named.is_empty() => ("not resolving", "warning"),
        _ if named.contains(&current) => ("points here", "success"),
        _ => ("points elsewhere", "warning"),
    }
}

/// private is an address ddns-scripts refuses to send: IPv4's private, shared
/// (carrier NAT), loopback and link-local ranges, and IPv6 outside global
/// unicast.
fn private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => v6.segments()[0] & 0xe000 != 0x2000,
    }
}

/// since is a span in its largest unit, at least a minute. The units read
/// the same in Slovenian, and a span carries no words a catalog would need.
fn since(secs: u64) -> String {
    match (secs / 86400, secs / 3600, secs / 60) {
        (0, 0, m) => format!("{} min", m.max(1)),
        (0, h, _) => format!("{h} h"),
        (days, _, _) => format!("{days} d"),
    }
}

fn drawer(d: &Ddns, existing: Option<&Service>, s: &Service, e: &Errors) -> RowDrawer {
    // A provider set by hand outside the list leads it, under its own name, so
    // the drawer opens on what the section says rather than on the first choice.
    let kept = s
        .elsewhere()
        .then(|| SelectOption::new(&s.provider, &s.provider));
    let providers = kept
        .into_iter()
        .chain(PROVIDERS.iter().map(|p| SelectOption::new(p.id, p.label)))
        .chain([SelectOption::new(CUSTOM, "Another provider (update URL)")])
        .collect();
    let mut fields = vec![
        Widget::select(
            "provider",
            "Provider",
            &s.provider,
            providers,
            e.get("provider"),
        )
        .writes("service_name"),
        field(
            "lookup_host",
            "Hostname",
            &s.hostname,
            "fqdn",
            "The full hostname the provider gave you, such as myhome.duckdns.org.",
            e,
        )
        .writes("lookup_host"),
    ];
    for p in &PROVIDERS {
        let mut children = Vec::new();
        if p.id == CLOUDFLARE {
            children.push(field(
                "zone",
                "Zone",
                &s.zone,
                "fqdn",
                "The domain the hostname is in, as Cloudflare lists it.",
                e,
            ));
        }
        if let User::Typed(label) = p.user {
            children.push(field("username", label, &s.username, "", "", e));
        }
        children.push(secret(p.secret, s, e));
        fields.push(when(p.id, s, children));
    }
    if s.elsewhere() {
        fields.push(when(
            &s.provider,
            s,
            vec![
                field(
                    "domain",
                    "Domain",
                    &s.domain,
                    "",
                    "The name as this provider expects it, which can differ from the hostname above.",
                    e,
                )
                .writes("domain"),
                field("username", "Username", &s.username, "", "", e),
                secret("Password", s, e),
            ],
        ));
    }
    fields.push(when(
        CUSTOM,
        s,
        vec![
            field(
                "update_url",
                "Update URL",
                &s.url,
                "",
                "The address the router calls to update the name. [IP], [DOMAIN], [USERNAME] and [PASSWORD] are filled in.",
                e,
            ),
            field("username", "Username", &s.username, "", "", e),
            secret("Password", s, e),
        ],
    ));
    fields.push(
        Widget::select(
            "use_ipv6",
            "Address",
            if s.v6 { "1" } else { "0" },
            vec![
                SelectOption::new("0", "IPv4"),
                SelectOption::new("1", "IPv6"),
            ],
            "",
        )
        .writes("use_ipv6"),
    );
    fields.extend(source(d, s, e));
    fields.push(Widget::switch_keyed(
        "enabled",
        "Keep it up to date",
        "enabled",
        "Checks the address on the timing below and updates the name when it changes.",
        s.enabled,
    ));
    fields.push(timing(s, e));
    // The section as saving would leave it, the password standing masked.
    let mut preview = existing.map(|x| x.values.clone()).unwrap_or_default();
    for (key, value) in writes(existing, s) {
        match value {
            Value::Null => preview.remove(&key),
            value => preview.insert(key, value),
        };
    }
    if s.saved_password || preview.contains_key("password") {
        preview.insert("password".into(), json!(MASK));
    }
    fields.push(Widget::config_preview(
        "/etc/config/ddns",
        &uci_text(
            "service",
            existing.map(|x| x.name.as_str()).unwrap_or(""),
            &preview,
        ),
    ));
    let mut children = vec![Widget::form("Save", fields)];
    if existing.is_some() {
        children.push(Widget::Form {
            style: String::new(),
            submit: String::new(),
            error: String::new(),
            note: String::new(),
            target: String::new(),
            fields: vec![
                Widget::hidden(DELETE, "1"),
                Widget::Confirm {
                    trigger: "Remove".into(),
                    title: String::new(),
                    message: "The router stops updating this name once you apply. The name keeps the last address it was given.".into(),
                    confirm: "Remove".into(),
                    cancel: String::new(),
                },
            ],
        });
    }
    RowDrawer {
        title: match existing {
            Some(x) => x.hostname.clone(),
            None => "New hostname".into(),
        },
        closed: "/plugins/ddns/".into(),
        open: true,
        children,
        ..Default::default()
    }
}

/// source is where the address comes from, and the network the name follows.
/// Choosing the source draws the drawer again, so the network names the option
/// it writes: `ip_network` read for the address, or `interface` whose coming
/// up starts a check.
fn source(d: &Ddns, s: &Service, e: &Errors) -> Vec<Widget> {
    if !s.edits_source() {
        return vec![Widget::text(
            "The address is read some other way, set outside this page (`ip_source`). Saving keeps it.",
        )];
    }
    let mut fields = vec![helped(
        Widget::select(
            "ip_source",
            "Address from",
            if s.by_network() { NETWORK } else { WEB },
            vec![
                SelectOption::new(NETWORK, "This router’s connection"),
                SelectOption::new(WEB, "Ask the internet"),
            ],
            "",
        )
        .writes("ip_source")
        .reshapes(),
        "Behind another router, this router’s own address is a private one that providers refuse. Ask the internet instead.",
    )];
    if !s.by_network() {
        // Whole sentences, one per family, so each has its catalog key.
        let help = match s.v6 {
            true => {
                "Where the router asks for its public address. Empty asks checkipv6.dyndns.com."
            }
            false => "Where the router asks for its public address. Empty asks checkip.dyndns.com.",
        };
        fields.push(field("ip_url", "Checked at", &s.ip_url, "", help, e).writes("ip_url"));
    }
    let (key, help) = match s.by_network() {
        true => (
            "ip_network",
            "The connection whose address the name points at.",
        ),
        false => (
            "interface",
            "The name is checked again whenever this connection comes up.",
        ),
    };
    fields.push(helped(
        Widget::select(
            "ip_network",
            "Network",
            &s.network,
            std::iter::once(SelectOption::new("", "Choose a network"))
                .chain(d.networks.iter().map(|n| SelectOption::new(n, n)))
                .collect(),
            e.get("ip_network"),
        )
        .writes(key),
        help,
    ));
    fields
}

/// UNITS are what ddns-scripts counts an interval in.
const UNITS: [&str; 4] = ["seconds", "minutes", "hours", "days"];

/// timing is how often the updater checks, sends anyway and retries, and when
/// it gives up: each a number and its unit, drawn as ddns-scripts reads them.
fn timing(s: &Service, e: &Errors) -> Widget {
    let value = |key: &str| {
        let default = TIMING
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, d)| *d)
            .unwrap_or("");
        s.timing.get(key).cloned().unwrap_or_else(|| default.into())
    };
    let units = || UNITS.iter().map(|u| SelectOption::new(u, u)).collect();
    // A number and its unit side by side, each labelled: the shell fuses a
    // group under one label only around a joining word, and none reads
    // between "10" and "minutes".
    let pair = |base: &str, label: &str, help: &str| {
        let (interval, unit) = (format!("{base}_interval"), format!("{base}_unit"));
        Widget::form_grid(
            2,
            vec![
                field(&interval, label, &value(&interval), "", help, e).writes(&interval),
                Widget::select(&unit, "Unit", &value(&unit), units(), "").writes(&unit),
            ],
        )
    };
    Widget::section(
        "Timing",
        "",
        vec![
            pair(
                "check",
                "Check every",
                "How often the router compares its address with the name’s. Under 5 minutes counts as 5.",
            ),
            pair(
                "force",
                "Send anyway every",
                "An update goes out this often even when nothing changed, so the provider keeps the name. 0 sends once and stops.",
            ),
            pair("retry", "Retry after", "How long to wait after a failed update."),
            field(
                "retry_max_count",
                "Tries before giving up",
                &value("retry_max_count"),
                "",
                "The updater stops after this many failed tries in a row. 0 keeps trying.",
                e,
            )
            .writes("retry_max_count"),
        ],
    )
    .ruled()
}

fn helped(mut w: Widget, text: &str) -> Widget {
    if let Widget::Field(Field { help, .. }) = &mut w {
        *help = text.into();
    }
    w
}

fn when(value: &str, s: &Service, children: Vec<Widget>) -> Widget {
    Widget::When {
        name: "provider".into(),
        value: value.into(),
        active: s.provider == value,
        children,
    }
}

fn field(name: &str, label: &str, value: &str, datatype: &str, help: &str, e: &Errors) -> Widget {
    Widget::Field(Field {
        name: name.into(),
        label: label.into(),
        kind: "text".into(),
        value: value.into(),
        datatype: datatype.into(),
        help: help.into(),
        error: e.get(name).into(),
        ..Default::default()
    })
}

/// secret is the provider's password or token. It is a shared secret, so it is
/// drawn with its eye to check it by, and what
/// was just typed stands in it when the drawer comes back refused; a saved one
/// is never put back.
fn secret(label: &str, s: &Service, e: &Errors) -> Widget {
    Widget::Field(Field {
        name: "password".into(),
        label: label.into(),
        kind: "password".into(),
        style: "reveal".into(),
        value: s.password.clone(),
        key: "password".into(),
        help: if s.saved_password {
            KEEP_HELP.into()
        } else {
            String::new()
        },
        error: e.get("password").into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Addresses;
    use verso_plugin::{Request, Snapshot, Ubus};

    fn request(snapshot: Value, state: Value) -> Request {
        Request {
            path: "/".into(),
            query: Form::default(),
            snapshot: Snapshot::from_value(snapshot),
            ubus: Ubus::from_value(state),
        }
    }

    fn router() -> Ddns {
        Ddns::read(&request(
            json!({
                "network": {
                    "loopback": {".type": "interface", ".name": "loopback", ".index": 0},
                    "wan": {".type": "interface", ".name": "wan", ".index": 1},
                    "wan6": {".type": "interface", ".name": "wan6", ".index": 2}
                },
                "ddns": {
                    "global": {".type": "ddns", ".name": "global", ".index": 0},
                    "myddns_ipv4": {".type": "service", ".name": "myddns_ipv4", ".index": 1,
                        "service_name": "dyndns.org", "domain": "yourhost.example.com"},
                    "home": {".type": "service", ".name": "home", ".index": 2, "enabled": "1",
                        "service_name": "cloudflare.com-v4", "domain": "home@example.com",
                        "username": "Bearer", "password": "s3cret", "ip_network": "wan"}
                }
            }),
            json!({"ddnsState": {"services": {"home": {"address": "203.0.113.7", "running": true, "updated": 7200}}}}),
        ))
    }

    fn body(e: &Envelope) -> Value {
        serde_json::to_value(e).unwrap()
    }

    #[test]
    fn a_service_reads_out_on_one_row_and_samples_stay_out() {
        let d = router();
        let b = body(&page(&d, ""));
        let rows = b["widget"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{b}");
        let cells = &rows[0]["cells"];
        assert_eq!(cells[0]["text"], "home.example.com");
        assert_eq!(cells[1]["text"], "Cloudflare · IPv4");
        assert_eq!(cells[2]["chips"][0]["label"], "wan");
        assert_eq!(cells[3]["text"], "203.0.113.7");
        assert_eq!(cells[4]["text"], "running");
        // A span, not a sentence: the units read the same in every catalog,
        // and the column's heading says what it is the span since.
        assert_eq!(b["widget"]["columns"][5]["label"], "Since last update");
        assert_eq!(b["widget"]["columns"][5]["kind"], "runtime");
        assert_eq!(cells[5]["text"], "2 h");
        assert!(!b.to_string().contains("s3cret"));
    }

    #[test]
    fn spans_read_in_their_largest_unit() {
        assert_eq!(since(0), "1 min");
        assert_eq!(since(59 * 60), "59 min");
        assert_eq!(since(3 * 3600 + 60), "3 h");
        assert_eq!(since(2 * 86400), "2 d");
    }

    #[test]
    fn a_new_cloudflare_name_writes_what_ddns_scripts_reads() {
        let d = router();
        let form = Form::parse(
            "provider=cloudflare.com-v4&lookup_host=nas.lab.example.com&zone=example.com&password=tok&use_ipv6=1&ip_network=wan6&enabled=1",
        );
        let b = body(&post(&d, NEW, &form));
        let op = &b["commit"][0];
        assert_eq!(op["section"], "nas_lab_example_com_v6");
        assert_eq!(op["type"], "service");
        assert_eq!(op["values"]["domain"], "nas.lab@example.com");
        assert_eq!(op["values"]["lookup_host"], "nas.lab.example.com");
        assert_eq!(op["values"]["username"], "Bearer");
        assert_eq!(op["values"]["password"], "tok");
        assert_eq!(op["values"]["use_ipv6"], "1");
        assert_eq!(op["values"]["use_https"], "1");
        assert_eq!(op["values"]["update_url"], Value::Null);
    }

    #[test]
    fn duckdns_and_desec_spell_their_own_names() {
        let d = router();
        let duck = writes(
            None,
            &posted(
                None,
                &Form::parse("provider=duckdns.org&lookup_host=myhome.duckdns.org&password=t"),
            ),
        );
        assert_eq!(duck["domain"], "myhome");
        assert_eq!(duck["username"], Value::Null);
        let desec = writes(
            None,
            &posted(
                None,
                &Form::parse("provider=desec.io&lookup_host=me.dedyn.io&password=t"),
            ),
        );
        assert_eq!(desec["username"], "me.dedyn.io");
        assert!(validate(
            &d,
            None,
            &posted(
                None,
                &Form::parse(
                    "provider=duckdns.org&lookup_host=x.example.com&password=t&ip_network=wan"
                )
            )
        )
        .get("lookup_host")
        .contains("duckdns"));
    }

    #[test]
    fn an_edit_keeps_the_saved_secret_and_a_refusal_saves_nothing() {
        let d = router();
        let kept = body(&post(&d, "home", &Form::parse(
            "provider=cloudflare.com-v4&lookup_host=home.example.com&zone=example.com&ip_network=wan&enabled=1",
        )));
        assert_eq!(kept["commit"][0]["section"], "home");
        assert!(kept["commit"][0]["values"].get("password").is_none());
        let refused = body(&post(
            &d,
            NEW,
            &Form::parse("provider=no-ip.com&lookup_host=a.ddns.net&ip_network=wan"),
        ));
        assert!(refused.get("commit").is_none());
        assert_eq!(refused["notice"]["text"], REFUSED);
    }

    #[test]
    fn a_custom_url_needs_the_ip_placeholder_and_follows_its_scheme() {
        let d = router();
        let bad = posted(
            None,
            &Form::parse(
                "provider=custom&lookup_host=a.example.org&update_url=https://x/&ip_network=wan",
            ),
        );
        assert!(!validate(&d, None, &bad).get("update_url").is_empty());
        let plain = writes(
            None,
            &posted(
                None,
                &Form::parse(
                    "provider=custom&lookup_host=a.example.org&update_url=http://x/?ip=[IP]",
                ),
            ),
        );
        assert_eq!(plain["service_name"], Value::Null);
        assert_eq!(plain["use_https"], "0");
    }

    #[test]
    fn a_confirmed_remove_deletes_the_section() {
        let b = body(&post(&router(), "home", &Form::parse("delete=1")));
        assert_eq!(b["commit"][0]["delete"], true);
        assert_eq!(b["commit"][0]["section"], "home");
    }

    fn saved(source: &str, network: &str, interface: &str) -> Service {
        Service {
            name: "home".into(),
            enabled: true,
            provider: "dynv6.com".into(),
            hostname: "home.dynv6.net".into(),
            saved_password: true,
            source: source.into(),
            network: network.into(),
            interface: interface.into(),
            ..Default::default()
        }
    }

    fn resaved(existing: &Service, form: &str) -> Map<String, Value> {
        writes(Some(existing), &posted(Some(existing), &Form::parse(form)))
    }

    #[test]
    fn a_save_leaves_an_address_source_set_by_hand() {
        let web = saved("script", "wan", "wan");
        let out = resaved(
            &web,
            "provider=dynv6.com&lookup_host=home.dynv6.net&enabled=1&ip_source=web&ip_network=lan",
        );
        for key in ["ip_source", "ip_network", "interface"] {
            assert!(!out.contains_key(key), "{key} written: {out:?}");
        }
        let d = router();
        let drawer =
            serde_json::to_string(&drawer(&d, Some(&web), &web, &Errors::default())).unwrap();
        assert!(!drawer.contains("\"ip_network\""), "{drawer}");
        assert!(validate(
            &d,
            Some(&web),
            &posted(
                Some(&web),
                &Form::parse("provider=dynv6.com&lookup_host=home.dynv6.net")
            )
        )
        .get("ip_network")
        .is_empty());
    }

    #[test]
    fn a_new_name_can_ask_the_internet_for_its_address() {
        let out = writes(
            None,
            &posted(
                None,
                &Form::parse(
                    "provider=dynv6.com&lookup_host=home.dynv6.net&ip_source=web&ip_network=wan",
                ),
            ),
        );
        assert_eq!(out["ip_source"], "web");
        assert_eq!(out["interface"], "wan");
        assert_eq!(out["ip_url"], Value::Null);
        assert!(!out.contains_key("ip_network"), "{out:?}");
        let checker = writes(
            None,
            &posted(
                None,
                &Form::parse("provider=dynv6.com&lookup_host=h.dynv6.net&ip_source=web&ip_url=https://api.ipify.org&ip_network=wan"),
            ),
        );
        assert_eq!(checker["ip_url"], "https://api.ipify.org");
    }

    #[test]
    fn switching_the_source_leaves_the_other_sources_options() {
        let mut net = saved("", "wan", "wan");
        net.values.insert("ip_network".into(), json!("wan"));
        let to_web = resaved(
            &net,
            "provider=dynv6.com&lookup_host=home.dynv6.net&ip_source=web&ip_network=wan",
        );
        assert_eq!(to_web["ip_source"], "web");
        assert!(!to_web.contains_key("ip_network"), "{to_web:?}");
        let mut web = saved("web", "wan", "wan");
        web.ip_url = "https://api.ipify.org".into();
        let to_net = resaved(
            &web,
            "provider=dynv6.com&lookup_host=home.dynv6.net&ip_source=network&ip_network=wan",
        );
        assert_eq!(to_net["ip_source"], "network");
        assert_eq!(to_net["ip_network"], "wan");
        assert!(!to_net.contains_key("ip_url"), "{to_net:?}");
        let kept = resaved(
            &net,
            "provider=dynv6.com&lookup_host=home.dynv6.net&ip_network=wan",
        );
        assert!(!kept.contains_key("ip_source"), "{kept:?}");
    }

    #[test]
    fn choosing_a_source_redraws_the_drawer_with_its_own_fields() {
        let d = router();
        let form = Form::parse(
            "_action=reshape&provider=dynv6.com&lookup_host=a.dynv6.net&ip_source=web&ip_network=wan",
        );
        let b = body(&post(&d, NEW, &form));
        assert!(b.get("commit").is_none());
        assert!(b.get("notice").is_none());
        let drawer = b["act"]["drawer"].to_string();
        assert!(drawer.contains("\"ip_url\""), "{drawer}");
        assert!(drawer.contains("\"key\":\"interface\""), "{drawer}");
        let bad = posted(None, &Form::parse("provider=dynv6.com&lookup_host=a.dynv6.net&ip_source=web&ip_url=ftp://x&ip_network=wan"));
        assert!(!validate(&d, None, &bad).get("ip_url").is_empty());
    }

    #[test]
    fn a_save_moves_the_trigger_only_where_it_followed_the_network() {
        let together = resaved(
            &saved("", "wan", "wan"),
            "provider=dynv6.com&lookup_host=home.dynv6.net&ip_network=wan6",
        );
        assert_eq!(together["ip_network"], "wan6");
        assert_eq!(together["interface"], "wan6");
        assert!(!together.contains_key("ip_source"));
        let apart = resaved(
            &saved("network", "wan", "lan"),
            "provider=dynv6.com&lookup_host=home.dynv6.net&ip_network=wan6",
        );
        assert_eq!(apart["ip_network"], "wan6");
        assert!(!apart.contains_key("interface"));
    }

    #[test]
    fn a_cloudflare_email_sign_in_survives_a_save() {
        let mut email = saved("", "wan", "wan");
        email.provider = CLOUDFLARE.into();
        email.username = "me@example.com".into();
        let out = resaved(&email, "provider=cloudflare.com-v4&lookup_host=home.example.com&zone=example.com&ip_network=wan");
        assert!(!out.contains_key("username"), "{out:?}");
        let switched = resaved(&saved("", "wan", "wan"), "provider=cloudflare.com-v4&lookup_host=home.example.com&zone=example.com&ip_network=wan");
        assert_eq!(switched["username"], "Bearer");
    }

    fn wan(d: &mut Ddns, address: &str) {
        d.addresses.insert(
            "wan".into(),
            Addresses {
                v4: address.parse().ok(),
                v6: None,
            },
        );
    }

    #[test]
    fn a_name_says_whether_it_points_at_the_router() {
        let mut d = router();
        let home = |d: &Ddns| state(d, &d.services[0]);
        // Nothing to compare with: the updater runs, and that is all that is known.
        assert_eq!(home(&d), ("running", ""));
        wan(&mut d, "203.0.113.7");
        assert_eq!(home(&d), ("points here", "success"));
        wan(&mut d, "198.51.100.4");
        assert_eq!(home(&d), ("points elsewhere", "warning"));
        d.services[0].live.as_mut().unwrap().address = String::new();
        assert_eq!(home(&d), ("not resolving", "warning"));
        // A name with several addresses points here when one of them is ours.
        d.services[0].live.as_mut().unwrap().address = "203.0.113.9\n198.51.100.4".into();
        assert_eq!(home(&d), ("points here", "success"));
        let b = body(&page(&d, ""));
        assert_eq!(
            b["widget"]["rows"][0]["cells"][3]["text"],
            "203.0.113.9, 198.51.100.4"
        );
    }

    #[test]
    fn a_private_address_says_the_router_sits_behind_another() {
        let mut d = router();
        for private in ["192.168.1.20", "172.30.1.171", "10.0.0.2", "100.72.4.1"] {
            wan(&mut d, private);
            assert_eq!(
                state(&d, &d.services[0]),
                ("private address", "warning"),
                "{private}"
            );
        }
        // Asking the internet, the router cannot see what it was told: no verdict.
        d.services[0].source = WEB.into();
        assert_eq!(state(&d, &d.services[0]), ("running", ""));
        wan(&mut d, "203.0.113.7");
        assert_eq!(state(&d, &d.services[0]), ("points here", "success"));
    }

    #[test]
    fn the_router_reads_its_address_as_ddns_scripts_does() {
        let mut r = request(
            json!({}),
            json!({"networkState": {"interfaces": [
                {"interface": "wan", "ipv4-address": [{"address": "203.0.113.7", "mask": 24}]},
                {"interface": "wan6", "ipv6-address": [],
                    "ipv6-prefix-assignment": [{"address": "2001:db8:1::", "mask": 64,
                        "local-address": {"address": "2001:db8:1::1", "mask": 64}}]}
            ]}}),
        );
        r.query = Form::default();
        let d = Ddns::read(&r);
        let mut s = Service {
            network: "wan6".into(),
            v6: true,
            ..Default::default()
        };
        assert_eq!(
            d.current(&s),
            "2001:0db8:0001:0000:0000:0000:0000:0001".parse().ok()
        );
        s.network = "wan".into();
        s.v6 = false;
        assert_eq!(d.current(&s), "203.0.113.7".parse().ok());
    }

    #[test]
    fn a_switched_on_row_sends_its_address_now() {
        let d = router();
        let row = &body(&page(&d, ""))["widget"]["rows"][0]["cells"][6]["actions"][0];
        assert_eq!(row["icon"], "refresh-cw");
        assert_eq!(row["name"], UPDATE);
        assert_eq!(row["value"], "home");
        let b = body(&post(&d, "", &Form::parse("update=home")));
        assert_eq!(b["commands"][0]["name"], "ddns-update");
        assert_eq!(b["commands"][0]["args"]["section"], "home");
        assert!(b.get("commit").is_none());
        assert_eq!(b["notice"]["text"], SENDING);
        // Switched off, or no longer there, nothing is sent.
        let mut off = router();
        off.services[0].enabled = false;
        let none = body(&page(&off, ""))["widget"]["rows"][0]["cells"][6]["actions"].clone();
        assert!(none.as_array().unwrap().iter().all(|a| a["name"] != UPDATE));
        assert!(body(&post(&off, "", &Form::parse("update=home")))
            .get("commands")
            .is_none());
        assert!(body(&post(&d, "", &Form::parse("update=gone")))
            .get("commands")
            .is_none());
    }

    const NAME: &str = "provider=dynv6.com&lookup_host=home.dynv6.net&ip_network=wan";

    #[test]
    fn timing_shows_ddns_scripts_defaults_and_an_unchanged_save_writes_none() {
        let d = router();
        let s = saved("", "wan", "wan");
        let drawer = serde_json::to_string(&drawer(&d, Some(&s), &s, &Errors::default())).unwrap();
        for (name, value) in [
            ("check_interval", "10"),
            ("force_interval", "72"),
            ("retry_interval", "60"),
            ("retry_max_count", "0"),
        ] {
            assert!(
                drawer.contains(&format!("\"name\":\"{name}\",\"label\""))
                    && drawer.contains(&format!("\"value\":\"{value}\"")),
                "{name}={value} not drawn"
            );
        }
        let out = resaved(
            &s,
            &format!("{NAME}&check_interval=10&check_unit=minutes&force_interval=72&force_unit=hours&retry_interval=60&retry_unit=seconds&retry_max_count=0"),
        );
        for (key, _) in TIMING {
            assert!(!out.contains_key(key), "{key} written: {out:?}");
        }
    }

    #[test]
    fn timing_writes_only_what_changed_and_keeps_what_was_set() {
        let mut s = saved("", "wan", "wan");
        s.timing.insert("force_unit".into(), "days".into());
        s.timing.insert("force_interval".into(), "7".into());
        let out = resaved(
            &s,
            &format!("{NAME}&check_interval=30&check_unit=minutes&force_interval=7&force_unit=days&retry_interval=60&retry_unit=seconds&retry_max_count=5"),
        );
        assert_eq!(out["check_interval"], "30");
        assert_eq!(out["retry_max_count"], "5");
        for key in [
            "check_unit",
            "force_interval",
            "force_unit",
            "retry_interval",
            "retry_unit",
        ] {
            assert!(!out.contains_key(key), "{key} written: {out:?}");
        }
    }

    #[test]
    fn timing_takes_whole_numbers_and_known_units() {
        let d = router();
        let s = saved("", "wan", "wan");
        for (bad, key) in [
            ("check_interval=ten&check_unit=minutes", "check_interval"),
            ("check_interval=-1&check_unit=minutes", "check_interval"),
            ("check_interval=10&check_unit=weeks", "check_interval"),
            ("retry_max_count=x", "retry_max_count"),
        ] {
            let posted = posted(Some(&s), &Form::parse(&format!("{NAME}&{bad}")));
            assert!(
                !validate(&d, Some(&s), &posted).get(key).is_empty(),
                "{bad}"
            );
        }
    }

    fn he_net() -> Service {
        let mut s = saved("", "wan", "wan");
        s.provider = "he.net".into();
        s.hostname = "home.example.net".into();
        s.domain = "home.example.net".into();
        s.username = "home.example.net".into();
        s
    }

    #[test]
    fn a_provider_outside_the_list_keeps_its_own_option_and_fields() {
        let d = router();
        let s = he_net();
        let drawer = serde_json::to_value(drawer(&d, Some(&s), &s, &Errors::default())).unwrap();
        let text = drawer.to_string();
        assert!(
            text.contains(r#"{"label":"he.net","value":"he.net"}"#),
            "{text}"
        );
        // Its own fields are the ones shown, the domain among them.
        fn find<'a>(v: &'a Value, hit: &dyn Fn(&Value) -> bool) -> Option<&'a Value> {
            if hit(v) {
                return Some(v);
            }
            match v {
                Value::Array(a) => a.iter().find_map(|c| find(c, hit)),
                Value::Object(o) => o.values().find_map(|c| find(c, hit)),
                _ => None,
            }
        }
        let own = find(&drawer, &|v| v["type"] == "when" && v["value"] == "he.net").unwrap();
        assert_eq!(own["active"], true);
        assert!(own.to_string().contains(r#""name":"domain""#), "{own}");
    }

    #[test]
    fn a_provider_outside_the_list_saves_what_its_drawer_shows_and_nothing_else() {
        let s = he_net();
        let out = resaved(
            &s,
            "provider=he.net&lookup_host=home.example.net&domain=home.example.net&username=home.example.net&ip_network=wan",
        );
        assert_eq!(out["service_name"], "he.net");
        assert_eq!(out["domain"], "home.example.net");
        assert_eq!(out["username"], "home.example.net");
        for key in ["update_url", "use_https", "password"] {
            assert!(!out.contains_key(key), "{key} written: {out:?}");
        }
        let d = router();
        assert!(validate(&d, Some(&s), &posted(Some(&s), &Form::parse("provider=he.net&lookup_host=home.example.net&domain=home.example.net&ip_network=wan"))).is_empty());
        // Only the name that has it keeps it: a new one picks from the list.
        let fresh = posted(None, &Form::parse("provider=he.net&lookup_host=a.example.net&domain=a.example.net&password=x&ip_network=wan"));
        assert!(!validate(&d, None, &fresh).get("provider").is_empty());
    }

    #[test]
    fn a_refused_save_keeps_the_token_just_typed_and_never_a_saved_one() {
        let d = router();
        let refused = body(&post(
            &d,
            NEW,
            &Form::parse("provider=duckdns.org&lookup_host=myhome&password=tok&ip_network=wan"),
        ));
        assert_eq!(refused["notice"]["text"], REFUSED);
        let typed = refused.to_string();
        assert!(typed.contains(r#""style":"reveal""#), "{typed}");
        assert!(typed.contains(r#""value":"tok""#), "{typed}");
        // The saved Cloudflare token stays on the router.
        let opened = body(&page(&d, "home")).to_string();
        assert!(!opened.contains("s3cret"));
        assert!(!opened.contains(r#""value":"tok""#));
    }

    #[test]
    fn an_updater_that_gave_up_says_so() {
        let mut d = router();
        d.services[0].live = None;
        assert_eq!(state(&d, &d.services[0]), ("not running", "danger"));
        d.known = false;
        assert_eq!(state(&d, &d.services[0]), ("unknown", ""));
    }
}
