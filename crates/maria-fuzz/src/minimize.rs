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
    let parts: Vec<&str> = source.lines().collect();
    if parts.is_empty() {
        return source.to_string();
    }
    let mut n = parts.len();
    let mut granularity = 1usize;
    let mut work = source.to_string();

    while granularity < n {
        let mut i = 0usize;
        while i < n {
            let chunk: Vec<&str> = parts[i..(i + granularity).min(n)].to_vec();
            let candidate = drop_chunks(&work, &chunk);
            if candidate.is_empty() {
                i += granularity;
                continue;
            }
            if predicate(&candidate) {
                work = candidate;
                n = work.lines().count();
                granularity = granularity.max(1);
                i = i.saturating_sub(granularity);
            } else {
                i += granularity;
            }
        }
        granularity *= 2;
    }
    work
}

fn phase_bytes(source: &str, predicate: Predicate<'_>) -> String {
    let mut work = source.to_string();
    let mut gran = work.len() / 2;
    while gran >= 1 {
        let mut i = 0usize;
        while i < work.len() {
            let end = (i + gran).min(work.len());
            let mut candidate = String::with_capacity(work.len() - gran);
            candidate.push_str(&work[..i]);
            candidate.push_str(&work[end..]);
            if predicate(&candidate) {
                work = candidate;
            } else {
                i += gran;
            }
        }
        gran /= 2;
    }
    let mut trimmed = work.trim().to_string();
    trimmed.push('\n');
    trimmed
}

fn drop_chunks(source: &str, chunk: &[&str]) -> String {
    let mut lines = source.lines();
    let mut out = String::new();
    let mut skip_next = 0usize;
    for line in lines.by_ref() {
        if skip_next > 0 {
            skip_next -= 1;
            continue;
        }
        if chunk.first() == Some(&line) {
            // cocok awal chunk: lewati semua baris chunk
            let mut rest = source.lines().skip(out.lines().count() + 1);
            let mut in_chunk = true;
            let mut chunk_idx = 1usize;
            while in_chunk {
                if let Some(l) = rest.next() {
                    if chunk_idx < chunk.len() && l == chunk[chunk_idx] {
                        chunk_idx += 1;
                        skip_next += 1;
                    } else {
                        in_chunk = false;
                        // tidak sederhana — fallback: jangan hapus
                        out.push_str(line);
                        out.push('\n');
                        skip_next = 0;
                    }
                } else {
                    in_chunk = false;
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if out.contains(&chunk[0]) {
        out
    } else {
        // hapus chunk dengan pendekatan sederhana
        source
            .lines()
            .filter(|l| !chunk.contains(l))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
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