use maria_parser::preprocessor::Preprocessor;

fn dump(name: &str, src: &str) {
    let mut pp = Preprocessor::new();
    match pp.preprocess(src, None) {
        Ok(expanded) => {
            println!("=== {} ===\n{}\n======", name, expanded);
        }
        Err(e) => println!("=== {} ERROR: {} ===\n", name, e),
    }
}

#[test]
fn dump_variants() {
    // V1: dengan paste new``ARGS_ dan default ARGS_=()
    dump(
        "V1 paste+default",
        r#"
interface foo;
  bit enable_bar = 1'b0;
  covergroup bar; endgroup
  `define TEST(NAME_, COND_ = 1'b1, ARGS_ = ()) bit en_``NAME_ = 1'b0; NAME_ NAME_``_inst; initial begin #1; if ((en_``NAME_)||(COND_)) begin NAME_``_inst = new``ARGS_; end end
  `TEST(bar, enable_bar)
  wire x;
endinterface
"#,
    );
    // V2: tanpa paste new``ARGS_ (pakai new() literal)
    dump(
        "V2 no-paste-new",
        r#"
interface foo;
  bit enable_bar = 1'b0;
  covergroup bar; endgroup
  `define TEST(NAME_, COND_ = 1'b1, ARGS_ = ()) NAME_ NAME_``_inst; initial begin #1; if (COND_) begin NAME_``_inst = new(); end end
  `TEST(bar, enable_bar)
  wire x;
endinterface
"#,
    );
    // V3: paste en_``NAME_ saja
    dump(
        "V3 paste-name-only",
        r#"
module top;
  `define TEST(NAME_) bit en_``NAME_ = 1'b0;
  `TEST(bar)
endmodule
"#,
    );
    // V4: new``ARGS_ dengan ARGS_ eksplisit () (dipanggil 3 arg)
    dump(
        "V4 explicit-empty-args",
        r#"
interface foo;
  covergroup bar; endgroup
  `define TEST(NAME_, COND_ = 1'b1, ARGS_ = ()) NAME_ NAME_``_inst; initial begin NAME_``_inst = new``ARGS_; end
  `TEST(bar, 1'b1, ())
  wire x;
endinterface
"#,
    );
}
