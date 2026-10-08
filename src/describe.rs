// SPDX-License-Identifier: GPL-2.0-only
// SPDX-FileCopyrightText: 2026 Mono Technologies Inc.

//! What this page's staged changes amount to, in the review: one line per
//! name, however many options its save wrote. The snapshot holds the staged
//! config, so an added or changed name reads by its hostname; a removed one is
//! gone from it and reads by its section.

use verso_plugin::{Change, Description, Snapshot};

use crate::model::{Service, CONFIG};

pub fn describe(changes: &[Change], snapshot: &Snapshot) -> Vec<Description> {
    let mut sections: Vec<(&str, Vec<usize>)> = Vec::new();
    for (i, c) in changes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.config == CONFIG)
    {
        match sections.iter_mut().find(|(s, _)| *s == c.section) {
            Some((_, covers)) => covers.push(i),
            None => sections.push((&c.section, vec![i])),
        }
    }
    sections
        .into_iter()
        .map(|(section, covers)| {
            let ops = covers.iter().map(|&i| &changes[i]);
            let service = snapshot
                .section(CONFIG, section)
                .map(|s| Service::of(&s, None));
            let plain = match service {
                None => format!("Removed {section}"),
                Some(s) if ops.clone().any(|c| c.op == "add-section") => format!(
                    "Added {} ({}, {})",
                    s.hostname,
                    s.provider_label(),
                    if s.v6 { "IPv6" } else { "IPv4" }
                ),
                Some(s) if ops.clone().all(|c| c.option == "enabled") => {
                    format!(
                        "Turned {} {}",
                        if s.enabled { "on" } else { "off" },
                        s.hostname
                    )
                }
                Some(s) => format!("Changed {}", s.hostname),
            };
            Description::new(plain, covers)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use verso_plugin::json;

    fn change(op: &str, section: &str, option: &str, value: &str) -> Change {
        serde_json::from_value(json!({
            "config": "ddns", "op": op, "section": section, "option": option, "value": value
        }))
        .unwrap()
    }

    fn staged() -> Snapshot {
        Snapshot::from_value(json!({"ddns": {
            "nas_v6": {".type": "service", ".name": "nas_v6", ".index": 0, "enabled": "1",
                "service_name": "cloudflare.com-v4", "domain": "nas@example.com",
                "lookup_host": "nas.example.com", "use_ipv6": "1"},
            "home": {".type": "service", ".name": "home", ".index": 1, "enabled": "0",
                "service_name": "duckdns.org", "lookup_host": "home.duckdns.org"}
        }}))
    }

    #[test]
    fn each_name_is_one_line_however_many_options_its_save_wrote() {
        let changes = vec![
            change("add-section", "nas_v6", "service", ""),
            change("set", "nas_v6", "domain", "nas@example.com"),
            change("set", "nas_v6", "password", "tok"),
            change("set", "home", "enabled", "0"),
            change("remove-section", "old", "", ""),
            change("set", "home", "check_interval", "5"),
        ];
        let lines = describe(&changes, &staged());
        let plain: Vec<&str> = lines.iter().map(|l| l.plain.as_str()).collect();
        assert_eq!(
            plain,
            [
                "Added nas.example.com (Cloudflare, IPv6)",
                "Changed home.duckdns.org",
                "Removed old"
            ]
        );
        assert_eq!(lines[0].covers, vec![0, 1, 2]);
        assert_eq!(lines[1].covers, vec![3, 5]);
    }

    #[test]
    fn a_switch_on_its_own_says_which_way() {
        let lines = describe(&[change("set", "home", "enabled", "0")], &staged());
        assert_eq!(lines[0].plain, "Turned off home.duckdns.org");
    }

    #[test]
    fn other_configs_keep_their_raw_lines() {
        let other: Change = serde_json::from_value(json!({
            "config": "network", "op": "set", "section": "wan", "option": "proto", "value": "dhcp"
        }))
        .unwrap();
        assert!(describe(&[other], &staged()).is_empty());
    }
}
