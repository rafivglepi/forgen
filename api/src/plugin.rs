use crate::{FileReplacement, WorkspaceContext};

/// Implemented by every Forgen plugin.
///
/// The Forgen runtime calls [`Plugin::run`] **once per invocation** with the
/// complete [`WorkspaceContext`] and collects the returned [`FileReplacement`]
/// list. Plugins may freely cross-reference files and return replacements for
/// any number of files in a single call.
///
/// # Built-in vs. dynamic plugins
///
/// Plugins can be used in two ways:
///
/// 1. **Built-in** — compiled directly into `cargo-forgen`. Implement this
///    trait and register the plugin in the runner. Useful during development.
///
/// 2. **Dynamic (dylib)** — compiled as a `cdylib` crate and placed in the
///    `forgen-plugins/` directory of the workspace. Use the [`plugin_export!`]
///    macro to generate the required C-ABI entry points.
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
    /// - The runner applies replacements in **reverse offset order**, so
    ///   plugins do not need to account for position shifts caused by
    ///   earlier insertions in the same file.
    /// - Returning an empty `Vec` is valid and means "no changes".
    fn run(&self, ctx: &WorkspaceContext) -> Vec<FileReplacement>;
}

/// Generates the C-ABI entry points required for a dynamically-loaded Forgen
/// plugin (a `cdylib` crate).
///
/// The plugin type must implement both [`Plugin`] and [`Default`].
/// `Default` is used to construct the plugin on each invocation; if your
/// plugin requires configuration, read it from the [`WorkspaceContext`] or
/// from environment variables / files inside [`Plugin::run`].
///
/// # Generated symbols
///
/// | Symbol                  | Signature                                       |
/// |-------------------------|-------------------------------------------------|
/// | `forgen_plugin_name`    | `extern "C" fn() -> *const c_char`              |
/// | `forgen_run`            | `extern "C" fn(*const c_char) -> *mut c_char`   |
/// | `forgen_free`           | `unsafe extern "C" fn(*mut c_char)`             |
///
/// `forgen_run` accepts a JSON-encoded [`WorkspaceContext`] and returns a
/// JSON-encoded `Vec<FileReplacement>`. The caller (the Forgen runtime) is
/// responsible for freeing the returned pointer with `forgen_free`.
///
/// # Example
///
/// ```rust,no_run
/// // In your cdylib plugin crate (Cargo.toml: crate-type = ["cdylib"])
/// use forgen_api::{plugin_export, FileReplacement, Plugin, WorkspaceContext};
///
/// #[derive(Default)]
/// pub struct MyPlugin;
///
/// impl Plugin for MyPlugin {
///     fn name(&self) -> &str {
///         "my-plugin"
///     }
///
///     fn run(&self, ctx: &WorkspaceContext) -> Vec<FileReplacement> {
///         // Inspect ctx.files, build replacements …
///         vec![]
///     }
/// }
///
/// plugin_export!(MyPlugin, "my-plugin");
/// ```
#[macro_export]
macro_rules! plugin_export {
    ($plugin_type:ty, $name:literal) => {
        /// Returns the null-terminated plugin name as a static C string.
        ///
        /// The returned pointer has `'static` lifetime and must **not** be freed.
        #[no_mangle]
        pub extern "C" fn forgen_plugin_name() -> *const ::std::os::raw::c_char {
            // Safety: the string literal has a trailing null byte, is valid
            // UTF-8, and lives for the duration of the program.
            concat!($name, "\0").as_ptr() as *const ::std::os::raw::c_char
        }

        /// Accepts a JSON-encoded [`WorkspaceContext`], runs the plugin, and
        /// returns a JSON-encoded `Vec<FileReplacement>`.
        ///
        /// The caller must pass the returned pointer to [`forgen_free`] when
        /// done. Panics (rather than returning null) on serialization errors so
        /// that bugs surface loudly during development.
        #[no_mangle]
        pub extern "C" fn forgen_run(
            ctx_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            // Safety: the caller guarantees `ctx_json` is a valid,
            // null-terminated UTF-8 C string for the duration of this call.
            let ctx_str = unsafe {
                ::std::ffi::CStr::from_ptr(ctx_json)
                    .to_str()
                    .expect("forgen_run: ctx_json is not valid UTF-8")
            };

            let ctx: $crate::WorkspaceContext = $crate::serde_json::from_str(ctx_str)
                .expect("forgen_run: failed to deserialise WorkspaceContext");

            let plugin = <$plugin_type as ::std::default::Default>::default();
            let result = $crate::Plugin::run(&plugin, &ctx);

            let json = $crate::serde_json::to_string(&result)
                .expect("forgen_run: failed to serialise Vec<FileReplacement>");

            ::std::ffi::CString::new(json)
                .expect("forgen_run: serialised result contains an interior null byte")
                .into_raw()
        }

        /// Frees a pointer previously returned by [`forgen_run`].
        ///
        /// # Safety
        ///
        /// - `ptr` must have been obtained from a call to [`forgen_run`].
        /// - `ptr` must not have been freed already.
        /// - Passing `null` is safe and is a no-op.
        #[no_mangle]
        pub unsafe extern "C" fn forgen_free(ptr: *mut ::std::os::raw::c_char) {
            if !ptr.is_null() {
                // Reconstruct the CString and let it drop, which frees the
                // memory that was allocated by `CString::into_raw`.
                drop(::std::ffi::CString::from_raw(ptr));
            }
        }
    };
}
