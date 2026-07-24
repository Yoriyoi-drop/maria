#[cfg(test)]
mod debug_lexer_test {
    use crate::frontend::FastLexer;
    use crate::parser::lexer::{Lexer, Token};

    #[test]
    fn debug_lexer_diff() {
        let input = "module counter(input clk, input rst, output reg [3:0] count);
    always @(posedge clk) begin
        if (rst) count <= 0;
        else count <= count + 1;
    end
endmodule";

        // Legacy
        let mut legacy = Lexer::new(input);
        let mut lt = Vec::new();
        loop {
            let (tok, _, _) = legacy.next_token();
            if tok == Token::Eof { break; }
            lt.push(tok);
        }

        // Fast
        let mut fast = FastLexer::new(input, "");
        let mut ft = Vec::new();
        loop {
            let (tok, _, _) = fast.next_token();
            if tok == Token::Eof { break; }
            ft.push(tok);
        }

        assert_eq!(lt.len(), ft.len(), "legacy={}, fast={}", lt.len(), ft.len());

        for (i, (l, f)) in lt.iter().zip(ft.iter()).enumerate() {
            assert_eq!(
                std::mem::discriminant(l),
                std::mem::discriminant(f),
                "Pos {}: legacy={:?} vs fast={:?}", i, l, f
            );
        }
    }
}
