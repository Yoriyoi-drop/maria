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

/// Gate drive strength keywords
pub fn is_strength_keyword(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(),
        "supply0" | "supply1" | "strong0" | "strong1" | "pull0" | "pull1"
        | "weak0" | "weak1" | "highz0" | "highz1"
    )
}
