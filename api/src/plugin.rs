use crate::{FileReplacement, WorkspaceContext};

/// Implemented by every Forgen plugin.
///
/// Plugin crates are plain Rust library crates (`rlib`) — they implement
/// this trait and are called directly (no FFI, no serialisation) by the
/// suite crate that the user writes for their workspace.
///
/// Only the suite crate itself is compiled as a `cdylib`; it uses
/// [`plugin_suite!`] to expose the single C entry-point that the
/// Forgen CLI loads.
pub trait Plugin: Send + Sync {
    /// Human-readable name used in log output and error messages.
    fn name(&self) -> &str;

    /// Analyse the workspace and return any replacements to apply.
    ///
    /// # Contract
    ///
    /// - Receive the **entire workspace at once** — cross-file analysis is
    ///   fully supported.
    /// - Return one [`FileReplacement`] per file that should be modified.
    ///   Files that need no changes should simply be omitted.
    /// - The runner applies replacements in **reverse offset order** so
    ///   plugins do not need to account for position shifts from earlier
    ///   insertions in the same file.
    /// - Returning an empty `Vec` is valid and means "no changes".
    fn run(&self, ctx: &WorkspaceContext) -> Vec<FileReplacement>;
}

/// Generates the three C-ABI entry points required for a Forgen suite
/// `cdylib` crate.
///
/// The single argument is any expression that, when called with
/// `&WorkspaceContext`, returns `Vec<FileReplacement>`.  In practice this is
/// either a plain function name or a closure that fans out to multiple plugin
/// crates.
///
/// # Generated symbols
///
/// | Symbol                | Signature                                                      |
/// |-----------------------|----------------------------------------------------------------|
/// | `forgen_abi_version`  | `extern "C" fn() -> u64`                                      |
/// | `forgen_run`          | `unsafe extern "C" fn(*const WorkspaceContext) -> *mut Vec<FileReplacement>` |
/// | `forgen_free`         | `unsafe extern "C" fn(*mut Vec<FileReplacement>)`             |
///
/// `forgen_run` receives a raw pointer to the `WorkspaceContext` that lives
/// on the CLI's stack and returns a `Box`-backed pointer to the result.  The
/// CLI frees it with `Box::from_raw`; both sides must use the system
/// allocator (no custom `#[global_allocator]`).
///
/// # Safety requirements
///
/// - Both the CLI binary and the suite dylib must be compiled with the
///   same version of `forgen-api` (the CLI checks [`FORGEN_ABI_VERSION`]
///   before calling anything else).
/// - Neither the CLI nor the suite crate may override the global
///   allocator; both use the system `malloc`/`free` so cross-boundary
///   deallocation is safe.
///
/// # Example
///
/// ```rust,no_run
/// // forgen-suite/src/lib.rs
/// use forgen_api::{plugin_suite, FileReplacement, Plugin, WorkspaceContext};
/// use my_plugin::MyPlugin;
/// use another_plugin::AnotherPlugin;
///
/// plugin_suite!(|ctx: &WorkspaceContext| {
///     let mut out = Vec::new();
///     out.extend(MyPlugin.run(ctx));
///     out.extend(AnotherPlugin.run(ctx));
///     out
/// });
/// ```
///
/// Or with a named function:
///
/// ```rust,no_run
/// use forgen_api::{plugin_suite, FileReplacement, Plugin, WorkspaceContext};
/// use my_plugin::MyPlugin;
///
/// fn run(ctx: &WorkspaceContext) -> Vec<FileReplacement> {
///     MyPlugin.run(ctx)
/// }
///
/// plugin_suite!(run);
/// ```
///
/// [`FORGEN_ABI_VERSION`]: crate::FORGEN_ABI_VERSION
#[macro_export]
macro_rules! plugin_suite {
    ($run_fn:expr) => {
        /// Returns the `forgen-api` ABI version this suite was
        /// compiled against.  The CLI aborts the load if the value does not
        /// match its own compile-time constant.
        #[no_mangle]
        pub extern "C" fn forgen_abi_version() -> u64 {
            $crate::FORGEN_ABI_VERSION
        }

        /// Runs all plugins registered in this suite.
        ///
        /// # Safety
        ///
        /// `__ctx` must be a valid, non-null pointer to a `WorkspaceContext`
        /// that remains valid for the entire duration of this call.  The
        /// returned pointer is a `Box`-backed heap allocation; the caller
        /// must eventually pass it to [`forgen_free`].
        #[no_mangle]
        pub unsafe extern "C" fn forgen_run(
            __ctx: *const $crate::WorkspaceContext,
        ) -> *mut ::std::vec::Vec<$crate::FileReplacement> {
            // Safety: the CLI guarantees __ctx is a valid reference for the
            // duration of this call.
            let __ctx_ref: &$crate::WorkspaceContext = unsafe { &*__ctx };
            let __result: ::std::vec::Vec<$crate::FileReplacement> = ($run_fn)(__ctx_ref);
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(__result))
        }

        /// Frees a `Vec<FileReplacement>` previously returned by
        /// [`forgen_run`].
        ///
        /// # Safety
        ///
        /// - `__ptr` must have been returned by a call to [`forgen_run`].
        /// - `__ptr` must not have been freed already.
        /// - Passing `null` is safe and is a no-op.
        #[no_mangle]
        pub unsafe extern "C" fn forgen_free(__ptr: *mut ::std::vec::Vec<$crate::FileReplacement>) {
            if !__ptr.is_null() {
                // Reconstruct the Box so it is properly deallocated.
                drop(::std::boxed::Box::from_raw(__ptr));
            }
        }
    };
}
