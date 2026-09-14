`ifndef NVAL
`define NVAL 50
`endif

// Q2 AUDIT — candidate: import prepass O(M * P^2).
// `import pkg_x::*` di setiap module: untuk tiap nama dari package, elaborator
// mengecek `module.items.iter().any(...)` (dengan items yang tumbuh setiap
// function di-push). Per module: P(P+1)/2 item-scan. M module -> M*P^2/2.
// File ini: package dgn NVAL function + NVAL module yang semuanya import ::*.

package pkg_x;
`define GEN_FN(n) function automatic logic [7:0] f``n``(input logic [7:0] a); return a; endfunction
`GEN_FN(0)
`GEN_FN(1)
`GEN_FN(2)
`GEN_FN(3)
`GEN_FN(4)
`GEN_FN(5)
`GEN_FN(6)
`GEN_FN(7)
`GEN_FN(8)
`GEN_FN(9)
`GEN_FN(10)
`GEN_FN(11)
`GEN_FN(12)
`GEN_FN(13)
`GEN_FN(14)
`GEN_FN(15)
`GEN_FN(16)
`GEN_FN(17)
`GEN_FN(18)
`GEN_FN(19)
`GEN_FN(20)
`GEN_FN(21)
`GEN_FN(22)
`GEN_FN(23)
`GEN_FN(24)
`GEN_FN(25)
`GEN_FN(26)
`GEN_FN(27)
`GEN_FN(28)
`GEN_FN(29)
`GEN_FN(30)
`GEN_FN(31)
`GEN_FN(32)
`GEN_FN(33)
`GEN_FN(34)
`GEN_FN(35)
`GEN_FN(36)
`GEN_FN(37)
`GEN_FN(38)
`GEN_FN(39)
`GEN_FN(40)
`GEN_FN(41)
`GEN_FN(42)
`GEN_FN(43)
`GEN_FN(44)
`GEN_FN(45)
`GEN_FN(46)
`GEN_FN(47)
`GEN_FN(48)
`GEN_FN(49)
endpackage

module m0; import pkg_x::*; logic [7:0] q0; initial q0 = f0(q0); endmodule
module m1; import pkg_x::*; logic [7:0] q1; initial q1 = f1(q1); endmodule
module m2; import pkg_x::*; logic [7:0] q2; initial q2 = f2(q2); endmodule
module m3; import pkg_x::*; logic [7:0] q3; initial q3 = f3(q3); endmodule
module m4; import pkg_x::*; logic [7:0] q4; initial q4 = f4(q4); endmodule
module m5; import pkg_x::*; logic [7:0] q5; initial q5 = f5(q5); endmodule
module m6; import pkg_x::*; logic [7:0] q6; initial q6 = f6(q6); endmodule
module m7; import pkg_x::*; logic [7:0] q7; initial q7 = f7(q7); endmodule
module m8; import pkg_x::*; logic [7:0] q8; initial q8 = f8(q8); endmodule
module m9; import pkg_x::*; logic [7:0] q9; initial q9 = f9(q9); endmodule
module m10; import pkg_x::*; logic [7:0] q10; initial q10 = f10(q10); endmodule
module m11; import pkg_x::*; logic [7:0] q11; initial q11 = f11(q11); endmodule
module m12; import pkg_x::*; logic [7:0] q12; initial q12 = f12(q12); endmodule
module m13; import pkg_x::*; logic [7:0] q13; initial q13 = f13(q13); endmodule
module m14; import pkg_x::*; logic [7:0] q14; initial q14 = f14(q14); endmodule
module m15; import pkg_x::*; logic [7:0] q15; initial q15 = f15(q15); endmodule
module m16; import pkg_x::*; logic [7:0] q16; initial q16 = f16(q16); endmodule
module m17; import pkg_x::*; logic [7:0] q17; initial q17 = f17(q17); endmodule
module m18; import pkg_x::*; logic [7:0] q18; initial q18 = f18(q18); endmodule
module m19; import pkg_x::*; logic [7:0] q19; initial q19 = f19(q19); endmodule
module m20; import pkg_x::*; logic [7:0] q20; initial q20 = f20(q20); endmodule
module m21; import pkg_x::*; logic [7:0] q21; initial q21 = f21(q21); endmodule
module m22; import pkg_x::*; logic [7:0] q22; initial q22 = f22(q22); endmodule
module m23; import pkg_x::*; logic [7:0] q23; initial q23 = f23(q23); endmodule
module m24; import pkg_x::*; logic [7:0] q24; initial q24 = f24(q24); endmodule
module m25; import pkg_x::*; logic [7:0] q25; initial q25 = f25(q25); endmodule
module m26; import pkg_x::*; logic [7:0] q26; initial q26 = f26(q26); endmodule
module m27; import pkg_x::*; logic [7:0] q27; initial q27 = f27(q27); endmodule
module m28; import pkg_x::*; logic [7:0] q28; initial q28 = f28(q28); endmodule
module m29; import pkg_x::*; logic [7:0] q29; initial q29 = f29(q29); endmodule
module m30; import pkg_x::*; logic [7:0] q30; initial q30 = f30(q30); endmodule
module m31; import pkg_x::*; logic [7:0] q31; initial q31 = f31(q31); endmodule
module m32; import pkg_x::*; logic [7:0] q32; initial q32 = f32(q32); endmodule
module m33; import pkg_x::*; logic [7:0] q33; initial q33 = f33(q33); endmodule
module m34; import pkg_x::*; logic [7:0] q34; initial q34 = f34(q34); endmodule
module m35; import pkg_x::*; logic [7:0] q35; initial q35 = f35(q35); endmodule
module m36; import pkg_x::*; logic [7:0] q36; initial q36 = f36(q36); endmodule
module m37; import pkg_x::*; logic [7:0] q37; initial q37 = f37(q37); endmodule
module m38; import pkg_x::*; logic [7:0] q38; initial q38 = f38(q38); endmodule
module m39; import pkg_x::*; logic [7:0] q39; initial q39 = f39(q39); endmodule
module m40; import pkg_x::*; logic [7:0] q40; initial q40 = f40(q40); endmodule
module m41; import pkg_x::*; logic [7:0] q41; initial q41 = f41(q41); endmodule
module m42; import pkg_x::*; logic [7:0] q42; initial q42 = f42(q42); endmodule
module m43; import pkg_x::*; logic [7:0] q43; initial q43 = f43(q43); endmodule
module m44; import pkg_x::*; logic [7:0] q44; initial q44 = f44(q44); endmodule
module m45; import pkg_x::*; logic [7:0] q45; initial q45 = f45(q45); endmodule
module m46; import pkg_x::*; logic [7:0] q46; initial q46 = f46(q46); endmodule
module m47; import pkg_x::*; logic [7:0] q47; initial q47 = f47(q47); endmodule
module m48; import pkg_x::*; logic [7:0] q48; initial q48 = f48(q48); endmodule
module m49; import pkg_x::*; logic [7:0] q49; initial q49 = f49(q49); endmodule

module top_plat; endmodule