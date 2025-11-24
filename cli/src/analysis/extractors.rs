use crate::analysis::{extensions::extract_type_names, AnalysisContext};
use crate::models::{
    ClosureInfo, FieldInfo, FileTypeInfo, FunctionBodyInfo, ItemInfo, LocalVarInfo, ParamInfo,
    TraitItemInfo, VariantInfo, INFERRED_TYPE,
};
use anyhow::Result;
use ra_ap_hir::HasSource;
use ra_ap_syntax::{ast, ast::HasName, AstNode};

fn format_id(item: &dyn std::fmt::Debug) -> String {
    format!("{:?}", item).replace(" ", "")
}

/// Trait for extracting different types of items
///
/// This trait provides a uniform interface for extracting various HIR items
/// (functions, structs, enums, etc.) into the Forgen output format.
/// Each item type has its own extractor implementation.
pub trait ItemExtractor<T> {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        item: &T,
        file_info: &mut FileTypeInfo,
        is_external: bool,
    ) -> Result<()>;
}

pub struct FunctionExtractor;
impl ItemExtractor<ra_ap_hir::Function> for FunctionExtractor {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        func: &ra_ap_hir::Function,
        file_info: &mut FileTypeInfo,
        is_external: bool,
    ) -> Result<()> {
        let return_type = ctx.display(func.ret_type(ctx.db));
        extract_type_names(&return_type, ctx.referenced_types);

        file_info.items.push(ItemInfo::Function {
            name: ctx.display_name(func.name(ctx.db)),
            id: format_id(func),
            params: extract_function_params(ctx, func),
            return_type,
            body: if !is_external {
                extract_function_body(ctx, func)
            } else {
                None
            },
        });

        Ok(())
    }
}

pub struct StructExtractor;
impl ItemExtractor<ra_ap_hir::Struct> for StructExtractor {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        struct_def: &ra_ap_hir::Struct,
        file_info: &mut FileTypeInfo,
        _is_external: bool,
    ) -> Result<()> {
        file_info.items.push(ItemInfo::Struct {
            name: ctx.display_name(struct_def.name(ctx.db)),
            id: format_id(struct_def),
            fields: extract_struct_fields(ctx, struct_def),
        });

        Ok(())
    }
}

pub struct EnumExtractor;
impl ItemExtractor<ra_ap_hir::Enum> for EnumExtractor {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        enum_def: &ra_ap_hir::Enum,
        file_info: &mut FileTypeInfo,
        _is_external: bool,
    ) -> Result<()> {
        file_info.items.push(ItemInfo::Enum {
            name: ctx.display_name(enum_def.name(ctx.db)),
            id: format_id(enum_def),
            variants: extract_enum_variants(ctx, enum_def),
        });

        Ok(())
    }
}

pub struct TraitExtractor;
impl ItemExtractor<ra_ap_hir::Trait> for TraitExtractor {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        trait_def: &ra_ap_hir::Trait,
        file_info: &mut FileTypeInfo,
        _is_external: bool,
    ) -> Result<()> {
        file_info.items.push(ItemInfo::Trait {
            name: ctx.display_name(trait_def.name(ctx.db)),
            id: format_id(trait_def),
            items: extract_trait_items(ctx, trait_def),
        });

        Ok(())
    }
}

pub struct TypeAliasExtractor;
impl ItemExtractor<ra_ap_hir::TypeAlias> for TypeAliasExtractor {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        type_alias: &ra_ap_hir::TypeAlias,
        file_info: &mut FileTypeInfo,
        _is_external: bool,
    ) -> Result<()> {
        let target = ctx.display(type_alias.ty(ctx.db));
        extract_type_names(&target, ctx.referenced_types);

        file_info.items.push(ItemInfo::TypeAlias {
            name: ctx.display_name(type_alias.name(ctx.db)),
            id: format_id(type_alias),
            target,
        });

        Ok(())
    }
}

pub struct ConstExtractor;
impl ItemExtractor<ra_ap_hir::Const> for ConstExtractor {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        const_def: &ra_ap_hir::Const,
        file_info: &mut FileTypeInfo,
        _is_external: bool,
    ) -> Result<()> {
        let ty = ctx.display(const_def.ty(ctx.db));
        extract_type_names(&ty, ctx.referenced_types);

        file_info.items.push(ItemInfo::Const {
            name: const_def
                .name(ctx.db)
                .map(|n| ctx.display_name(n))
                .unwrap_or_else(|| "_".to_string()),
            id: format_id(const_def),
            ty,
        });

        Ok(())
    }
}

pub struct StaticExtractor;
impl ItemExtractor<ra_ap_hir::Static> for StaticExtractor {
    fn extract(
        &self,
        ctx: &mut AnalysisContext,
        static_def: &ra_ap_hir::Static,
        file_info: &mut FileTypeInfo,
        _is_external: bool,
    ) -> Result<()> {
        let ty = ctx.display(static_def.ty(ctx.db));
        extract_type_names(&ty, ctx.referenced_types);

        file_info.items.push(ItemInfo::Static {
            name: ctx.display_name(static_def.name(ctx.db)),
            id: format_id(static_def),
            ty,
        });

        Ok(())
    }
}

fn extract_function_params(
    ctx: &mut AnalysisContext,
    func: &ra_ap_hir::Function,
) -> Vec<ParamInfo> {
    func.params_without_self(ctx.db)
        .into_iter()
        .enumerate()
        .map(|(idx, param)| {
            let ty = ctx.display(param.ty());
            extract_type_names(&ty, ctx.referenced_types);
            ParamInfo {
                name: param
                    .name(ctx.db)
                    .map(|name| ctx.display_name(name))
                    .unwrap_or_else(|| format!("_{}", idx)),
                ty,
                type_ref: None,
            }
        })
        .collect()
}

fn extract_struct_fields(
    ctx: &mut AnalysisContext,
    struct_def: &ra_ap_hir::Struct,
) -> Vec<FieldInfo> {
    struct_def
        .fields(ctx.db)
        .into_iter()
        .map(|field| {
            let ty = ctx.display(field.ty(ctx.db));
            extract_type_names(&ty, ctx.referenced_types);
            FieldInfo {
                name: ctx.display_name(field.name(ctx.db)),
                ty,
                type_ref: None,
            }
        })
        .collect()
}

fn extract_enum_variants(
    ctx: &mut AnalysisContext,
    enum_def: &ra_ap_hir::Enum,
) -> Vec<VariantInfo> {
    enum_def
        .variants(ctx.db)
        .into_iter()
        .map(|variant| VariantInfo {
            name: ctx.display_name(variant.name(ctx.db)),
            fields: variant
                .fields(ctx.db)
                .into_iter()
                .map(|field| {
                    let ty = ctx.display(field.ty(ctx.db));
                    extract_type_names(&ty, ctx.referenced_types);
                    FieldInfo {
                        name: ctx.display_name(field.name(ctx.db)),
                        ty,
                        type_ref: None,
                    }
                })
                .collect(),
        })
        .collect()
}

fn extract_trait_items(
    ctx: &mut AnalysisContext,
    trait_def: &ra_ap_hir::Trait,
) -> Vec<TraitItemInfo> {
    trait_def
        .items(ctx.db)
        .into_iter()
        .map(|item| match item {
            ra_ap_hir::AssocItem::Function(func) => {
                let return_type = ctx.display(func.ret_type(ctx.db));
                extract_type_names(&return_type, ctx.referenced_types);

                TraitItemInfo::Function {
                    name: ctx.display_name(func.name(ctx.db)),
                    params: extract_function_params(ctx, &func),
                    return_type,
                }
            },
            ra_ap_hir::AssocItem::TypeAlias(ty) => TraitItemInfo::TypeAlias {
                name: ctx.display_name(ty.name(ctx.db)),
            },
            ra_ap_hir::AssocItem::Const(c) => {
                let ty = ctx.display(c.ty(ctx.db));
                extract_type_names(&ty, ctx.referenced_types);
                TraitItemInfo::Const {
                    name: c
                        .name(ctx.db)
                        .map(|n| ctx.display_name(n))
                        .unwrap_or_else(|| "_".to_string()),
                    ty,
                }
            },
        })
        .collect()
}

fn extract_function_body(
    ctx: &mut AnalysisContext,
    func: &ra_ap_hir::Function,
) -> Option<FunctionBodyInfo> {
    let body_expr = func.source(ctx.db)?.value.body()?;
    Some(FunctionBodyInfo {
        locals: extract_locals(body_expr.syntax()),
        closures: extract_closures(body_expr.syntax()),
    })
}

fn extract_locals(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<LocalVarInfo> {
    let mut locals = Vec::new();

    for node in syntax.descendants() {
        if let Some(let_stmt) = ast::LetStmt::cast(node) {
            if let Some(local) = extract_local_var(&let_stmt, locals.len()) {
                locals.push(local);
            }
        }
    }

    locals
}

fn extract_local_var(let_stmt: &ast::LetStmt, id: usize) -> Option<LocalVarInfo> {
    let (mutable, name) = match &let_stmt.pat()? {
        ast::Pat::IdentPat(ident_pat) => (
            ident_pat.mut_token().is_some(),
            ident_pat.name().map(|n| n.to_string()),
        ),
        _ => (false, None),
    };

    Some(LocalVarInfo {
        name,
        ty: if let Some(ty_node) = let_stmt.ty() {
            ty_node.syntax().text().to_string()
        } else {
            INFERRED_TYPE.to_string()
        },
        id,
        mutable,
    })
}

fn extract_closures(syntax: &ra_ap_syntax::SyntaxNode) -> Vec<ClosureInfo> {
    let mut closures = Vec::new();

    for node in syntax.descendants() {
        if let Some(closure) = ast::ClosureExpr::cast(node) {
            closures.push(extract_closure_info(&closure, closures.len()));
        }
    }

    closures
}

fn extract_closure_info(closure: &ast::ClosureExpr, id: usize) -> ClosureInfo {
    ClosureInfo {
        id,
        params: closure
            .param_list()
            .map(|params| {
                params
                    .params()
                    .enumerate()
                    .map(|(idx, param)| ParamInfo {
                        name: param
                            .pat()
                            .and_then(|p| {
                                if let ast::Pat::IdentPat(ident) = p {
                                    ident.name().map(|n| n.to_string())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| format!("_{}", idx)),
                        ty: param
                            .ty()
                            .map(|ty_node| ty_node.syntax().text().to_string())
                            .unwrap_or_else(|| INFERRED_TYPE.to_string()),
                        type_ref: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        return_type: closure
            .ret_type()
            .map(|ret| ret.syntax().text().to_string())
            .unwrap_or_else(|| INFERRED_TYPE.to_string()),
    }
}
