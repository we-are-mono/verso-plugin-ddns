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
