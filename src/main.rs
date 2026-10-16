// SPDX-License-Identifier: GPL-2.0-only
// SPDX-FileCopyrightText: 2026 Mono Technologies Inc.

//! The Verso Dynamic DNS plugin: names kept pointing at the router, over
//! ddns-scripts.
//!
//! Every request is answered from the reads the shell brokers with it
//! (ADR-007): the ddns and network configs, and the helper's `ddnsState`, which
//! reads what each updater last did from its run folder. The plugin reaches
//! nothing itself.

use verso_plugin::{serve_described, Envelope, Form, Request};

mod describe;
mod model;
mod page;

fn main() {
    serve_described("ddns", get, post, describe::describe);
}

// The plugin is one page, at its own address: a hostname opens beside it by
// query. Any path below it has nothing at it.
fn get(request: &Request) -> Envelope {
    if !request.path.trim_matches('/').is_empty() {
        return Envelope::not_found();
    }
    page::page(&model::Ddns::read(request), &request.query.get(page::OPEN))
}

fn post(request: &Request, form: &Form) -> Envelope {
    if !request.path.trim_matches('/').is_empty() {
        return Envelope::not_found();
    }
    page::post(
        &model::Ddns::read(request),
        &request.query.get(page::OPEN),
        form,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use verso_plugin::{Snapshot, Ubus, Value};

    fn at(path: &str) -> Request {
        Request {
            path: path.into(),
            query: Form::default(),
            snapshot: Snapshot::from_value(Value::Null),
            ubus: Ubus::from_value(Value::Null),
        }
    }

    /// The plugin is one page, at its own address; any other path has nothing
    /// at it and says so, rather than passing for that page.
    #[test]
    fn a_path_below_the_page_has_nothing_at_it() {
        for path in ["/nowhere", "/hosts/x"] {
            assert!(get(&at(path)).not_found, "{path}");
            assert!(post(&at(path), &Form::default()).not_found, "{path}");
        }
        for path in ["", "/"] {
            assert!(!get(&at(path)).not_found, "{path}");
        }
    }

    /// Every catalog key is English this plugin still says: rewording a string
    /// orphans its translation, and this is where that shows.
    #[test]
    fn the_slovenian_catalog_has_no_orphans() {
        let catalog: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(include_str!("../i18n/sl.json")).unwrap();
        // A string continued over lines (`\` at the end) reads as one.
        let source: String = [
            include_str!("page.rs"),
            include_str!("model.rs"),
            include_str!("../manifest.json"),
        ]
        .concat()
        .split("\\\n")
        .map(str::trim_start)
        .collect();
        for key in catalog.keys() {
            assert!(source.contains(&format!("\"{key}\"")), "orphaned: {key}");
        }
    }
}
