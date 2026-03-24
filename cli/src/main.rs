mod loader;
mod replacements;
mod workspace;

use anyhow::{Context, Result};
use cargo_metadata::DependencyKind as CargoDep;
use clap::{Parser, Subcommand};
use forgen_api::Replacement;
use forgen_api::{
    syntax::raw::{Child as SyntaxChild, RawNode, RawToken},
    syntax::SyntaxKind,
    Dependency, DependencySource, DirNode, EnumDef, FieldDef, FileContext as ApiFileContext,
    FileRef, FnDef, FnParam, FsEntry, ImplDef, LetBinding, PackageManifest, Plugin, StructDef,
    TextRange as ApiTextRange, VariantDef, WorkspaceContext, WorkspaceManifest,
};
use notify_debouncer_mini::{new_debouncer, notify::*};
use ra_ap_hir::{Crate, HirDisplay, Semantics};
use ra_ap_ide_db::{base_db::SourceDatabase, EditionedFileId, FileId, RootDatabase};
use ra_ap_paths::AbsPathBuf;
use ra_ap_syntax::{ast, ast::HasName, ast::HasVisibility, AstNode, Edition, SyntaxElement};
use ra_ap_vfs::Vfs;
use std::collections::{HashMap, HashSet};

use std::path::PathBuf;
use std::sync::{mpsc::channel, OnceLock};
use std::time::Duration;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Forgen - compile-time codegen for Rust
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, bin_name = "cargo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the Forgen plugin runner
    Forgen(Args),
}

#[derive(Parser, Debug)]
struct Args {
    /// Path to Cargo.toml (defaults to ./Cargo.toml in current directory)
    #[arg(value_name = "MANIFEST")]
    manifest: Option<PathBuf>,

    /// Watch for file changes and re-run plugins (development mode)
    #[arg(short, long)]
    watch: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    let Command::Forgen(args) = cli.command;

    println!("🚀 Forgen");
    println!("=========================================\n");

    let manifest_path = args.manifest.unwrap_or_else(|| PathBuf::from("Cargo.toml"));
    println!("📦 Loading project: {}", manifest_path.display());

    let manifest_path_abs = manifest_path.canonicalize()?;
    let manifest_path_str = manifest_path_abs
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?;
    let manifest_path = AbsPathBuf::try_from(manifest_path_str)
        .map_err(|e| anyhow::anyhow!("Invalid path: {:?}", e))?;

    let workspace_info = workspace::get_workspace_info(&manifest_path_abs)?;
    let (mut host, mut vfs) = workspace::load_workspace(&manifest_path)?;

    if args.watch {
        println!("👀 Watch mode enabled - monitoring for changes...\n");
        println!("Press Ctrl+C to stop\n");

        run_plugins(&host, &vfs, &workspace_info)?;

        let (tx, rx) = channel();
        let mut debouncer = new_debouncer(Duration::from_millis(500), tx)?;

        if workspace_info.members.is_empty() {
            anyhow::bail!("No source directories found to watch");
        }

        for src_path in &workspace_info.members {
            debouncer
                .watcher()
                .watch(src_path, RecursiveMode::Recursive)
                .with_context(|| format!("Failed to watch {:?}", src_path))?;
            println!("📁 Watching: {}", src_path.display());
        }
        println!();

        loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    // Collect every .rs path that was touched — this covers
                    // creates, deletes, renames, and plain modifications since
                    // notify-debouncer-mini collapses all of them into a single
                    // path-level event.
                    let changed_files: Vec<_> = events
                        .iter()
                        .filter(|e| e.path.extension().and_then(|s| s.to_str()) == Some("rs"))
                        .map(|e| e.path.clone())
                        .collect();

                    if !changed_files.is_empty() {
                        // Summarise what happened so the user knows why a
                        // re-run was triggered (created / deleted / modified).
                        let created: Vec<_> = changed_files
                            .iter()
                            .filter(|p| {
                                // A file that exists now but the VFS doesn't
                                // know about yet is effectively "new".
                                p.exists()
                            })
                            .collect();
                        let deleted: Vec<_> =
                            changed_files.iter().filter(|p| !p.exists()).collect();

                        if !created.is_empty() {
                            for p in &created {
                                println!(
                                    "  📝 {}",
                                    p.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("(unknown)")
                                );
                            }
                        }
                        if !deleted.is_empty() {
                            for p in &deleted {
                                println!(
                                    "  🗑  {}",
                                    p.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("(unknown)")
                                );
                            }
                        }

                        println!("🔄 File system change detected, re-running plugins...");
                        match workspace::apply_file_changes(&mut host, &mut vfs, &changed_files) {
                            Ok(_) => match run_plugins(&host, &vfs, &workspace_info) {
                                Ok(_) => println!("✅ Done\n"),
                                Err(e) => eprintln!("❌ Plugin error: {}\n", e),
                            },
                            Err(e) => eprintln!("❌ Error applying file changes: {}\n", e),
                        }
                    }
                }
                Ok(Err(e)) => eprintln!("Watch error: {:?}", e),
                Err(e) => {
                    eprintln!("Channel error: {:?}", e);
                    break;
                }
            }
        }
    } else {
        run_plugins(&host, &vfs, &workspace_info)?;
        println!("\n✨ Done!");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Plugin runner
// ---------------------------------------------------------------------------

fn run_plugins(
    db: &RootDatabase,
    vfs: &Vfs,
    workspace_info: &workspace::WorkspaceInfo,
) -> Result<()> {
    let project_dir = &workspace_info.root;
    let cargo_meta = &workspace_info.cargo_metadata;

    // Plugins come entirely from the plugin suite — no built-ins.
    let mut plugins: Vec<Box<dyn Plugin>> = Vec::new();

    // Plugin suite: `[workspace.metadata.forgen] suite = "..."`
    if let Some(suite) = loader::load_suite(cargo_meta) {
        println!();
        plugins.push(suite);
    }

    let sema = Semantics::new(db);

    // Enumerate source files.
    let mut seen: HashSet<FileId> = HashSet::new();
    let mut file_queue: Vec<EditionedFileId> = Vec::new();
    for krate in Crate::all(db) {
        if krate.origin(db).is_local() {
            collect_module_files(db, &krate.root_module(), &mut seen, &mut file_queue);
        }
    }

    println!(
        "🔍 Building workspace context from {} file(s)...",
        file_queue.len()
    );

    let workspace_ctx =
        build_workspace_context(&sema, db, vfs, project_dir, file_queue, cargo_meta)?;

    println!("🧩 Running {} plugin(s)...\n", plugins.len());

    let mut total_changes: usize = 0;
    let mut replacements_by_path: HashMap<String, Vec<Replacement>> = HashMap::new();

    for plugin in &plugins {
        let file_replacements = plugin.run(&workspace_ctx);

        for fr in file_replacements {
            if fr.replacements.is_empty() {
                continue;
            }

            println!(
                "  🧩 {} → {} replacement(s)  [{}]",
                fr.path,
                fr.replacements.len(),
                plugin.name(),
            );
            total_changes += fr.replacements.len();
            replacements_by_path
                .entry(fr.path)
                .or_default()
                .extend(fr.replacements);
        }
    }

    let total_saved = replacements::write_saved_replacements(
        project_dir,
        &workspace_ctx.files,
        &replacements_by_path,
    )?;

    println!();
    if total_saved > 0 {
        println!(
            "✅ Saved {} total replacement patch(es) to target/.forgen/",
            total_saved
        );
    } else if total_changes > 0 {
        println!("✅ Replacements were generated, but all serialised patch sets were empty");
    } else {
        println!("✅ No replacements generated");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// WorkspaceContext builder
// ---------------------------------------------------------------------------

fn build_workspace_context(
    sema: &Semantics<RootDatabase>,
    db: &RootDatabase,
    vfs: &Vfs,
    project_dir: &PathBuf,
    file_queue: Vec<EditionedFileId>,
    cargo_meta: &cargo_metadata::Metadata,
) -> Result<WorkspaceContext> {
    let mut files: Vec<ApiFileContext> = Vec::new();

    // Normalise the workspace root to a forward-slash string once so we can
    // do reliable prefix stripping even on Windows where VFS paths may carry
    // the `\\?\` extended-path prefix while `project_dir` does not.
    let root_norm = normalize_path_str(&project_dir.to_string_lossy());

    for editioned_id in file_queue {
        let file_id = editioned_id.file_id();

        let Some(path) = workspace::file_id_to_path(vfs, file_id, project_dir) else {
            continue;
        };

        let source = String::from(&*SourceDatabase::file_text(db, file_id));
        let parsed = sema.parse(editioned_id);
        let syntax = parsed.syntax();

        // Pre-compute inferred types for unannotated `let` bindings.
        let mut pat_type_cache: HashMap<(u32, u32), String> = HashMap::new();
        for node in syntax.descendants() {
            let Some(let_stmt) = ast::LetStmt::cast(node) else {
                continue;
            };
            if let_stmt.ty().is_some() {
                continue;
            }
            let Some(pat) = let_stmt.pat() else { continue };
            let Some(init) = let_stmt.initializer() else {
                continue;
            };
            if let Some(type_info) = sema.type_of_expr(&init) {
                let ty_str = type_info.original.display(db, Edition::CURRENT).to_string();
                let r = pat.syntax().text_range();
                pat_type_cache.insert((u32::from(r.start()), u32::from(r.end())), ty_str);
            }
        }

        let rel_path = {
            let p = normalize_path_str(&path.to_string_lossy());
            p.strip_prefix(&root_norm)
                .map(|s| s.trim_start_matches('/').to_owned())
                .unwrap_or(p)
        };

        // Build the full serialisable CST (includes all trivia/comments).
        let tree = build_raw_node(syntax);

        files.push(ApiFileContext {
            path: rel_path,
            source,
            tree,
            let_bindings: extract_let_bindings(syntax, &pat_type_cache),
            functions: extract_functions(syntax),
            structs: extract_structs(syntax),
            enums: extract_enums(syntax),
            impls: extract_impls(syntax),
        });
    }

    let manifest = build_manifest(cargo_meta);
    let file_tree = build_file_tree(&files);

    Ok(WorkspaceContext {
        workspace_root: root_norm,
        manifest,
        file_tree,
        files,
    })
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Collapse backslashes to forward slashes and strip the Windows extended-path
/// prefix (`\\?\` or `\\?\UNC\`) so that two paths referring to the same
/// location always compare equal as strings.
fn normalize_path_str(raw: &str) -> String {
    let s = raw.replace('\\', "/");
    // Strip \\?\ (becomes //?/ after backslash replacement)
    if let Some(rest) = s.strip_prefix("//?/UNC/") {
        return format!("//{}", rest);
    }
    if let Some(rest) = s.strip_prefix("//?/") {
        return rest.to_owned();
    }
    s
}

// ---------------------------------------------------------------------------
// CST converter  (ra_ap_syntax → forgen_api::syntax)
// ---------------------------------------------------------------------------

/// Converts a `ra_ap_syntax::SyntaxKind` to our `SyntaxKind` via the debug-
/// string name. This avoids binding to the internal numeric representation of
/// ra_ap_syntax and degrades gracefully (unknown → `ERROR`) when using an
/// older or newer version of ra_ap_syntax that has different variants.
fn convert_kind(k: ra_ap_syntax::SyntaxKind) -> SyntaxKind {
    type A = SyntaxKind;
    static MAP: OnceLock<HashMap<&'static str, SyntaxKind>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        let mut m: HashMap<&'static str, SyntaxKind> = HashMap::with_capacity(320);
        macro_rules! ins {
            ($($n:ident),* $(,)?) => { $(m.insert(stringify!($n), A::$n);)* }
        }
        ins!(
            // Punctuation
            DOLLAR,
            SEMICOLON,
            COMMA,
            L_PAREN,
            R_PAREN,
            L_CURLY,
            R_CURLY,
            L_BRACK,
            R_BRACK,
            L_ANGLE,
            R_ANGLE,
            AT,
            POUND,
            TILDE,
            QUESTION,
            AMP,
            PIPE,
            PLUS,
            STAR,
            SLASH,
            CARET,
            PERCENT,
            UNDERSCORE,
            DOT,
            DOT2,
            DOT3,
            DOT2EQ,
            COLON,
            COLON2,
            EQ,
            EQ2,
            FAT_ARROW,
            BANG,
            NEQ,
            MINUS,
            THIN_ARROW,
            LTEQ,
            GTEQ,
            PLUSEQ,
            MINUSEQ,
            PIPEEQ,
            AMPEQ,
            CARETEQ,
            SLASHEQ,
            STAREQ,
            PERCENTEQ,
            AMP2,
            PIPE2,
            SHL,
            SHR,
            SHLEQ,
            SHREQ,
            // Strict keywords
            SELF_TYPE_KW,
            ABSTRACT_KW,
            AS_KW,
            BECOME_KW,
            BOX_KW,
            BREAK_KW,
            CONST_KW,
            CONTINUE_KW,
            CRATE_KW,
            DO_KW,
            ELSE_KW,
            ENUM_KW,
            EXTERN_KW,
            FALSE_KW,
            FINAL_KW,
            FN_KW,
            FOR_KW,
            IF_KW,
            IMPL_KW,
            IN_KW,
            LET_KW,
            LOOP_KW,
            MACRO_KW,
            MATCH_KW,
            MOD_KW,
            MOVE_KW,
            MUT_KW,
            OVERRIDE_KW,
            PRIV_KW,
            PUB_KW,
            REF_KW,
            RETURN_KW,
            SELF_KW,
            STATIC_KW,
            STRUCT_KW,
            SUPER_KW,
            TRAIT_KW,
            TRUE_KW,
            TYPE_KW,
            TYPEOF_KW,
            UNSAFE_KW,
            UNSIZED_KW,
            USE_KW,
            VIRTUAL_KW,
            WHERE_KW,
            WHILE_KW,
            YIELD_KW,
            // Contextual keywords
            ASM_KW,
            ASYNC_KW,
            ATT_SYNTAX_KW,
            AUTO_KW,
            BUILTIN_KW,
            CLOBBER_ABI_KW,
            DEFAULT_KW,
            DYN_KW,
            FORMAT_ARGS_KW,
            GEN_KW,
            GLOBAL_ASM_KW,
            LABEL_KW,
            MACRO_RULES_KW,
            NAKED_ASM_KW,
            OFFSET_OF_KW,
            OPTIONS_KW,
            PRESERVES_FLAGS_KW,
            PURE_KW,
            RAW_KW,
            READONLY_KW,
            SAFE_KW,
            SYM_KW,
            TRY_KW,
            UNION_KW,
            YEET_KW,
            // Literals
            BYTE,
            BYTE_STRING,
            CHAR,
            C_STRING,
            FLOAT_NUMBER,
            INT_NUMBER,
            STRING,
            // Trivia / special tokens
            COMMENT,
            ERROR,
            FRONTMATTER,
            IDENT,
            LIFETIME_IDENT,
            NEWLINE,
            SHEBANG,
            WHITESPACE,
            TOMBSTONE,
            // Composite node kinds
            ABI,
            ARG_LIST,
            ARRAY_EXPR,
            ARRAY_TYPE,
            ASM_CLOBBER_ABI,
            ASM_CONST,
            ASM_DIR_SPEC,
            ASM_EXPR,
            ASM_LABEL,
            ASM_OPERAND_EXPR,
            ASM_OPERAND_NAMED,
            ASM_OPTION,
            ASM_OPTIONS,
            ASM_REG_OPERAND,
            ASM_REG_SPEC,
            ASM_SYM,
            ASSOC_ITEM_LIST,
            ASSOC_TYPE_ARG,
            ATTR,
            AWAIT_EXPR,
            BECOME_EXPR,
            BIN_EXPR,
            BLOCK_EXPR,
            BOX_PAT,
            BREAK_EXPR,
            CALL_EXPR,
            CAST_EXPR,
            CLOSURE_EXPR,
            CONST,
            CONST_ARG,
            CONST_BLOCK_PAT,
            CONST_PARAM,
            CONTINUE_EXPR,
            DYN_TRAIT_TYPE,
            ENUM,
            EXPR_STMT,
            EXTERN_BLOCK,
            EXTERN_CRATE,
            EXTERN_ITEM_LIST,
            FIELD_EXPR,
            FN,
            FN_PTR_TYPE,
            FOR_BINDER,
            FOR_EXPR,
            FOR_TYPE,
            FORMAT_ARGS_ARG,
            FORMAT_ARGS_ARG_NAME,
            FORMAT_ARGS_EXPR,
            GENERIC_ARG_LIST,
            GENERIC_PARAM_LIST,
            IDENT_PAT,
            IF_EXPR,
            IMPL,
            IMPL_TRAIT_TYPE,
            INDEX_EXPR,
            INFER_TYPE,
            ITEM_LIST,
            LABEL,
            LET_ELSE,
            LET_EXPR,
            LET_STMT,
            LIFETIME,
            LIFETIME_ARG,
            LIFETIME_PARAM,
            LITERAL,
            LITERAL_PAT,
            LOOP_EXPR,
            MACRO_CALL,
            MACRO_DEF,
            MACRO_EXPR,
            MACRO_ITEMS,
            MACRO_PAT,
            MACRO_RULES,
            MACRO_STMTS,
            MACRO_TYPE,
            MATCH_ARM,
            MATCH_ARM_LIST,
            MATCH_EXPR,
            MATCH_GUARD,
            META,
            METHOD_CALL_EXPR,
            MODULE,
            NAME,
            NAME_REF,
            NEVER_TYPE,
            OFFSET_OF_EXPR,
            OR_PAT,
            PARAM,
            PARAM_LIST,
            PAREN_EXPR,
            PAREN_PAT,
            PAREN_TYPE,
            PARENTHESIZED_ARG_LIST,
            PATH,
            PATH_EXPR,
            PATH_PAT,
            PATH_SEGMENT,
            PATH_TYPE,
            PREFIX_EXPR,
            PTR_TYPE,
            RANGE_EXPR,
            RANGE_PAT,
            RECORD_EXPR,
            RECORD_EXPR_FIELD,
            RECORD_EXPR_FIELD_LIST,
            RECORD_FIELD,
            RECORD_FIELD_LIST,
            RECORD_PAT,
            RECORD_PAT_FIELD,
            RECORD_PAT_FIELD_LIST,
            REF_EXPR,
            REF_PAT,
            REF_TYPE,
            RENAME,
            REST_PAT,
            RET_TYPE,
            RETURN_EXPR,
            RETURN_TYPE_SYNTAX,
            SELF_PARAM,
            SLICE_PAT,
            SLICE_TYPE,
            SOURCE_FILE,
            STATIC,
            STMT_LIST,
            STRUCT,
            TOKEN_TREE,
            TRAIT,
            TRAIT_ALIAS,
            TRY_BLOCK_MODIFIER,
            TRY_EXPR,
            TUPLE_EXPR,
            TUPLE_FIELD,
            TUPLE_FIELD_LIST,
            TUPLE_PAT,
            TUPLE_STRUCT_PAT,
            TUPLE_TYPE,
            TYPE_ALIAS,
            TYPE_ANCHOR,
            TYPE_ARG,
            TYPE_BOUND,
            TYPE_BOUND_LIST,
            TYPE_PARAM,
            UNDERSCORE_EXPR,
            UNION,
            USE,
            USE_BOUND_GENERIC_ARGS,
            USE_TREE,
            USE_TREE_LIST,
            VARIANT,
            VARIANT_LIST,
            VISIBILITY,
            WHERE_CLAUSE,
            WHERE_PRED,
            WHILE_EXPR,
            WILDCARD_PAT,
            YEET_EXPR,
            YIELD_EXPR,
        );
        m
    });
    let s = format!("{k:?}");
    map.get(s.as_str()).copied().unwrap_or(SyntaxKind::ERROR)
}

/// Recursively serialise a `ra_ap_syntax::SyntaxNode` into a [`RawNode`],
/// preserving the full CST including whitespace and comment tokens.
fn build_raw_node(node: &ra_ap_syntax::SyntaxNode) -> RawNode {
    RawNode {
        kind: convert_kind(node.kind()),
        range: to_api_range(node.text_range()),
        children: node
            .children_with_tokens()
            .map(|child| match child {
                SyntaxElement::Node(n) => SyntaxChild::Node(build_raw_node(&n)),
                SyntaxElement::Token(t) => SyntaxChild::Token(RawToken {
                    kind: convert_kind(t.kind()),
                    text: t.text().to_string(),
                    range: to_api_range(t.text_range()),
                }),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Cargo manifest builder
// ---------------------------------------------------------------------------

fn build_manifest(meta: &cargo_metadata::Metadata) -> WorkspaceManifest {
    let members: Vec<PackageManifest> = meta
        .workspace_packages()
        .iter()
        .map(|pkg| {
            let mut deps = Vec::new();
            let mut dev_deps = Vec::new();
            let mut build_deps = Vec::new();

            for dep in &pkg.dependencies {
                let converted = convert_dependency(dep);
                match dep.kind {
                    CargoDep::Normal => deps.push(converted),
                    CargoDep::Development => dev_deps.push(converted),
                    CargoDep::Build => build_deps.push(converted),
                    _ => deps.push(converted),
                }
            }

            let features: HashMap<String, Vec<String>> = pkg
                .features
                .iter()
                .map(|(k, v): (&String, &Vec<String>)| (k.clone(), v.clone()))
                .collect();

            PackageManifest {
                name: pkg.name.clone(),
                version: pkg.version.to_string(),
                edition: pkg.edition.to_string(),
                authors: pkg.authors.clone(),
                description: pkg.description.clone(),
                license: pkg.license.clone(),
                repository: pkg.repository.clone(),
                dependencies: deps,
                dev_dependencies: dev_deps,
                build_dependencies: build_deps,
                features,
                metadata: pkg.metadata.clone(),
            }
        })
        .collect();

    WorkspaceManifest {
        members,
        workspace_root: meta.workspace_root.to_string().replace('\\', "/"),
        target_directory: meta.target_directory.to_string().replace('\\', "/"),
        metadata: meta.workspace_metadata.clone(),
    }
}

fn convert_dependency(dep: &cargo_metadata::Dependency) -> Dependency {
    let source = if let Some(path) = &dep.path {
        DependencySource::Path {
            path: path.to_string().replace('\\', "/"),
        }
    } else {
        match dep.source.as_deref() {
            Some(s) if s.starts_with("git+") => DependencySource::Git {
                url: s.to_string(),
                rev: None,
            },
            Some(_) => DependencySource::Registry,
            None => DependencySource::Unknown,
        }
    };

    Dependency {
        name: dep.name.clone(),
        rename: dep.rename.clone(),
        req: dep.req.to_string(),
        features: dep.features.clone(),
        optional: dep.optional,
        default_features: dep.uses_default_features,
        source,
    }
}

// ---------------------------------------------------------------------------
// File-tree builder
// ---------------------------------------------------------------------------

fn build_file_tree(files: &[ApiFileContext]) -> DirNode {
    let mut root = DirNode {
        name: String::new(),
        path: String::new(),
        entries: Vec::new(),
    };
    for file in files {
        let parts: Vec<&str> = file.path.split('/').collect();
        insert_into_tree(&mut root, &parts, &file.path);
    }
    sort_dir(&mut root);
    root
}

fn insert_into_tree(dir: &mut DirNode, remaining: &[&str], full_path: &str) {
    if remaining.is_empty() {
        return;
    }
    if remaining.len() == 1 {
        dir.entries.push(FsEntry::File(FileRef {
            name: remaining[0].to_string(),
            path: full_path.to_string(),
        }));
        return;
    }

    let dir_name = remaining[0];
    let existing_idx = dir
        .entries
        .iter()
        .position(|e| matches!(e, FsEntry::Dir(d) if d.name == dir_name));

    let idx = if let Some(i) = existing_idx {
        i
    } else {
        let dir_path = if dir.path.is_empty() {
            dir_name.to_string()
        } else {
            format!("{}/{}", dir.path, dir_name)
        };
        dir.entries.push(FsEntry::Dir(DirNode {
            name: dir_name.to_string(),
            path: dir_path,
            entries: Vec::new(),
        }));
        dir.entries.len() - 1
    };

    if let FsEntry::Dir(subdir) = &mut dir.entries[idx] {
        insert_into_tree(subdir, &remaining[1..], full_path);
    }
}

fn sort_dir(dir: &mut DirNode) {
    dir.entries.sort_by(|a, b| {
        let name_a = match a {
            FsEntry::Dir(d) => d.name.as_str(),
            FsEntry::File(f) => f.name.as_str(),
        };
        let name_b = match b {
            FsEntry::Dir(d) => d.name.as_str(),
            FsEntry::File(f) => f.name.as_str(),
        };
        name_a.cmp(name_b)
    });
    for entry in &mut dir.entries {
        if let FsEntry::Dir(subdir) = entry {
            sort_dir(subdir);
        }
    }
}

// ---------------------------------------------------------------------------
// AST → API type converters  (typed helper fields on FileContext)
// ---------------------------------------------------------------------------

#[inline]
fn to_api_range(r: ra_ap_syntax::TextRange) -> ApiTextRange {
    ApiTextRange {
        start: u32::from(r.start()),
        end: u32::from(r.end()),
    }
}

fn extract_let_bindings(
    syntax: &ra_ap_syntax::SyntaxNode,
    pat_types: &HashMap<(u32, u32), String>,
) -> Vec<LetBinding> {
    let mut bindings = Vec::new();

    for node in syntax.descendants() {
        let Some(let_stmt) = ast::LetStmt::cast(node) else {
            continue;
        };
        let Some(pat) = let_stmt.pat() else { continue };

        let (name, is_mut) = match &pat {
            ast::Pat::IdentPat(ident_pat) => {
                let Some(n) = ident_pat.name() else { continue };
                (n.to_string(), ident_pat.mut_token().is_some())
            }
            _ => continue,
        };

        if name.is_empty() || name == "_" {
            continue;
        }

        let explicit_type = let_stmt
            .ty()
            .map(|ty| ty.syntax().text().to_string().trim().to_owned());

        let inferred_type = if explicit_type.is_none() {
            let r = pat.syntax().text_range();
            pat_types
                .get(&(u32::from(r.start()), u32::from(r.end())))
                .cloned()
        } else {
            None
        };

        bindings.push(LetBinding {
            name,
            explicit_type,
            inferred_type,
            range: to_api_range(let_stmt.syntax().text_range()),
            is_mut,
        });
    }

    bindings
}

fn extract_fn_def(fn_node: &ast::Fn) -> Option<FnDef> {
    let name = fn_node.name()?.to_string();

    let has_self = fn_node
        .param_list()
        .and_then(|pl| pl.self_param())
        .is_some();

    let params: Vec<FnParam> = fn_node
        .param_list()
        .map(|pl| {
            pl.params()
                .map(|p| {
                    let name = p
                        .pat()
                        .and_then(|pat| match pat {
                            ast::Pat::IdentPat(ip) => ip.name().map(|n| n.to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "_".to_string());
                    let ty = p
                        .ty()
                        .map(|t| t.syntax().text().to_string().trim().to_owned());
                    FnParam { name, ty }
                })
                .collect()
        })
        .unwrap_or_default();

    let return_type = fn_node
        .ret_type()
        .and_then(|rt| rt.ty())
        .map(|t| t.syntax().text().to_string().trim().to_owned());

    let is_pub = fn_node
        .visibility()
        .map(|v| v.syntax().text().to_string().starts_with("pub"))
        .unwrap_or(false);

    let is_async = fn_node.async_token().is_some();

    Some(FnDef {
        name,
        params,
        has_self,
        return_type,
        range: to_api_range(fn_node.syntax().text_range()),
        is_pub,
        is_async,
    })
}

fn extract_functions(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<FnDef> {
    syntax
        .descendants()
        .filter_map(ast::Fn::cast)
        .filter_map(|fn_node| extract_fn_def(&fn_node))
        .collect()
}

fn extract_record_fields(list: &ast::RecordFieldList) -> Vec<FieldDef> {
    list.fields()
        .filter_map(|f| {
            let name = f.name()?.to_string();
            let ty = f
                .ty()
                .map(|t| t.syntax().text().to_string().trim().to_owned())
                .unwrap_or_default();
            let is_pub = f
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);
            Some(FieldDef { name, ty, is_pub })
        })
        .collect()
}

fn extract_tuple_fields(list: &ast::TupleFieldList) -> Vec<FieldDef> {
    list.fields()
        .enumerate()
        .map(|(i, f)| {
            let ty = f
                .ty()
                .map(|t| t.syntax().text().to_string().trim().to_owned())
                .unwrap_or_default();
            let is_pub = f
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);
            FieldDef {
                name: i.to_string(),
                ty,
                is_pub,
            }
        })
        .collect()
}

fn extract_structs(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<StructDef> {
    syntax
        .descendants()
        .filter_map(ast::Struct::cast)
        .filter_map(|s| {
            let name = s.name()?.to_string();
            let is_pub = s
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);
            let (fields, tuple_fields) = match s.field_list() {
                Some(ast::FieldList::RecordFieldList(list)) => {
                    (extract_record_fields(&list), vec![])
                }
                Some(ast::FieldList::TupleFieldList(list)) => (vec![], extract_tuple_fields(&list)),
                None => (vec![], vec![]),
            };
            Some(StructDef {
                name,
                fields,
                tuple_fields,
                range: to_api_range(s.syntax().text_range()),
                is_pub,
            })
        })
        .collect()
}

fn extract_enums(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<EnumDef> {
    syntax
        .descendants()
        .filter_map(ast::Enum::cast)
        .filter_map(|e| {
            let name = e.name()?.to_string();
            let is_pub = e
                .visibility()
                .map(|v| v.syntax().text().to_string().starts_with("pub"))
                .unwrap_or(false);
            let variants: Vec<VariantDef> = e
                .variant_list()
                .map(|vl| {
                    vl.variants()
                        .filter_map(|v| {
                            let name = v.name()?.to_string();
                            let (fields, tuple_fields) = match v.field_list() {
                                Some(ast::FieldList::RecordFieldList(list)) => {
                                    (extract_record_fields(&list), vec![])
                                }
                                Some(ast::FieldList::TupleFieldList(list)) => {
                                    (vec![], extract_tuple_fields(&list))
                                }
                                None => (vec![], vec![]),
                            };
                            Some(VariantDef {
                                name,
                                fields,
                                tuple_fields,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(EnumDef {
                name,
                variants,
                range: to_api_range(e.syntax().text_range()),
                is_pub,
            })
        })
        .collect()
}

fn extract_impls(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<ImplDef> {
    syntax
        .descendants()
        .filter_map(ast::Impl::cast)
        .filter_map(|impl_node| {
            let self_ty = impl_node
                .self_ty()
                .map(|t| t.syntax().text().to_string().trim().to_owned())?;
            let trait_ = impl_node
                .trait_()
                .map(|t| t.syntax().text().to_string().trim().to_owned());
            let methods: Vec<FnDef> = impl_node
                .assoc_item_list()
                .map(|list| {
                    list.assoc_items()
                        .filter_map(|item| {
                            if let ast::AssocItem::Fn(fn_node) = item {
                                extract_fn_def(&fn_node)
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(ImplDef {
                self_ty,
                trait_,
                methods,
                range: to_api_range(impl_node.syntax().text_range()),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Module file collector
// ---------------------------------------------------------------------------

fn collect_module_files(
    db: &RootDatabase,
    module: &ra_ap_hir::Module,
    seen: &mut HashSet<FileId>,
    queue: &mut Vec<EditionedFileId>,
) {
    if let Some(editioned_id) = module.definition_source(db).file_id.file_id() {
        let file_id = editioned_id.file_id();
        if seen.insert(file_id) {
            queue.push(editioned_id);
        }
    }
    for child in module.children(db) {
        collect_module_files(db, &child, seen, queue);
    }
}
