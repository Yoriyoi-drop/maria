use crate::ast::*;

/// Check if an expression is valid as an lvalue (assignment target).
pub fn is_valid_lvalue(expr: &Expr) -> bool {
    matches!(expr,
        Expr::Ident(_)
        | Expr::RangeSelect { .. }
        | Expr::BitSelect { .. }
        | Expr::PartSelect { .. }
        | Expr::Concat(_)
        | Expr::MemberAccess { .. }
    )
}
