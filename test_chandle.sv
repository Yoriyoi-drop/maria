// Test module untuk chandle type declaration
module test_chandle;

    // Deklarasi chandle
    chandle ctx;

    // Import DPI-C function
    import "DPI-C" function chandle create_ctx();
    import "DPI-C" function void tick_ctx(input chandle ctx);
    import "DPI-C" function void close_ctx(input chandle ctx);

    initial begin
        // Inisialisasi ctx dari DPI function
        ctx = create_ctx();
        // Gunakan ctx di DPI call
        tick_ctx(ctx)
        #1;
        close_ctx(ctx);
        ctx = null;
        #1 $finish;
    end

endmodule
