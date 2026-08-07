use crate::parser::preprocessor::Preprocessor;

/// Bangun preprocessor dengan incdirs + defines (dari workspace/config).
pub fn build_preprocessor(incdirs: &[String], defines: &[(String, String)]) -> Preprocessor {
    let mut pp = Preprocessor::new();
    for dir in incdirs {
        pp.add_search_path(dir);
    }
    for (name, value) in defines {
        pp.define(name, value);
    }
    pp
}

/// Preprocess file → source terproses.
pub fn preprocess_file(
    path: &str,
    incdirs: &[String],
    defines: &[(String, String)],
) -> Result<String, crate::error::SimError> {
    let mut pp = build_preprocessor(incdirs, defines);
    pp.preprocess_file(path)
}

/// Preprocess string sumber → source terproses.
pub fn preprocess_str(
    source: &str,
    incdirs: &[String],
    defines: &[(String, String)],
) -> Result<String, crate::error::SimError> {
    let mut pp = build_preprocessor(incdirs, defines);
    pp.preprocess(source, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocess_str_basic() {
        let out = preprocess_str("`define W 8\nmodule m;\nendmodule\n", &[], &[]).unwrap();
        assert!(out.contains("module m"));
    }

    #[test]
    fn test_build_preprocessor_defines() {
        let mut pp = build_preprocessor(&[], &[("W".into(), "8".into())]);
        let out = pp.preprocess("wire [`W-1:0] x;", None).unwrap();
        assert!(out.contains("8"));
    }
}
