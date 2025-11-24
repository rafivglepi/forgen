use crate::analysis::AnalysisContext;
use ra_ap_hir::{HasSource, ModuleDef};
use ra_ap_ide_db::FileId;

pub trait HirItemExt {
    fn display_name(&self, ctx: &AnalysisContext) -> Option<String>;

    fn file_id(&self, ctx: &AnalysisContext) -> Option<FileId>;
}

impl HirItemExt for ModuleDef {
    fn display_name(&self, ctx: &AnalysisContext) -> Option<String> {
        match self {
            ModuleDef::Function(f) => Some(f.name(ctx.db).display(ctx.db, ctx.edition).to_string()),
            ModuleDef::Adt(adt) => match adt {
                ra_ap_hir::Adt::Struct(s) => {
                    Some(s.name(ctx.db).display(ctx.db, ctx.edition).to_string())
                },
                ra_ap_hir::Adt::Enum(e) => {
                    Some(e.name(ctx.db).display(ctx.db, ctx.edition).to_string())
                },
                ra_ap_hir::Adt::Union(u) => {
                    Some(u.name(ctx.db).display(ctx.db, ctx.edition).to_string())
                },
            },
            ModuleDef::Trait(t) => Some(t.name(ctx.db).display(ctx.db, ctx.edition).to_string()),
            ModuleDef::TypeAlias(t) => {
                Some(t.name(ctx.db).display(ctx.db, ctx.edition).to_string())
            },
            ModuleDef::Const(c) => c
                .name(ctx.db)
                .map(|n| n.display(ctx.db, ctx.edition).to_string()),
            ModuleDef::Static(s) => Some(s.name(ctx.db).display(ctx.db, ctx.edition).to_string()),
            _ => None,
        }
    }

    fn file_id(&self, ctx: &AnalysisContext) -> Option<FileId> {
        match self {
            ModuleDef::Function(f) => f
                .source(ctx.db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::Adt(adt) => match adt {
                ra_ap_hir::Adt::Struct(s) => s
                    .source(ctx.db)
                    .and_then(|src| src.file_id.file_id())
                    .map(|eid| eid.file_id()),
                ra_ap_hir::Adt::Enum(e) => e
                    .source(ctx.db)
                    .and_then(|src| src.file_id.file_id())
                    .map(|eid| eid.file_id()),
                ra_ap_hir::Adt::Union(u) => u
                    .source(ctx.db)
                    .and_then(|src| src.file_id.file_id())
                    .map(|eid| eid.file_id()),
            },
            ModuleDef::Trait(t) => t
                .source(ctx.db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::TypeAlias(t) => t
                .source(ctx.db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::Const(c) => c
                .source(ctx.db)
                .and_then(|s| s.file_id.file_id())
                .map(|eid| eid.file_id()),
            ModuleDef::Static(s) => s
                .source(ctx.db)
                .and_then(|src| src.file_id.file_id())
                .map(|eid| eid.file_id()),
            _ => None,
        }
    }
}
