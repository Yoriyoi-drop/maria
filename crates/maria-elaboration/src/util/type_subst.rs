//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Type parameter substitution.
//!
//! Fungsi:
//!   - substitute_class_types()   — substitusi tipe di class declaration
//!   - substitute_data_type()     — substitusi tipe di DataType
//!   - substitute_expr_types()    — substitusi tipe di expression
//!
//! ──────────────────────────────────────────────────────────────────────────────

use maria_ast::*;

/// Substitusi type parameter di dalam ClassDecl.
/// Mengganti semua kemunculan param_name dengan replacement di member class.
pub fn substitute_class_types(
    cd: ClassDecl,
    param_name: &str,
    replacement: &DataType,
) -> ClassDecl {
    let mut new_members = Vec::new();
    for member in cd.members {
        match member {
            ClassMember::Decl(mut decl) => {
                decl.dtype = substitute_data_type(decl.dtype, param_name, replacement);
                new_members.push(ClassMember::Decl(decl));
            }
            ClassMember::Function(mut fd) => {
                fd.return_type = fd
                    .return_type
                    .map(|dt| Box::new(substitute_data_type(*dt, param_name, replacement)));
                new_members.push(ClassMember::Function(fd));
            }
            ClassMember::Task(td) => {
                new_members.push(ClassMember::Task(td));
            }
            ClassMember::Constraint { name, body, is_static } => {
                let new_body = body
                    .into_iter()
                    .map(|ci| substitute_constraint_item(ci, param_name, replacement))
                    .collect();
                new_members.push(ClassMember::Constraint {
                    name,
                    body: new_body,
                    is_static,
                });
            }
            ClassMember::Let(mut ld) => {
                // LANG-40: substitusi type param di body let (class generic).
                ld.expr = substitute_expr_types(ld.expr, param_name, replacement);
                new_members.push(ClassMember::Let(ld));
            }
        }
    }
    ClassDecl {
        members: new_members,
        ..cd
    }
}

/// Substitusi type parameter di dalam DataType AST.
/// Mengganti UserDefined(name) yang cocok dengan param_name.
pub fn substitute_data_type(dt: DataType, param_name: &str, replacement: &DataType) -> DataType {
    match dt {
        DataType::UserDefined(ref name) if name == param_name => replacement.clone(),
        DataType::Signed(inner) => DataType::Signed(Box::new(substitute_data_type(
            *inner,
            param_name,
            replacement,
        ))),
        DataType::EnumType { base, members } => DataType::EnumType {
            base: base.map(|b| Box::new(substitute_data_type(*b, param_name, replacement))),
            members,
        },
        DataType::StructType { members } => DataType::StructType {
            members: members
                .into_iter()
                .map(|m| StructMember {
                    dtype: Box::new(substitute_data_type(*m.dtype, param_name, replacement)),
                    ..m
                })
                .collect(),
        },
        DataType::UnionType { members } => DataType::UnionType {
            members: members
                .into_iter()
                .map(|m| StructMember {
                    dtype: Box::new(substitute_data_type(*m.dtype, param_name, replacement)),
                    ..m
                })
                .collect(),
        },
        other => other,
    }
}

/// Substitusi type parameter di dalam Expr AST.
/// Berguna untuk constraint expression dengan type parameter.
pub fn substitute_expr_types(e: Expr, param_name: &str, replacement: &DataType) -> Expr {
    match e {
        Expr::BinaryOp { lhs, op, rhs } => Expr::BinaryOp {
            lhs: Box::new(substitute_expr_types(*lhs, param_name, replacement)),
            op,
            rhs: Box::new(substitute_expr_types(*rhs, param_name, replacement)),
        },
        Expr::UnaryOp { op, expr } => Expr::UnaryOp {
            op,
            expr: Box::new(substitute_expr_types(*expr, param_name, replacement)),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(substitute_expr_types(
            *inner,
            param_name,
            replacement,
        ))),
        Expr::Concat(items) => Expr::Concat(
            items
                .into_iter()
                .map(|e| substitute_expr_types(e, param_name, replacement))
                .collect(),
        ),
        Expr::Replicate { count, expr } => Expr::Replicate {
            count: Box::new(substitute_expr_types(*count, param_name, replacement)),
            expr: Box::new(substitute_expr_types(*expr, param_name, replacement)),
        },
        Expr::TernaryOp {
            cond,
            true_expr,
            false_expr,
        } => Expr::TernaryOp {
            cond: Box::new(substitute_expr_types(*cond, param_name, replacement)),
            true_expr: Box::new(substitute_expr_types(*true_expr, param_name, replacement)),
            false_expr: Box::new(substitute_expr_types(*false_expr, param_name, replacement)),
        },
        Expr::FuncCall { name, args, line, col } => Expr::FuncCall {
            name,
            args: args
                .into_iter()
                .map(|a| substitute_expr_types(a, param_name, replacement))
                .collect(),
            line,
            col,
        },
        other => other,
    }
}

/// Substitusi type parameter di dalam satu item constraint (F12). Rekursif
/// ke dalam cabang `if/else` — item di luar cabang (`Expr`, `SolveBefore`)
/// diteruskan apa adanya (tanpa ekspresi tipe yang relevan).
fn substitute_constraint_item(
    ci: ConstraintItem,
    param_name: &str,
    replacement: &DataType,
) -> ConstraintItem {
    match ci {
        ConstraintItem::Expr(e) => ConstraintItem::Expr(substitute_expr_types(e, param_name, replacement)),
        ConstraintItem::SolveBefore { vars } => ConstraintItem::SolveBefore { vars },
        // LANG-31: `soft expr` — substitusi ekspresi di dalam soft constraint.
        ConstraintItem::Soft(e) => ConstraintItem::Soft(substitute_expr_types(e, param_name, replacement)),
        ConstraintItem::If { cond, then, els } => ConstraintItem::If {
            cond: substitute_expr_types(cond, param_name, replacement),
            then: then
                .into_iter()
                .map(|ci| substitute_constraint_item(ci, param_name, replacement))
                .collect(),
            els: els
                .into_iter()
                .map(|ci| substitute_constraint_item(ci, param_name, replacement))
                .collect(),
        },
    }
}
