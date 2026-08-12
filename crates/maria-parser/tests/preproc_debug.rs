use maria_parser::lexer::Lexer;
use maria_parser::preprocessor::Preprocessor;
use maria_parser::Parser;

#[test]
fn debug_nested_stringify() {
    let src = r##"`define ASSERT_ERROR(__name) \
  $error("FAIL: %s", `"__name`")

`define ASSERT_I(__name, __prop) \
  __name: assert (__prop) \
    else begin \
      `ASSERT_ERROR(__name) \
    end

module nested_macro3;
  logic [1:0] dv_hook;
  initial begin
    `ASSERT_I(accelerate_regulators_power_up_time, dv_hook inside {[0:3]})
  end
endmodule
"##;
    let mut pp = Preprocessor::new();
    let out = pp.preprocess(src, None).unwrap();
    eprintln!("=====PREPROC OUT=====\n{}\n=====END=====", out);
    // CLI menambahkan directive `line di depan setiap file (main.rs)
    let out = format!("`line 1 \"/tmp/nested_macro3.sv\"\n{}\n", out);
    eprintln!("=====WITH LINEDIR=====\n{}\n=====END=====", out);
    // Now parse the preprocessed output
    let mut lex = Lexer::new(&out);
    let mut tokens = Vec::new();
    loop {
        let (tok, l, c) = lex.next_token();
        if matches!(tok, maria_parser::lexer::Token::Eof) {
            break;
        }
        tokens.push((tok, l, c));
    }
    let mut parser = Parser::new(tokens, &out);
    let design = parser.parse_design();
    eprintln!("=====PARSE result: {:?}=====", design.as_ref().err());
    assert!(out.contains("$error"), "output should contain $error");
    assert!(design.is_ok(), "parsing preprocessed output should succeed");
}
