# maria — RTL Simulator untuk SystemVerilog

A Rust-based RTL simulator for SystemVerilog. Compiles `.sv` files through a pipeline of preprocessor → lexer → parser → AST → elaborator → IR → simulation engine → VCD output.

## Quick start

```shell
cargo run -- test/counter.sv
cargo run -- test/tb_counter.sv -T 200
cargo run -- file.sv --ast      # print AST
cargo run -- file.sv --tokens   # print tokens
```

## Project file

A `.maria` file lists `.sv` sources (one per line, `#` for comments):

```
counter.sv
tb_counter.sv
```

```shell
cargo run -- --start .maria
```

## Build & test

```shell
cargo build
cargo test
```

## Pipeline

```
.sv → preprocessor → lexer → parser → AST → elaborator → IR → engine → VCD
```

## License

MIT
