//! ddmin minimalizer (Zeller 1999).
//!
//! Dua fase: line-level (chunk eksponensial 1,2,4...) lalu byte-level
//! (granularity separuh dari len/2 hingga 1). Predikat: bug ter-reproduksi.

/// Predikat: apakah source masih memicu bug yang sama?
pub type Predicate<'a> = &'a dyn Fn(&str) -> bool;

/// Minimalkan source sampai ukuran minimal sambil predikat tetap true.
pub fn ddmin(source: &str, predicate: Predicate) -> String {
    if !predicate(source) {
        return source.to_string();
    }
    let joined = phase_lines(source, predicate);
    phase_bytes(&joined, predicate)
}

fn phase_lines(source: &str, predicate: Predicate<'_>) -> String {
    let mut parts: Vec<String> = source.lines().map(str::to_owned).collect();
    if parts.len() < 2 {
        return source.to_string();
    }
    let mut granularity = 2usize;

    while parts.len() >= 2 {
        let chunk_size = parts.len().div_ceil(granularity);
        let mut reduced = false;
        let mut start = 0usize;
        while start < parts.len() {
            let end = (start + chunk_size).min(parts.len());
            let mut candidate_parts = parts.clone();
            candidate_parts.drain(start..end);
            let candidate = candidate_parts.join("\n") + "\n";
            if !candidate_parts.is_empty() && predicate(&candidate) {
                parts = candidate_parts;
                granularity = granularity.saturating_sub(1).max(2);
                reduced = true;
                break;
            }
            start = end;
        }
        if !reduced {
            if granularity >= parts.len() {
                break;
            }
            granularity = (granularity * 2).min(parts.len());
        }
    }

    parts.join("\n") + "\n"
}

fn phase_bytes(source: &str, predicate: Predicate<'_>) -> String {
    let mut chars: Vec<char> = source.chars().collect();
    let mut gran = chars.len() / 2;
    while gran >= 1 {
        let mut i = 0usize;
        while i < chars.len() {
            let end = (i + gran).min(chars.len());
            let mut candidate_chars = chars.clone();
            candidate_chars.drain(i..end);
            let candidate: String = candidate_chars.iter().collect();
            if predicate(&candidate) {
                chars = candidate_chars;
            } else {
                i += gran;
            }
        }
        gran /= 2;
    }
    let mut trimmed: String = chars.iter().collect::<String>().trim().to_string();
    trimmed.push('\n');
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddmin_reduces() {
        let src = "module top;\n  // c1\n  // c2\n  // c3\n  always @* begin\n    y = 1;\n  end\nendmodule\n";
        // predikat: mengandung "y = 1"
        let pred = |s: &str| s.contains("y = 1");
        let min = ddmin(src, &pred);
        assert!(min.contains("y = 1"));
        assert!(min.len() <= src.len());
    }

    #[test]
    fn ddmin_empty_pred_false() {
        let src = "module top;\nendmodule\n";
        let pred = |_: &str| false;
        let min = ddmin(src, &pred);
        // predikat false → return source original
        assert_eq!(min, src);
    }
}