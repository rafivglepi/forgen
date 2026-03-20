//! Test-plugins plugin suite for the Forgen development workspace.
//!
//! This crate shows the recommended layout for a real project's plugin
//! suite: plugin logic lives in sub-modules of the same crate,
//! keeping everything in one place without an extra workspace member.

mod plugins;

use forgen_api::{plugin_suite, FileReplacement, Plugin, WorkspaceContext};

fn run(ctx: &WorkspaceContext) -> Vec<FileReplacement> {
    let mut out = Vec::new();

    // Register plugins here — just regular Rust method calls, no FFI.
    out.extend(plugins::example::ExamplePlugin.run(ctx));
    out.extend(plugins::f64_logger::F64LoggerPlugin.run(ctx));

    out
}

plugin_suite!(run);
