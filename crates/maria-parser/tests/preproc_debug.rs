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

// Regression: unknown macro di awal baris (`DV_SPINWAIT_EXIT(...)) yang body
// multi-baris-nya memuat komentar `//` SEBELUM baris berisi `` `MACRO `` lainnya.
// Sebelumnya loop scan blind-copy sisa string setelah komentar → backtick
// macro di baris lanjutan bocor ke output (lexical error E1002).
#[test]
fn comment_before_backtick_macro_in_multiline_unknown_macro() {
    let src = r##"`define DV_CHECK_EQ(ACT_, EXP_, MSG_) \
  begin \
    if ((ACT_ ) == (EXP_)) ; \
  end

module top;
  initial begin
    `DV_SPINWAIT_EXIT(
        forever begin
          // 1 extra cycle to make sure no race condition
          if (clk) break;
          `DV_CHECK_EQ(a, 1,
                       $sformatf("fatal %0s", b))
        end,
        wait(c));;
  end
endmodule
"##;
    let mut pp = Preprocessor::new();
    let out = pp.preprocess(src, None).unwrap();
    // Tidak boleh ada backtick ` tersisa di output (akan jadi E1002).
    assert!(
        !out.contains('`'),
        "output mengandung backtick bocor (E1002), out:\n{}",
        out
    );
    // Macro yang dikenal tetap di-expand (DV_CHECK_EQ jadi begin/end polos).
    assert!(
        out.contains("if ((a ) == (1))"),
        "DV_CHECK_EQ tidak di-expand, out:\n{}",
        out
    );
}

/// Parse source & assert tidak ada error (macro NON-EXPAND di-buang).
fn assert_no_parse_error(name: &str, src: &str) {
    let mut pp = Preprocessor::new();
    let out = pp.preprocess(src, None).unwrap();
    let out = format!("`line 1 \"/tmp/{}.sv\"\n{}\n", name, out);
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
    eprintln!(
        "=====PARSE '{}' result: {:?}=====",
        name,
        design.as_ref().err()
    );
    assert!(design.is_ok(), "parse '{}' harus sukses: {:?}", name, design);
}

// Macro NON-EXPAND sbg module item (`Ident(`) sbg buang — bukan instance.
#[test]
fn unexpanded_macro_at_module_member_is_skipped() {
    assert_no_parse_error(
        "t_mod_macro",
        r##"module top;
  DV_FCOV_INSTANTIATE_CG(adc_ctrl_hw_reset_cg)
  ASSERT(ScrambledImpliesBuffered_A, Info.secret |-> Info.variant == Buffered)
  logic [7:0] x;
  initial begin
    x = 1;
  end
endmodule
"##,
    );
}

// Macro NON-EXPAND sbg statement di blok initial/always: body argumen berisi
// `;`/`{...}` (constraint dv_macros) atau operator SVA (prim_assert).
#[test]
fn unexpanded_macro_statement_is_skipped() {
    assert_no_parse_error(
        "t_stmt_macro",
        r##"module top;
  logic [7:0] a;
  initial begin
    a = 1;
    DV_CHECK_STD_RANDOMIZE_WITH_FATAL(a, a inside {[1:7]};, , MSG)
    DV_SPINWAIT_EXIT(#10ns;, a == 1;)
    ASSERT(IbexImmBMuxSelValid, a |-> a == 1)
    a = 2;
  end
endmodule
"##,
    );
}

