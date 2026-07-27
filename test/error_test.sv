module error_test;
    // Error 1: undeclared type
    my_custom_type signal1;
    
    // Error 2: expected semicolon
    logic [7:0] data
    assign data = 8'hFF;
    
    // Error 3: null interface dereference (runtime)
    initial begin
        automatic int i = 0;
        // This should cause a NullHandle error if we try to use a null object
    end
endmodule
