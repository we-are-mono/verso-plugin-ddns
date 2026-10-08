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

fn get(request: &Request) -> Envelope {
    page::page(&model::Ddns::read(request), &request.query.get(page::OPEN))
}

fn post(request: &Request, form: &Form) -> Envelope {
    page::post(
        &model::Ddns::read(request),
        &request.query.get(page::OPEN),
        form,
    )
}

#[cfg(test)]
mod tests {
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
