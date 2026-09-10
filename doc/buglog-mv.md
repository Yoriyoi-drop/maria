# Buglog maria-mv — hasil eksplorasi (pencari bug maria utama)

Pipeline `.mv` → transpile → SV → sim dipakai sebagai oracle untuk menemukan
bug di maria utama (parser/elaborator/simulator). Status: ✅ fixed / ⏳ open.

## ✅ Fixed

1. **Array decl-init assignment pattern reversed** — `rom [0:3] = '{a,b,c,d}`
   menghasilkan `rom[0]=d` (Concat MSB-first vs storage unpacked elemen-0 di
   bit terendah; decl-init di-fold jadi Const lalu write utuh). Fix: decompose
   Concat → per-elemen assign di decl-init (pola sama dgn statement assign).

2. **VCD array elemen lebar salah** — `rom[i]` di-emit 32-bit `[31:0]` padahal
   elemen 8-bit (VCD header pakai lebar total, bukan width/array_depth).

3. **always_comb array-index sensitivity** — `always_comb val = rom[idx]` tidak
   re-trigger saat `idx` berubah (IR `ArrayIndex` sensitivity hanya collect
   signal dasar, index hilang). Fix: collect index juga.

5. **Signed relational vs literal** — `signed [7:0] s; s=-1; s < 0` → false
   (unsigned 255<0). Fix: SQL relational signed bila SALAH SATU operand signed
   (&& → || di eval/expr.rs); Div/Mod ikut.

6. **Real literal arithmetic NaN** — `1.5 + 2.25` → NaN (evaluator biner hanya
   kenal is_real utk operand signal; literal murni jalur integer). Fix:
   fold real literal+literal di elaborasi (`fold_binary_real`); vs signal
   (variable + literal) sudah jalan via jalur is_real.

7. **Signed Div/Mod vs literal** — `s = -4; sd = s / 2` → 126 (unsigned 252/2).
   Operand sinyal di-cast zero-extend ke 32 oleh konteks (`Cast{32, Signal}`)
   sebelum eval signed → l=252. Fix: clip operand Div/Mod ke lebar ASLI signal
   (signed_raw_operand menembus Cast/Signed wrapper) → -4/2 = -2 (0xFE).

## ⏳ Open

4. **Unpacked array MULTI-dimensi hanya 1 dim disimpan** — parser SV
   (`maria-parser/src/decl.rs` skip blind) hanya menyimpan SATU dimensi
   unpacked; `logic [7:0] mat [0:1] [0:1]` menjadi width 16 / 2 elemen
   (harus 4), init `'{'{1,2},'{3,4}}` tak ter-decompose → semua elemen 0.
   Butuh representasi multi unpacked dims di AST `Decl` + elaborasi
   (array_dims, width product) + engine index — skope besar.
   Reproduksi SV murni: `logic [7:0] mat [0:1][0:1] = '{{1,2},{3,4}};` →
   `mat[0][0]` = 0 (harus 1). `.mv`: `sig mat : logic[8][2][2] = ...`.
   Catatan: engine juga belum dukung index 2-d (`mat[i][j]` → E1002).