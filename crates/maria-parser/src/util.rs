use maria_ast::*;

/// Konversi digit desimal (string) ke pola biner presisi penuh — dipakai
/// parser utk literal desimal yang melebihi i64/u64 sehingga tidak bisa
/// disimpan di `Value::Decimal` (dulu diam-diam jadi 0 atau Expr::Ident).
/// Underscore pemisah diabaikan; hasil tanpa leading zero ("0" bila nol).
pub fn dec_str_to_bits(s: &str) -> String {
    let mut digits: Vec<u8> = s
        .bytes()
        .filter(|b| b.is_ascii_digit())
        .map(|b| b - b'0')
        .collect();
    if digits.is_empty() {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while digits.iter().any(|&d| d != 0) {
        // Satu langkah pembagian digit-vektor oleh 2, sisanya = bit berikut.
        let mut rem = 0u16;
        for d in digits.iter_mut() {
            let cur = rem * 10 + *d as u16;
            *d = (cur / 2) as u8;
            rem = cur % 2;
        }
        out.push(if rem == 1 { '1' } else { '0' });
    }
    if out.is_empty() {
        "0".to_string()
    } else {
        out.into_iter().rev().collect()
    }
}

/// Check if an expression is valid as an lvalue (assignment target).
pub fn is_valid_lvalue(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Ident { .. }
            | Expr::RangeSelect { .. }
            | Expr::BitSelect { .. }
            | Expr::PartSelect { .. }
            | Expr::Concat(_)
            | Expr::MemberAccess { .. }
    )
}

/// Gate drive strength keywords
pub fn is_strength_keyword(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "supply0"
            | "supply1"
            | "strong0"
            | "strong1"
            | "pull0"
            | "pull1"
            | "weak0"
            | "weak1"
            | "highz0"
            | "highz1"
    )
}
