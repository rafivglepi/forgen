//! Test-plugins plugin suite for the Forgen development workspace.
//!
//! This crate shows the recommended layout for a real project's plugin
//! suite: plugin logic lives in sub-modules of the same crate,
//! keeping everything in one place without an extra workspace member.

mod plugins;

use forgen_api::{plugin_suite, FileReplacement, SuiteRuntime, WorkspaceContext};

fn run(ctx: &WorkspaceContext, runtime: &mut SuiteRuntime) -> Vec<FileReplacement> {
    let mut out = Vec::new();

    // Register plugins here — just regular Rust method calls, no FFI.
    out.extend(runtime.run_plugin(&plugins::example::ExamplePlugin, ctx));
    out.extend(runtime.run_plugin(&plugins::seeded_binding::SeededBindingPlugin, ctx));
    out.extend(runtime.run_plugin(&plugins::f64_logger::F64LoggerPlugin, ctx));

    out
}

plugin_suite!(run);
