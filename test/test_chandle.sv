// Test: chandle type declaration
// Seharusnya: ctx dikenali sebagai signal di modul test_chandle
module test_chandle;

    chandle ctx;           // deklarasi chandle
    chandle ptr1, ptr2;    // multiple chandle declarations

    import "DPI-C" function chandle create_ctx();
    import "DPI-C" function void tick_ctx(input chandle ctx_i, input chandle p1, input chandle p2);
    import "DPI-C" function void close_ctx(input chandle ctx_i);

    initial begin
        ctx = create_ctx();
        ptr1 = ctx;
        ptr2 = ctx;
        tick_ctx(ctx, ptr1, ptr2);
        #1;
        close_ctx(ctx);
        ctx = null;
        #1 $finish;
    end

endmodule
