use std::path::Path;

/// Deteksi file template yang TIDAK layak di-parse sebagai SystemVerilog.
///
/// OpenTitan / riscv-dv memakai konvensi `*.tpl.sv`: file Jinja2 dengan
/// direktif `% if`/`% for`/`${...}` yang harus di-render dulu (bukan SV
/// valid). Filelist yang mencantumkannya (mis. `opentitan_rtl.f`) harus
/// men-skip file ini agar compile tidak gagal di lexer (mis. `'SecureIbex'`
/// terbaca sebagai literal signed `'s` yang invalid).
pub fn is_template_source(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.ends_with(".tpl.sv")
        || name.ends_with(".tpl.svh")
        || name.ends_with(".tpl.v")
        || name.ends_with(".tpl.vh")
        || name.ends_with(".tpl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_template_suffixes() {
        for p in [
            "riscv_core_setting.tpl.sv",
            "foo.tpl.svh",
            "a.tpl.v",
            "b.tpl.vh",
            "c.tpl",
            "x.TPL.SV",
        ] {
            assert!(is_template_source(Path::new(p)), "harus terdeteksi: {}", p);
        }
    }

    #[test]
    fn accepts_plain_sources() {
        for p in ["counter.sv", "top.svh", "rtl/ibex_core.sv", "plain.tpl_notes"] {
            assert!(
                !is_template_source(Path::new(p)),
                "tidak boleh terdeteksi: {}",
                p
            );
        }
    }
}
